#!/usr/bin/env python3
"""Validate reviewed method/signature variants against every selected JAR.

One schema supports both stable method tables (``identity = "ordinal"``) and
builds whose signatures appear/disappear (``identity = "signature"``). The
ledger stores the judgment; this tool recomputes presence, raw signature, and
method-shape groups directly from original classfiles.
"""

from __future__ import annotations

import argparse
import copy
import json
import sys
import tomllib
from collections import defaultdict
from pathlib import Path
from typing import Sequence

ROOT = Path(__file__).resolve().parents[2]
CORPUS_TOOLS = ROOT / "tools" / "corpus"
sys.path.insert(0, str(CORPUS_TOOLS))

import classfile  # noqa: E402
import corpus  # noqa: E402

CLASSIFICATIONS = {
    "common",
    "common-semantic-with-policy",
    "lineage-adapter",
    "device-policy",
    "content-balance-policy",
    "localization-policy",
    "symbol-layout-adapter",
}


class VariantError(RuntimeError):
    pass


def method_key(method: classfile.MethodSymbol, identity: str) -> str:
    if identity == "ordinal":
        return str(method.ordinal)
    if identity == "signature":
        return f"{method.name}:{method.descriptor}"
    raise VariantError(f"unknown variant identity {identity!r}")


def collect_live(owner: str, build_ids: Sequence[str], identity: str) -> dict[str, dict[str, tuple[str, str, str] | None]]:
    wanted = set(build_ids)
    selected = {build.build_id: build for build in corpus.builds() if build.build_id in wanted}
    missing = sorted(wanted - set(selected))
    if missing:
        raise VariantError(f"variant builds are absent from the corpus: {missing}")
    per_build: dict[str, dict[str, tuple[str, str, str]]] = {}
    all_keys: set[str] = set()
    for build_id in build_ids:
        classes = {}
        for member, data in corpus.jar_members(selected[build_id].payload):
            if member.endswith(".class"):
                info = classfile.parse_class(member, data)
                classes[info.internal_name] = info
        if owner not in classes:
            raise VariantError(f"{build_id}: class {owner!r} is absent")
        methods: dict[str, tuple[str, str, str]] = {}
        for method in classes[owner].methods:
            key = method_key(method, identity)
            if key in methods:
                raise VariantError(f"{build_id}: duplicate variant identity {key!r}")
            methods[key] = (method.name, method.descriptor, method.shape_sha256 or "no-code")
        per_build[build_id] = methods
        all_keys.update(methods)
    return {
        key: {build: per_build[build].get(key) for build in build_ids}
        for key in sorted(all_keys)
    }


def grouped_observations(
    observations: dict[str, tuple[str, str, str] | None]
) -> list[dict]:
    groups: dict[tuple[str, str, str] | None, list[str]] = defaultdict(list)
    for build, observation in observations.items():
        groups[observation].append(build)
    result = []
    for observation, builds in groups.items():
        if observation is None:
            result.append({"builds": sorted(builds), "present": False})
        else:
            name, descriptor, shape = observation
            result.append(
                {
                    "builds": sorted(builds),
                    "present": True,
                    "name": name,
                    "descriptor": descriptor,
                    "shape_sha256": shape,
                }
            )
    return sorted(result, key=lambda row: (not row["present"], row["builds"]))


def normalized_manifest_observations(rows: object) -> list[dict]:
    if not isinstance(rows, list):
        return []
    normalized = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        value = {
            "builds": sorted(row.get("builds", [])),
            "present": row.get("present"),
        }
        if row.get("present") is True:
            value.update(
                name=row.get("name"),
                descriptor=row.get("descriptor"),
                shape_sha256=row.get("shape_sha256"),
            )
        normalized.append(value)
    return sorted(normalized, key=lambda row: (not row["present"], row["builds"]))


