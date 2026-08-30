#!/usr/bin/env python3
"""Enforce the dependency direction of the reusable J2ME runtime crates."""

from __future__ import annotations

import argparse
import pathlib
import sys
import tempfile
import tomllib


ALLOWED_INTERNAL: dict[str, set[str]] = {
    "j2me-canvas": set(),
    "j2me-codec": set(),
    "j2me-device": set(),
    "j2me-device-nokia": {"j2me-device"},
    "j2me-jvm": set(),
    "j2me-media": set(),
    "j2me-me": {"j2me-canvas", "j2me-device", "j2me-jvm"},
    "j2me-nokia": {"j2me-canvas", "j2me-jvm", "j2me-me"},
    "j2me-input": {"j2me-device", "j2me-device-nokia"},
    "j2me-platform": {"j2me-device", "j2me-me", "j2me-media"},
    "j2me-platform-native": {
        "j2me-device",
        "j2me-input",
        "j2me-me",
        "j2me-media",
        "j2me-platform",
    },
    "j2me-platform-web": {
        "j2me-device",
        "j2me-me",
        "j2me-media",
        "j2me-platform",
    },
}

FORBIDDEN_PORTABLE_DEPS = {
    "cpal",
    "js-sys",
    "libc",
    "wasm-bindgen",
    "web-sys",
    "winit",
}


def _dependencies(document: dict) -> set[str]:
    names: set[str] = set()
    for table_name in ("dependencies", "dev-dependencies", "build-dependencies"):
        names.update(document.get(table_name, {}))
    for target in document.get("target", {}).values():
        for table_name in ("dependencies", "dev-dependencies", "build-dependencies"):
            names.update(target.get(table_name, {}))
    return names


def validate(root: pathlib.Path) -> list[str]:
    errors: list[str] = []
    crates_dir = root / "crates"
    manifests = sorted(crates_dir.glob("*/Cargo.toml"))
    if not manifests:
        manifest = root / "Cargo.toml"
        try:
            with manifest.open("rb") as handle:
                document = tomllib.load(handle)
        except (OSError, tomllib.TOMLDecodeError) as error:
            return [f"cannot read consumer workspace manifest: {error}"]
        runtime = {
            name: dependency
            for name, dependency in document.get("workspace", {}).get("dependencies", {}).items()
            if name.startswith("j2me-")
        }
        if not {"j2me-jvm", "j2me-me"}.issubset(runtime):
            errors.append("consumer workspace must pin at least j2me-jvm and j2me-me")
        for name, dependency in runtime.items():
            if not isinstance(dependency, dict):
                errors.append(f"consumer dependency {name!r} is not an explicit git revision")
                continue
            if "path" in dependency:
                errors.append(f"consumer dependency {name!r} must not use a copied path crate")
            revision = dependency.get("rev")
            if not dependency.get("git") or not isinstance(revision, str) or len(revision) != 40:
                errors.append(f"consumer dependency {name!r} must pin a 40-character git rev")
        return errors
    found: set[str] = set()

    for manifest in manifests:
        with manifest.open("rb") as handle:
            document = tomllib.load(handle)
        name = document.get("package", {}).get("name", "")
        if not name.startswith("j2me-"):
            continue
        found.add(name)
        if name not in ALLOWED_INTERNAL:
            errors.append(f"{manifest.relative_to(root)}: unclassified reusable crate {name!r}")
            continue

        dependencies = _dependencies(document)
        actual_internal = {dep for dep in dependencies if dep.startswith("j2me-")}
        forbidden = actual_internal - ALLOWED_INTERNAL[name]
        if forbidden:
            errors.append(
                f"{manifest.relative_to(root)}: forbidden internal dependencies: "
                + ", ".join(sorted(forbidden))
            )

        if name in {"j2me-me", "j2me-platform"}:
            leaked = dependencies & FORBIDDEN_PORTABLE_DEPS
            if leaked:
                errors.append(
                    f"{manifest.relative_to(root)}: host-specific dependencies leaked into "
                    f"portable runtime: {', '.join(sorted(leaked))}"
                )

        source_dir = manifest.parent / "src"
        for source in sorted(source_dir.rglob("*.rs")):
            if "stalker" in source.read_text(encoding="utf-8").lower():
                errors.append(
                    f"{source.relative_to(root)}: game-specific 'stalker' token in reusable crate"
                )

    missing = set(ALLOWED_INTERNAL) - found
    if missing:
        errors.append("missing reusable crate manifests: " + ", ".join(sorted(missing)))
    return errors


def self_test() -> int:
    with tempfile.TemporaryDirectory() as temporary:
        root = pathlib.Path(temporary)
        for name, allowed in ALLOWED_INTERNAL.items():
            crate = root / "crates" / name
            (crate / "src").mkdir(parents=True)
            deps = "\n".join(f'{dep} = {{ path = "../{dep}" }}' for dep in sorted(allowed))
            (crate / "Cargo.toml").write_text(
                f'[package]\nname = "{name}"\nversion = "0.0.0"\n[dependencies]\n{deps}\n',
                encoding="utf-8",
            )
            (crate / "src" / "lib.rs").write_text("", encoding="utf-8")

        if validate(root):
            print("architecture self-test fixture unexpectedly failed", file=sys.stderr)
            return 1

        manifest = root / "crates" / "j2me-me" / "Cargo.toml"
        manifest.write_text(
            manifest.read_text(encoding="utf-8")
            + '\nj2me-platform-native = { path = "../j2me-platform-native" }\n',
            encoding="utf-8",
        )
        errors = validate(root)
        if not any("j2me-platform-native" in error for error in errors):
            print("architecture self-test did not reject a reverse dependency", file=sys.stderr)
            return 1

        manifest.write_text(
            manifest.read_text(encoding="utf-8") + '\nwinit = "0.30"\n',
            encoding="utf-8",
        )
        errors = validate(root)
        if not any("host-specific" in error for error in errors):
            print("architecture self-test did not reject a host dependency", file=sys.stderr)
            return 1

    print("architecture self-test: injected violations were rejected")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path(__file__).parents[2])
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    errors = validate(args.root.resolve())
    if errors:
        for error in errors:
            print(f"architecture: {error}", file=sys.stderr)
        return 1
    print("architecture: reusable crate dependency directions are valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
