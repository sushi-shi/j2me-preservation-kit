#!/usr/bin/env python3
"""Decompile a recovery-target build's classes into git-ignored `_reference/`.

Generated decompiler output is EVIDENCE, never hand-edited source (it lives under
the ignored `_reference/` tree). We run two independent decompilers — jadx
(primary, readable) and cfr (cross-check) — so a suspicious jadx rendering can be
checked against cfr and, ultimately, `javap` bytecode.

The build defaults to `builds.toml`'s `baseline`; pass an id to override. The
payload is located from its `containers` (a top-level `_originals/<file>` or a
jar nested `inside _originals/<zip>`) and extracted to a scratch dir — the
immutable `_originals/` is never modified.

Usage:
    decompile.py [<build-id>]
"""
from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import zipfile
import hashlib
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore

REPO = Path(__file__).resolve().parents[2]
BUILDS = REPO / "java" / "reconstruction" / "builds.toml"
ORIGINALS = REPO / "_originals"
OUT_ROOT = REPO / "_reference" / "decompiled"


def load() -> dict:
    with BUILDS.open("rb") as fh:
        return tomllib.load(fh)


def find_payload(manifest: dict, build_id: str) -> dict:
    for entry in manifest.get("payload", []) + manifest.get("archived", []):
        if entry["id"] == build_id:
            return entry
    sys.exit(f"decompile: build id not found in builds.toml: {build_id}")


def materialize_jar(entry: dict, scratch: Path) -> Path:
    """Return a path to the build's .jar bytes (extracting from a zip if nested)."""
    for c in entry.get("containers", []):
        c = c.strip()
        if c.startswith("_originals/"):
            p = REPO / c
            if p.is_file():
                data = p.read_bytes()
                if hashlib.sha256(data).hexdigest() != entry["sha256"]:
                    sys.exit(f"decompile: sha256 mismatch for {entry['id']}: {p}")
                if len(data) != entry["bytes"]:
                    sys.exit(f"decompile: byte-count mismatch for {entry['id']}: {p}")
                return p
    # nested inside a zip
    for c in entry.get("containers", []):
        c = c.strip()
        marker = "inside _originals/"
        if c.startswith(marker):
            zip_path = REPO / "_originals" / Path(c[len(marker):]).name
            with zipfile.ZipFile(zip_path) as zf:
                for name in zf.namelist():
                    if name.lower().endswith(".jar"):
                        blob = zf.read(name)
                        out = scratch / Path(name).name
                        out.write_bytes(blob)
                        # verify against the recorded sha256
                        if (hashlib.sha256(blob).hexdigest() == entry["sha256"]
                                and len(blob) == entry["bytes"]):
                            return out
    sys.exit(f"decompile: could not locate jar bytes for {entry['id']}")


def main(argv: list[str]) -> int:
    manifest = load()
    build_id = argv[0] if argv else manifest.get("baseline")
    if not build_id:
        sys.exit("decompile: no build id and no baseline in builds.toml")
    entry = find_payload(manifest, build_id)

    out_dir = OUT_ROOT / build_id
    if out_dir.exists():
        shutil.rmtree(out_dir)
    (out_dir / "jadx").mkdir(parents=True)
    (out_dir / "cfr").mkdir(parents=True)

    with tempfile.TemporaryDirectory() as td:
        scratch = Path(td)
        jar = materialize_jar(entry, scratch)
        print(f"decompile: {build_id} <- {jar.name} ({jar.stat().st_size} bytes)")

        # jadx (primary). --no-res: we only want the source. Deterministic output.
        jadx = shutil.which("jadx")
        if jadx:
            subprocess.run(
                [jadx, "--no-res", "--no-imports", "-d", str(out_dir / "jadx"), str(jar)],
                check=False,
            )
            n = len(list((out_dir / "jadx").rglob("*.java")))
            print(f"  jadx: {n} .java files -> {out_dir / 'jadx'}")
        else:
            print("  jadx: not on PATH (run inside `nix develop`)")

        # cfr (cross-check).
        cfr = shutil.which("cfr")
        if cfr:
            res = subprocess.run(
                [cfr, str(jar), "--outputdir", str(out_dir / "cfr")],
                check=False, capture_output=True, text=True,
            )
            n = len(list((out_dir / "cfr").rglob("*.java")))
            print(f"  cfr: {n} .java files -> {out_dir / 'cfr'}")
            if res.returncode != 0 and n == 0:
                print(res.stderr[-500:])
        else:
            print("  cfr: not on PATH (run inside `nix develop`)")

    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
