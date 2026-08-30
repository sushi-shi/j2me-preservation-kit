#!/usr/bin/env python3
"""Classify and fingerprint every surviving J2ME build.

Parses every `.class` in each recorded JAR straight from bytes (no decompiler)
and emits three regenerable artifacts into the git-ignored `_reference/`:

  * ``class-inventory.json`` -- per build, every class with its ``class_sha256``,
    a name-blind ``shape_sha256``, and per-method ``code_sha256`` /
    ``opcode_sha256`` / ``shape_sha256`` fingerprints.
  * ``class-delta.json`` -- code families (builds whose game-class shape-set is
    identical) and pairwise-vs-baseline bytecode-similarity metrics. Classes are
    matched on shape first, name only as a fallback (rulebook R10).
  * ``builds.tsv`` -- one row per build: provenance and class/method
    counts, and its reviewed code family.

Regenerating is byte-identical (R3): output is fully sorted, ASCII-escaped, and
timestamp-free. `--self-test` proves that perturbing one class byte changes the
fingerprinted baseline without changing another build.
"""

from __future__ import annotations

import argparse
import io
import json
import sys
import zipfile
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

import classfile
import corpus

REPO = corpus.REPO
DEFAULT_OUT = REPO / "_reference"

# Device/API adapter classes bundled by some SKUs are not the game's
# transliteration surface. Game classes may themselves use packages, so exclude
# only well-known Java ME platform namespaces rather than every slash-bearing
# name.
PLATFORM_PREFIXES = (
    "java/",
    "javax/",
    "com/nokia/",
    "com/siemens/",
    "com/motorola/",
    "com/samsung/",
    "com/sony/",
    "com/sonyericsson/",
)


def is_game_class(info: classfile.ClassInfo) -> bool:
    return not info.internal_name.startswith(PLATFORM_PREFIXES)


@dataclass
class BuildAnalysis:
    build_id: str
    sha256: str
    size: int
    declared_language: str
    content_language: str | None
    official: object
    provenance_class: str
    collection: str
    classes: list[classfile.ClassInfo]

    @property
    def game_classes(self) -> list[classfile.ClassInfo]:
        return [c for c in self.classes if is_game_class(c)]

    @property
    def game_shape_set(self) -> tuple[str, ...]:
        return tuple(sorted(c.shape_sha256 for c in self.game_classes))

    def method_count(self) -> int:
        return sum(len(c.methods) for c in self.classes)

    def game_method_count(self) -> int:
        return sum(len(c.methods) for c in self.game_classes)


def content_language(payload: bytes) -> str | None:
    """Return a language only when the archive itself carries an unambiguous tag."""
    with zipfile.ZipFile(io.BytesIO(payload)) as jar:
        prefixes = {
            name.split(".", 1)[0].lower()
            for name in jar.namelist()
            if name.lower().endswith((".lng", ".lang"))
        }
    known = {p for p in prefixes if p in {"en", "ru", "de", "pl", "cz"}}
    if len(known) == 1:
        return next(iter(known))
    return None


def analyze(*, corrupt: tuple[str, str] | None = None) -> list[BuildAnalysis]:
    """Parse every surviving build. `corrupt=(build_id, member)` flips one
    byte of that class member before fingerprinting (used by --self-test)."""
    analyses: list[BuildAnalysis] = []
    for build in corpus.builds():
        with zipfile.ZipFile(io.BytesIO(build.payload)) as jar:
            class_members = sorted(n for n in jar.namelist() if n.endswith(".class"))
            classes = []
            for member in class_members:
                data = jar.read(member)
                if corrupt is not None and corrupt == (build.build_id, member) and data:
                    # Flip a byte deep inside the class (well past the header) so
                    # it lands in the method code, not the magic/version.
                    idx = len(data) // 2
                    data = data[:idx] + bytes([data[idx] ^ 0xFF]) + data[idx + 1:]
                classes.append(classfile.parse_class(member, data))
        analyses.append(
            BuildAnalysis(
                build_id=build.build_id,
                sha256=build.sha256,
                size=build.size,
                declared_language=build.declared_language,
                content_language=content_language(build.payload),
                official=build.official,
                provenance_class=build.provenance_class,
                collection=build.collection,
                classes=classes,
            )
        )
    return analyses


# --------------------------------------------------------------------------- #
# Cross-build correspondence and similarity.
# --------------------------------------------------------------------------- #

def method_multiset(classes: list[classfile.ClassInfo], key: str) -> Counter:
    return Counter(getattr(m, key) for c in classes for m in c.methods)


def dice(left: Counter, right: Counter) -> float:
    total = sum(left.values()) + sum(right.values())
    if total == 0:
        return 1.0
    overlap = sum((left & right).values())
    return 2.0 * overlap / total


