#!/usr/bin/env python3
"""Catalog and de-duplicate every NON-class resource in the surviving builds.

Enumerates each surviving JAR's members that are not `.class` files (music,
sprites, models, localization tables, the extension-less data packs, icons,
manifest) and de-duplicates them by content sha256 (R2/R10: identity is the
hash, never the member name). Each unique blob records every (build, member)
occurrence, so a blob shared across builds — or carried under different names —
is one row with full provenance.

Emits `_reference/resources.json` and `_reference/resources.tsv`. Regeneration
is byte-identical (R3). `--self-test` proves the dedup keys on content: it flips
one byte of a shared blob in memory and confirms that copy splits off.
"""

from __future__ import annotations

import argparse
import io
import json
import sys
import zipfile
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

import corpus

REPO = corpus.REPO
DEFAULT_OUT = REPO / "_reference"

EXT_CATEGORY = {
    "mid": "midi",
    "midi": "midi",
    "amr": "audio",
    "wav": "audio",
    "png": "image",
    "mdl": "model",
    "lng": "lang",
}


def category(member: str) -> str:
    if member == "META-INF/MANIFEST.MF":
        return "manifest"
    base = member.rsplit("/", 1)[-1]
    if "." in base:
        return EXT_CATEGORY.get(base.rsplit(".", 1)[1].lower(), "other")
    return "pack"  # extension-less data pack (a c d f i ldf mi Name)


@dataclass
class Blob:
    sha256: str
    size: int
    category: str
    occurrences: list[tuple[str, str]] = field(default_factory=list)  # (build, member)


def analyze(*, corrupt: tuple[str, str] | None = None) -> dict[str, Blob]:
    """sha256 -> Blob over all non-class resources of every surviving build.

    `corrupt=(build_id, member)` flips one byte of that resource in memory before
    hashing, used by --self-test to prove the dedup keys on content.
    """
    blobs: dict[str, Blob] = {}
    for build in corpus.builds():
        with zipfile.ZipFile(io.BytesIO(build.payload)) as jar:
            for member in sorted(n for n in jar.namelist() if not n.endswith("/")):
                if member.endswith(".class"):
                    continue
                data = jar.read(member)
                if corrupt is not None and corrupt == (build.build_id, member) and data:
                    data = bytes([data[0] ^ 0xFF]) + data[1:]
                sha = corpus.sha256(data)
                blob = blobs.get(sha)
                if blob is None:
                    blob = Blob(sha, len(data), category(member))
                    blobs[sha] = blob
                blob.occurrences.append((build.build_id, member))
    for blob in blobs.values():
        blob.occurrences.sort()
    return blobs


def representative_name(blob: Blob) -> str:
    return sorted({member for _, member in blob.occurrences})[0]


def carriers(blob: Blob) -> list[str]:
    return sorted({build for build, _ in blob.occurrences})


def member_names(blob: Blob) -> list[str]:
    return sorted({member for _, member in blob.occurrences})


def sort_key(blob: Blob) -> tuple:
    return (blob.category, representative_name(blob), blob.sha256)


def resources_json(blobs: dict[str, Blob]) -> str:
    ordered = sorted(blobs.values(), key=sort_key)
    per_build_count: dict[str, int] = defaultdict(int)
    per_category_count: dict[str, int] = defaultdict(int)
    total_occurrences = 0
    for blob in ordered:
        per_category_count[blob.category] += 1
        total_occurrences += len(blob.occurrences)
        for build in carriers(blob):
            per_build_count[build] += 1
    doc = {
        "unique_blob_count": len(ordered),
        "total_occurrences": total_occurrences,
        "unique_by_category": dict(sorted(per_category_count.items())),
        "unique_blobs_per_build": dict(sorted(per_build_count.items())),
        "blobs": [
            {
                "sha256": blob.sha256,
                "size": blob.size,
                "category": blob.category,
                "member_names": member_names(blob),
                "carriers": carriers(blob),
                "occurrences": [
                    {"build": build, "member": member}
                    for build, member in blob.occurrences
                ],
            }
            for blob in ordered
        ],
    }
    return json.dumps(doc, sort_keys=True, indent=2, ensure_ascii=True) + "\n"


def resources_tsv(blobs: dict[str, Blob]) -> str:
    ordered = sorted(blobs.values(), key=sort_key)
    lines = ["\t".join([
        "sha256_12", "size", "category", "n_builds", "n_occurrences",
        "representative_name", "carriers",
    ])]
    for blob in ordered:
        lines.append("\t".join([
            blob.sha256[:12],
            str(blob.size),
            blob.category,
            str(len(carriers(blob))),
            str(len(blob.occurrences)),
            representative_name(blob),
            ",".join(carriers(blob)),
        ]))
    return "\n".join(lines) + "\n"


def write_outputs(blobs: dict[str, Blob], out_dir: Path) -> dict[str, Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = {
        "resources.json": resources_json(blobs),
        "resources.tsv": resources_tsv(blobs),
    }
    written = {}
    for name, text in files.items():
        path = out_dir / name
        path.write_text(text, encoding="utf-8", newline="")
        written[name] = path
    return written


def print_summary(blobs: dict[str, Blob]) -> None:
    ordered = sorted(blobs.values(), key=sort_key)
    per_category: dict[str, int] = defaultdict(int)
    shared = 0
    for blob in ordered:
        per_category[blob.category] += 1
        if len(carriers(blob)) > 1:
            shared += 1
    total_occ = sum(len(b.occurrences) for b in ordered)
    print(f"Resources: {len(ordered)} unique blobs from {total_occ} occurrences "
          f"across {len(corpus.builds())} surviving builds "
          f"({shared} blobs shared by >1 build).")
    print("Unique blobs by category:")
    for cat, count in sorted(per_category.items()):
        print(f"  {cat:9s} {count}")


def self_test() -> int:
    """R3: prove the dedup keys on content, not on name."""
    clean = analyze()
    candidates = [blob for blob in clean.values() if blob.size > 0]
    if not candidates:
        print("SELF-TEST FAILED: corpus has no non-empty resource to perturb.")
        return 3
    shared = [
        blob for blob in candidates
        if len({build for build, _ in blob.occurrences}) >= 2
    ]
    target = min(shared or candidates, key=lambda blob: blob.sha256)
    build_id, member = target.occurrences[0]

    # Negative control: perturb exactly one occurrence. It must leave the
    # original content bucket (or remove it when that was its sole occurrence)
    # while total occurrence count remains invariant.
    dirty = analyze(corrupt=(build_id, member))
    clean_count = len(target.occurrences)
    dirty_count = len(dirty[target.sha256].occurrences) if target.sha256 in dirty else 0
    clean_total = sum(len(blob.occurrences) for blob in clean.values())
    dirty_total = sum(len(blob.occurrences) for blob in dirty.values())
    if dirty_count != clean_count - 1 or dirty_total != clean_total:
        print("SELF-TEST FAILED: perturbing one resource did not move exactly "
              "one occurrence to a new content hash.")
        return 3
    print(f"self-test OK: perturbing one occurrence of {target.sha256[:12]} "
          f"moves it out of that hash bucket ({clean_count} -> {dirty_count}); "
          f"{len(shared)} blobs are shared across builds (R3).")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()

    blobs = analyze()
    written = write_outputs(blobs, args.out_dir)
    print_summary(blobs)
    print("\nWrote:")
    for name, path in written.items():
        print(f"  {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
