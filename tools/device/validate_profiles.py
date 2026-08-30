#!/usr/bin/env python3
"""Validate game-owned device fragments and per-build profile assignments."""

from __future__ import annotations

import argparse
import pathlib
import sys
import tempfile
import tomllib


AXES = (
    "display",
    "input",
    "media",
    "haptics",
    "rms",
    "connector",
    "system",
    "font",
    "lifecycle",
)


def validate(catalog_path: pathlib.Path, builds_path: pathlib.Path) -> list[str]:
    errors: list[str] = []
    try:
        with catalog_path.open("rb") as handle:
            catalog = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        return [f"cannot read {catalog_path}: {error}"]
    try:
        with builds_path.open("rb") as handle:
            builds = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        return [f"cannot read {builds_path}: {error}"]

    if catalog.get("schema_version") != 1:
        errors.append("device-profiles.toml schema_version must be 1")
    profiles: dict[str, dict] = {}
    for index, profile in enumerate(catalog.get("profile", [])):
        profile_id = profile.get("id")
        if not isinstance(profile_id, str) or not profile_id:
            errors.append(f"profile row {index} has no non-empty id")
            continue
        if profile_id in profiles:
            errors.append(f"duplicate device profile id {profile_id!r}")
        profiles[profile_id] = profile
        unknown = set(profile) - ({"id"} | set(AXES))
        if unknown:
            errors.append(f"profile {profile_id!r} has unknown axes {sorted(unknown)}")
        for axis in AXES:
            fragment = profile.get(axis)
            if not isinstance(fragment, str) or not fragment:
                errors.append(f"profile {profile_id!r} has no {axis} fragment reference")
            elif fragment not in catalog.get(axis, {}):
                errors.append(
                    f"profile {profile_id!r} references missing {axis} fragment {fragment!r}"
                )

    for table_name in ("payload", "archived"):
        for payload in builds.get(table_name, []):
            if payload.get("kind") != "jar":
                continue
            build_id = payload.get("id", "<unnamed>")
            profile_id = payload.get("device_profile")
            if not isinstance(profile_id, str) or not profile_id:
                errors.append(f"build {build_id!r} has no reviewed device_profile")
            elif profile_id not in profiles:
                errors.append(
                    f"build {build_id!r} selects unknown device profile {profile_id!r}"
                )
    return errors


def self_test() -> int:
    catalog = b'''schema_version = 1
[display.d]\nwidth=1\nheight=1
[input.i]\nup=1\ndown=2\nleft=3\nright=4\nfire=5\nsoft_left=6\nsoft_right=7\nstar=42\npound=35\ndigits=[0,1,2,3,4,5,6,7,8,9]
[media.m]\ncontent_types=[]\ncontrols=[]\nmidi_renderer="none"
[haptics.h]\nvibration=false
[rms.r]
[connector.c]\nschemes=[]
[system.s]\ndefault_charset="UTF-8"\nproperties={"microedition.platform"="FixturePhone"}
[font.f]\nprovider="fixture"
[lifecycle.l]\nfocus_loss="none"\nfocus_gain="none"
[[profile]]\nid="phone"\ndisplay="d"\ninput="i"\nmedia="m"\nhaptics="h"\nrms="r"\nconnector="c"\nsystem="s"\nfont="f"\nlifecycle="l"
'''
    with tempfile.TemporaryDirectory() as temporary:
        root = pathlib.Path(temporary)
        catalog_path = root / "device-profiles.toml"
        builds_path = root / "builds.toml"
        catalog_path.write_bytes(catalog)
        builds_path.write_text(
            '[[payload]]\nid="build"\nkind="jar"\ndevice_profile="phone"\n', encoding="utf-8"
        )
        if validate(catalog_path, builds_path):
            print("device-profile self-test valid fixture failed", file=sys.stderr)
            return 1
        catalog_path.write_bytes(catalog.replace(b'system="s"\n', b""))
        missing_system = validate(catalog_path, builds_path)
        if not any("has no system fragment reference" in error for error in missing_system):
            print("device-profile self-test did not reject a missing system axis", file=sys.stderr)
            return 1
        catalog_path.write_bytes(catalog)
        builds_path.write_text(
            '[[payload]]\nid="build"\nkind="jar"\ndevice_profile="missing"\n', encoding="utf-8"
        )
        if not validate(catalog_path, builds_path):
            print("device-profile self-test did not reject unknown assignment", file=sys.stderr)
            return 1
    print("device-profile self-test: missing system axis and unknown build assignment were rejected")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--catalog", type=pathlib.Path, default=pathlib.Path("device-profiles.toml"))
    parser.add_argument("--builds", type=pathlib.Path, default=pathlib.Path("java/reconstruction/builds.toml"))
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    errors = validate(args.catalog, args.builds)
    if errors:
        for error in errors:
            print(f"device-profile: {error}", file=sys.stderr)
        return 1
    print("device-profile: catalog composition and build assignments are valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
