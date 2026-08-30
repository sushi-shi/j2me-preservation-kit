#!/usr/bin/env python3
"""Compile the canonical named Java reconstruction and optionally package a JAR.

The Java ME API stubs (`java/api-stubs/`) are compiled into a separate
temporary class path. They are never copied into the application classes or a
resulting JAR: a phone or emulator must provide the MIDP/LCDUI/RMS/MMAPI
APIs at runtime, exactly as it did for the original game.

When ``--jar`` is requested, the verified baseline's non-class resources are
copied into the ignored output JAR and its MIDlet entry is pointed at the named
class. Original classes and API stubs are never packaged.

The canonical tree under `java/src/main/java/` is recovered incrementally
(rulebook: the bytecode is the authority, the decompiles are drafting aids,
`java/reconstruction/symbols.toml` is the naming authority). This tool proves
the recovered source is well-formed, self-consistent Java at every step.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import shutil
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore

ROOT = Path(__file__).resolve().parents[2]
SOURCE_ROOT = ROOT / "java/src/main/java"
STUB_ROOT = ROOT / "java/api-stubs"
sys.path.insert(0, str(ROOT / "tools" / "corpus"))

import corpus  # noqa: E402

with (ROOT / "game.toml").open("rb") as handle:
    CONFIG = tomllib.load(handle)
JAVA_CONFIG = CONFIG.get("java", {})
SLUG = CONFIG.get("slug", "j2me-game")
CANONICAL_MIDLET = JAVA_CONFIG.get("canonical_midlet", "")
CANONICAL_CLASS_ENTRIES = sorted(JAVA_CONFIG.get("canonical_class_entries", []))
MINIMUM_JAR_ENTRIES = JAVA_CONFIG.get("minimum_jar_entries", 0)


def java_sources(root: Path) -> list[Path]:
    sources = sorted(root.rglob("*.java"))
    if not sources:
        raise SystemExit(f"no Java sources found under {root}")
    return sources


def run_javac(sources: list[Path], destination: Path, class_path: Path | None = None) -> None:
    command = [
        "javac",
        "-encoding",
        "UTF-8",
        "--release",
        "8",
        "-d",
        str(destination),
    ]
    if class_path is not None:
        command.extend(["-classpath", str(class_path)])
    command.extend(map(str, sources))
    # cwd under git-ignored _temp/ so a dropped javac argfile never lands in
    # the repo root.
    scratch = ROOT / "_temp"
    scratch.mkdir(exist_ok=True)
    subprocess.run(command, check=True, cwd=scratch)


def compile_source(emit_classes: Path | None = None) -> None:
    with tempfile.TemporaryDirectory(prefix=f"{SLUG}-java-build-") as temporary:
        temporary_root = Path(temporary)
        stub_classes = temporary_root / "stub-classes"
        application_classes = temporary_root / "application-classes"
        stub_classes.mkdir()
        application_classes.mkdir()

        run_javac(java_sources(STUB_ROOT), stub_classes)
        run_javac(java_sources(SOURCE_ROOT), application_classes, stub_classes)

        if emit_classes is not None:
            if emit_classes.exists():
                shutil.rmtree(emit_classes)
            shutil.copytree(application_classes, emit_classes)


def baseline_payload() -> bytes:
    baseline = corpus.load_manifest()["baseline"]
    return next(build.payload for build in corpus.builds() if build.build_id == baseline)


def canonical_manifest(original: bytes) -> bytes:
    if not CANONICAL_MIDLET:
        raise SystemExit("game.toml [java].canonical_midlet must be configured")
    lines = original.decode("ISO-8859-1").replace("\r\n", "\n").splitlines()
    output = []
    replaced = False
    for line in lines:
        if line.startswith("MIDlet-1:"):
            parts = line.split(",")
            if len(parts) != 3:
                raise SystemExit(f"unexpected MIDlet-1 manifest row: {line!r}")
            parts[2] = f" {CANONICAL_MIDLET}"
            line = ",".join(parts)
            replaced = True
        output.append(line)
    if not replaced:
        raise SystemExit("baseline manifest has no MIDlet-1 row")
    return ("\r\n".join(output) + "\r\n").encode("ISO-8859-1")


def jar_info(name: str) -> zipfile.ZipInfo:
    entry = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
    entry.external_attr = 0o100644 << 16
    entry.compress_type = zipfile.ZIP_DEFLATED
    return entry


def write_jar(classes_root: Path, destination: Path) -> None:
    """Write a deterministic runnable JAR from named classes + baseline assets."""

    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(destination.suffix + ".tmp")
    if temporary.exists():
        temporary.unlink()
    payload = baseline_payload()
    with zipfile.ZipFile(temporary, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        with zipfile.ZipFile(io.BytesIO(payload)) as original:
            manifest = original.read("META-INF/MANIFEST.MF")
            archive.writestr(jar_info("META-INF/MANIFEST.MF"), canonical_manifest(manifest))
            resources = sorted(
                name for name in original.namelist()
                if not name.endswith(("/", ".class")) and name != "META-INF/MANIFEST.MF"
            )
            for name in resources:
                archive.writestr(jar_info(name), original.read(name))
        for source in sorted(classes_root.rglob("*.class")):
            name = source.relative_to(classes_root).as_posix()
            archive.writestr(jar_info(name), source.read_bytes())
    temporary.replace(destination)


def validate_jar(path: Path) -> None:
    if not CANONICAL_CLASS_ENTRIES or MINIMUM_JAR_ENTRIES <= 0:
        raise ValueError(
            "game.toml [java] must define canonical_class_entries and "
            "minimum_jar_entries"
        )
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        app_classes = sorted(name for name in names if name.endswith(".class"))
        if any(name.startswith("javax/") for name in names):
            raise ValueError("Java ME API stub leaked into application JAR")
        if app_classes != CANONICAL_CLASS_ENTRIES:
            raise ValueError(f"application class closure differs: {app_classes}")
        manifest = archive.read("META-INF/MANIFEST.MF")
        selected_midlet = f", {CANONICAL_MIDLET}\r\n".encode("ISO-8859-1")
        if selected_midlet not in manifest:
            raise ValueError("manifest does not select the canonical MIDlet")
        if len(names) < MINIMUM_JAR_ENTRIES:
            raise ValueError("resource merge is vacuous")


def build_jar(destination: Path) -> None:
    with tempfile.TemporaryDirectory(prefix=f"{SLUG}-java-jar-") as temporary:
        classes = Path(temporary) / "classes"
        compile_source(classes)
        write_jar(classes, destination)
    validate_jar(destination)


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix=f"{SLUG}-java-self-test-") as temporary:
        root = Path(temporary)
        first = root / "first.jar"
        second = root / "second.jar"
        build_jar(first)
        build_jar(second)
        if hashlib.sha256(first.read_bytes()).digest() != hashlib.sha256(second.read_bytes()).digest():
            raise SystemExit("java-build self-test FAIL: two clean builds differ")
        bad = root / "bad.jar"
        shutil.copyfile(first, bad)
        with zipfile.ZipFile(bad, "a") as archive:
            archive.writestr(jar_info("javax/microedition/Fake.class"), b"not a class")
        try:
            validate_jar(bad)
        except ValueError as error:
            if "stub leaked" not in str(error):
                raise
        else:
            raise SystemExit("java-build self-test FAIL: injected API stub was accepted")
    print("java-build self-test OK: builds are deterministic and an injected API stub is rejected")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--jar",
        type=Path,
        help="package the compiled application classes into this deterministic JAR",
    )
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args()
    if arguments.self_test:
        return self_test()
    if arguments.jar is None:
        compile_source()
        print(f"typecheck ok: {len(java_sources(SOURCE_ROOT))} canonical sources")
        return 0
    build_jar(arguments.jar)
    print(f"wrote {arguments.jar}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