def match_classes(base: BuildAnalysis, other: BuildAnalysis) -> list[dict]:
    """Correspond base game classes to other's, shape first then name (R10)."""
    other_by_shape: dict[str, list[classfile.ClassInfo]] = {}
    for info in other.game_classes:
        other_by_shape.setdefault(info.shape_sha256, []).append(info)
    other_by_name = {info.internal_name: info for info in other.game_classes}
    used: set[str] = set()
    rows: list[dict] = []
    for info in base.game_classes:
        match = None
        method = "unmatched"
        # 1) exact structural (name-blind) match.
        candidates = [
            c for c in other_by_shape.get(info.shape_sha256, [])
            if c.internal_name not in used
        ]
        if candidates:
            candidates.sort(key=lambda c: c.internal_name)
            match = candidates[0]
            method = "shape"
        # 2) fall back to the obfuscated name.
        elif info.internal_name in other_by_name:
            candidate = other_by_name[info.internal_name]
            if candidate.internal_name not in used:
                match = candidate
                method = "name"
        row = {
            "base_class": info.internal_name,
            "base_shape": info.shape_sha256,
            "base_methods": len(info.methods),
        }
        if match is not None:
            used.add(match.internal_name)
            base_shapes = Counter(m.shape_sha256 for m in info.methods)
            other_shapes = Counter(m.shape_sha256 for m in match.methods)
            row.update(
                match_method=method,
                other_class=match.internal_name,
                other_shape=match.shape_sha256,
                other_methods=len(match.methods),
                shape_identical=info.shape_sha256 == match.shape_sha256,
                method_shape_dice=round(dice(base_shapes, other_shapes), 6),
            )
        else:
            row.update(match_method="unmatched", other_class=None)
        rows.append(row)
    return sorted(rows, key=lambda r: r["base_class"])


def pairwise(base: BuildAnalysis, other: BuildAnalysis) -> dict:
    base_g = base.game_classes
    other_g = other.game_classes
    code_dice = dice(method_multiset(base_g, "code_sha256"),
                     method_multiset(other_g, "code_sha256"))
    opcode_dice = dice(method_multiset(base_g, "opcode_sha256"),
                       method_multiset(other_g, "opcode_sha256"))
    shape_dice = dice(method_multiset(base_g, "shape_sha256"),
                      method_multiset(other_g, "shape_sha256"))
    code_shared = sum((method_multiset(base_g, "code_sha256")
                       & method_multiset(other_g, "code_sha256")).values())
    shape_shared = sum((method_multiset(base_g, "shape_sha256")
                        & method_multiset(other_g, "shape_sha256")).values())
    class_rows = match_classes(base, other)
    return {
        "base": base.build_id,
        "other": other.build_id,
        "base_game_methods": base.game_method_count(),
        "other_game_methods": other.game_method_count(),
        "game_methods_code_identical": code_shared,
        "game_methods_shape_shared": shape_shared,
        "game_method_code_dice": round(code_dice, 6),
        "game_method_opcode_dice": round(opcode_dice, 6),
        "game_method_shape_dice": round(shape_dice, 6),
        "classes": class_rows,
    }


def code_families(analyses: list[BuildAnalysis]) -> dict[str, list[str]]:
    """Group builds whose whole game-class shape-set is identical."""
    families: dict[tuple[str, ...], list[str]] = {}
    for build in analyses:
        families.setdefault(build.game_shape_set, []).append(build.build_id)
    # Name families after their alphabetically-first member (deterministic).
    named: dict[str, list[str]] = {}
    for members in families.values():
        named["+".join(sorted(members))] = sorted(members)
    return dict(sorted(named.items()))


# --------------------------------------------------------------------------- #
# Serialization.
# --------------------------------------------------------------------------- #

def inventory_json(analyses: list[BuildAnalysis]) -> str:
    builds = []
    for build in analyses:
        classes = []
        for info in sorted(build.classes, key=lambda c: c.member_path):
            methods = [
                {
                    "name": m.name,
                    "descriptor": m.descriptor,
                    "access_flags": m.access_flags,
                    "code_size": m.code_size,
                    "opcode_count": m.opcode_count,
                    "code_sha256": m.code_sha256,
                    "opcode_sha256": m.opcode_sha256,
                    "shape_sha256": m.shape_sha256,
                }
                for m in sorted(info.methods, key=lambda m: (m.name, m.descriptor,
                                                             m.ordinal))
            ]
            external_apis = sorted({
                call.split(".", 1)[0]
                for m in info.methods
                for call in m.calls
                if "/" in call.split(".", 1)[0]  # library/adapter owner, not a game class
            })
            classes.append({
                "member_path": info.member_path,
                "internal_name": info.internal_name,
                "is_game_class": is_game_class(info),
                "class_sha256": info.class_sha256,
                "shape_sha256": info.shape_sha256,
                "super_name": info.super_name,
                "interfaces": sorted(info.interfaces),
                "external_apis": external_apis,
                "major_version": info.major_version,
                "field_count": len(info.fields),
                "method_count": len(info.methods),
                "methods": methods,
            })
        builds.append({
            "build_id": build.build_id,
            "sha256": build.sha256,
            "size": build.size,
            "declared_language": build.declared_language,
            "content_language": build.content_language,
            "official": build.official,
            "provenance_class": build.provenance_class,
            "collection": build.collection,
            "class_count": len(build.classes),
            "game_class_count": len(build.game_classes),
            "method_count": build.method_count(),
            "game_method_count": build.game_method_count(),
            "classes": classes,
        })
    return json.dumps({"builds": builds}, sort_keys=True, indent=2,
                      ensure_ascii=True) + "\n"