def validate(manifest: dict, live: dict[str, dict[str, tuple[str, str, str] | None]]) -> list[str]:
    errors: list[str] = []
    if manifest.get("schema_version") != 1:
        errors.append("unsupported variant schema_version")
    builds = manifest.get("builds", [])
    if not isinstance(builds, list) or not builds or len(builds) != len(set(builds)):
        errors.append("variant builds must be a non-empty unique list")
        builds = []
    if manifest.get("expected_build_count") != len(builds):
        errors.append(
            f"expected_build_count is {manifest.get('expected_build_count')!r}, ledger has {len(builds)}"
        )
    if manifest.get("expected_method_keys") != len(live):
        errors.append(
            f"expected_method_keys is {manifest.get('expected_method_keys')!r}, live inventory has {len(live)}"
        )

    rows = manifest.get("method", [])
    keys = [str(row.get("key", "")) for row in rows]
    if len(keys) != len(set(keys)):
        errors.append("duplicate variant method key")
    missing = sorted(set(live) - set(keys))
    stale = sorted(set(keys) - set(live))
    if missing:
        errors.append(f"variant ledger missing method keys: {missing}")
    if stale:
        errors.append(f"variant ledger has stale method keys: {stale}")

    for row in rows:
        key = str(row.get("key", ""))
        classification = row.get("classification")
        if classification not in CLASSIFICATIONS:
            errors.append(f"{key}: invalid variant classification {classification!r}")
        reason = row.get("reason")
        if classification != "common" and (not isinstance(reason, str) or not reason.strip()):
            errors.append(f"{key}: non-common variant needs a reason")
        actual = grouped_observations(live.get(key, {}))
        reviewed = normalized_manifest_observations(row.get("observation", []))
        if reviewed != actual:
            errors.append(
                f"{key}: variant observations changed; reviewed={reviewed}, live={actual}"
            )
        if classification == "common":
            common = (
                len(actual) == 1
                and actual[0].get("present") is True
                and actual[0].get("builds") == sorted(builds)
            )
            if not common:
                errors.append(f"{key}: classification common hides a real build variant")
    return errors


def inventory_json(live: dict[str, dict[str, tuple[str, str, str] | None]]) -> str:
    value = [
        {"key": key, "observation": grouped_observations(observations)}
        for key, observations in sorted(live.items())
    ]
    return json.dumps(value, indent=2, sort_keys=True) + "\n"


def synthetic_fixture() -> tuple[dict, dict[str, dict[str, tuple[str, str, str] | None]]]:
    live = {
        "0": {
            "nokia": ("a", "()V", "a" * 64),
            "sony": ("a", "()V", "a" * 64),
        },
        "1": {
            "nokia": ("b", "(I)V", "b" * 64),
            "sony": ("c", "(I)V", "c" * 64),
        },
    }
    manifest = {
        "schema_version": 1,
        "owner": "g",
        "identity": "ordinal",
        "builds": ["nokia", "sony"],
        "expected_build_count": 2,
        "expected_method_keys": 2,
        "method": [
            {
                "key": "0",
                "classification": "common",
                "observation": grouped_observations(live["0"]),
            },
            {
                "key": "1",
                "classification": "device-policy",
                "reason": "device-specific implementation",
                "observation": grouped_observations(live["1"]),
            },
        ],
    }
    return manifest, live


def self_test() -> int:
    manifest, live = synthetic_fixture()
    if validate(manifest, live):
        raise VariantError("clean synthetic variant fixture is not green")
    changed = copy.deepcopy(live)
    changed["1"]["sony"] = ("c", "(I)V", "d" * 64)
    errors = validate(manifest, changed)
    if not any("variant observations changed" in error for error in errors):
        raise VariantError(f"one changed method shape did not turn red: {errors}")
    print("variant-ledger self-test OK: one changed method shape goes red")
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", nargs="?", type=Path)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--inventory", action="store_true")
    arguments = parser.parse_args(argv)
    try:
        if arguments.self_test:
            return self_test()
        if arguments.manifest is None:
            parser.error("MANIFEST is required unless --self-test is used")
        manifest = tomllib.loads(arguments.manifest.read_text(encoding="utf-8"))
        identity = manifest.get("identity")
        if identity not in {"ordinal", "signature"}:
            raise VariantError("identity must be 'ordinal' or 'signature'")
        live = collect_live(manifest.get("owner", ""), manifest.get("builds", []), identity)
        if arguments.inventory:
            print(inventory_json(live), end="")
            return 0
        errors = validate(manifest, live)
        if errors:
            print("\n".join(errors), file=sys.stderr)
            return 1
        print(
            f"variant ledger OK: {len(live)} method keys across "
            f"{len(manifest.get('builds', []))} builds ({identity})"
        )
        return 0
    except (OSError, VariantError, corpus.CorpusError) as error:
        print(f"variant ledger FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