def delta_json(analyses: list[BuildAnalysis]) -> str:
    baseline = analyses[0]
    pairs = [pairwise(baseline, other) for other in analyses[1:]]
    # Methods whose shape is present in every surviving build.
    per_build_shapes = [
        method_multiset(b.game_classes, "shape_sha256") for b in analyses
    ]
    common = per_build_shapes[0].copy()
    for counter in per_build_shapes[1:]:
        common &= counter
    doc = {
        "baseline": baseline.build_id,
        "code_families": code_families(analyses),
        "baseline_game_method_count": baseline.game_method_count(),
        "game_methods_shape_common_to_all": sum(common.values()),
        "pairwise_vs_baseline": pairs,
    }
    return json.dumps(doc, sort_keys=True, indent=2, ensure_ascii=True) + "\n"


def builds_tsv(analyses: list[BuildAnalysis], families: dict[str, list[str]]) -> str:
    family_of = {bid: name for name, members in families.items() for bid in members}
    lines = ["\t".join([
        "build_id", "sha256_12", "official", "provenance_class", "declared_language",
        "content_language", "class_count", "game_class_count", "method_count",
        "game_method_count", "code_family",
    ])]
    for build in analyses:
        lines.append("\t".join([
            build.build_id,
            build.sha256[:12],
            str(build.official),
            build.provenance_class,
            build.declared_language or "?",
            build.content_language or "?",
            str(len(build.classes)),
            str(len(build.game_classes)),
            str(build.method_count()),
            str(build.game_method_count()),
            family_of.get(build.build_id, "?"),
        ]))
    return "\n".join(lines) + "\n"


def write_outputs(analyses: list[BuildAnalysis], out_dir: Path) -> dict[str, Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    families = code_families(analyses)
    files = {
        "class-inventory.json": inventory_json(analyses),
        "class-delta.json": delta_json(analyses),
        "builds.tsv": builds_tsv(analyses, families),
    }
    written = {}
    for name, text in files.items():
        path = out_dir / name
        path.write_text(text, encoding="utf-8", newline="")
        written[name] = path
    return written


def print_summary(analyses: list[BuildAnalysis]) -> None:
    families = code_families(analyses)
    print("Surviving builds classified:")
    for build in analyses:
        print(f"  {build.build_id:25s} declared={build.declared_language or '?':7s} "
              f"content={build.content_language or '?':3s} classes={len(build.classes):2d} "
              f"(game={len(build.game_classes)}) methods={build.method_count():4d} "
              f"official={build.official}")
    print("\nCode families (identical game-class shape-set):")
    for name, members in families.items():
        print(f"  {name}")
    baseline = analyses[0]
    print(f"\nBaseline: {baseline.build_id} — "
          f"{baseline.game_method_count()} methods across "
          f"{len(baseline.game_classes)} game classes (transliteration surface).")
    for other in analyses[1:]:
        pw = pairwise(baseline, other)
        print(f"  vs {other.build_id:20s} "
              f"code_dice={pw['game_method_code_dice']:.3f} "
              f"opcode_dice={pw['game_method_opcode_dice']:.3f} "
              f"shape_dice={pw['game_method_shape_dice']:.3f} "
              f"({pw['game_methods_code_identical']} methods byte-identical)")


def self_test() -> int:
    """R3: prove the classifier's central claims are falsifiable."""
    clean = analyze()
    baseline = clean[0]
    target = max(baseline.game_classes, key=lambda info: len(info.methods)).member_path
    dirty = analyze(corrupt=(baseline.build_id, target))
    dirty_by_id = {build.build_id: build for build in dirty}
    if dirty_by_id[baseline.build_id].game_shape_set == baseline.game_shape_set:
        print("SELF-TEST FAILED: a one-byte baseline perturbation did not change "
              "its shape-set (vacuous fingerprint).")
        return 3
    controls = clean[1:]
    for control in controls:
        if dirty_by_id[control.build_id].game_shape_set != control.game_shape_set:
            print("SELF-TEST FAILED: perturbing the baseline changed another build.")
            return 3
    isolation = "only the baseline" if controls else "the single recorded build"
    print(f"self-test OK: perturbing {baseline.build_id}/{target} changes "
          f"{isolation}'s shape-set (fingerprints are load-bearing, R3).")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()

    analyses = analyze()
    written = write_outputs(analyses, args.out_dir)
    print_summary(analyses)
    print("\nWrote:")
    for name, path in written.items():
        print(f"  {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
