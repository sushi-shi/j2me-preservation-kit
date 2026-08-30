#!/usr/bin/env python3
"""Validate whole-port ownership and semantic call-path closure.

The per-node crosswalk proves admitted bodies. This gate supplies the global
denominators around it:

* every selected baseline Java field has one reviewed Rust-state decision;
* every production Rust declaration is Java-owned or a reasoned adaptation;
* every selected original bytecode call edge has a reviewed Rust realization.

The decisions remain game-owned TOML. This reusable coordinator recomputes the
Java inventories from the selected original JAR and the Rust inventory from the
shipped ``j2me-ast-audit`` emitter on every run.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Sequence

ROOT = Path(__file__).resolve().parents[2]
AST_TOOLS = ROOT / "tools" / "ast"
CORPUS_TOOLS = ROOT / "tools" / "corpus"
sys.path.insert(0, str(AST_TOOLS))
sys.path.insert(0, str(CORPUS_TOOLS))

import classfile  # noqa: E402
import corpus  # noqa: E402
import scope_crosswalk  # noqa: E402

SHA256 = re.compile(r"^[0-9a-f]{64}$")
FIELD_CLASSIFICATIONS = {
    "state",
    "constant",
    "aggregate",
    "derived",
    "host",
    "erased",
}
RUST_CLASSIFICATIONS = {
    "java-body",
    "java-field",
    "host-adapter",
    "runtime-adapter",
    "representation-adapter",
    "generated",
    "oracle-infrastructure",
}
RUST_ONLY_CLASSIFICATIONS = RUST_CLASSIFICATIONS - {"java-body", "java-field"}
EDGE_CLASSIFICATIONS = {"path", "runtime", "dispatch-adapter", "erased", "dead"}


class CompletenessError(RuntimeError):
    pass


@dataclass(frozen=True)
class RustEvidence:
    ast: str
    ast_sha256: str


def digest(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def field_key(owner: str, name: str, descriptor: str) -> str:
    return f"{owner}.{name}:{descriptor}"


def method_key(owner: str, name: str, descriptor: str) -> str:
    return f"{owner}.{name}:{descriptor}"


def display_path(value: str | Path) -> str:
    path = Path(value)
    if not path.is_absolute():
        path = ROOT / path
    path = path.resolve()
    try:
        return path.relative_to(ROOT).as_posix()
    except ValueError:
        return str(path)


def rust_key(row: dict) -> tuple[str, str]:
    return display_path(row.get("file", "")), str(row.get("item", ""))


def baseline_classes() -> dict[str, classfile.ClassInfo]:
    manifest = corpus.load_manifest()
    baseline = manifest.get("baseline")
    if not baseline:
        raise CompletenessError("builds.toml has no selected baseline")
    try:
        payload = next(build.payload for build in corpus.builds() if build.build_id == baseline)
    except StopIteration as error:
        raise CompletenessError(f"selected baseline {baseline!r} is absent") from error
    result = {}
    for member, data in corpus.jar_members(payload):
        if member.endswith(".class"):
            info = classfile.parse_class(member, data)
            result[info.internal_name] = info
    return result


def call_owner(signature: str) -> str | None:
    try:
        owner_and_name, _descriptor = signature.split(":", 1)
        owner, _name = owner_and_name.rsplit(".", 1)
        return owner
    except ValueError:
        return None


def java_inventory(
    classes: dict[str, classfile.ClassInfo],
    owners: Sequence[str],
    *,
    call_scope: str = "game",
    platform_prefixes: Sequence[str] = (),
) -> tuple[set[str], set[str], set[tuple[str, str]]]:
    missing = sorted(set(owners) - set(classes))
    if missing:
        raise CompletenessError(f"configured baseline classes are absent: {missing}")
    owner_set = set(owners)
    fields: set[str] = set()
    methods: set[str] = set()
    edges: set[tuple[str, str]] = set()
    if call_scope not in {"game", "game-and-platform"}:
        raise CompletenessError(f"unknown call_scope {call_scope!r}")

    for owner in owners:
        info = classes[owner]
        fields.update(field_key(owner, field.name, field.descriptor) for field in info.fields)
        for method in info.methods:
            caller = method_key(owner, method.name, method.descriptor)
            methods.add(caller)
            for callee in method.calls:
                callee_owner = call_owner(callee)
                selected = callee_owner in owner_set
                if call_scope == "game-and-platform":
                    selected = selected or (
                        callee_owner is not None
                        and any(callee_owner.startswith(prefix) for prefix in platform_prefixes)
                    )
                if selected:
                    edges.add((caller, callee))
    return fields, methods, edges


def discover_rust_sources(roots: Sequence[str], excludes: Sequence[str]) -> list[str]:
    excluded = tuple(excludes)
    sources: set[Path] = set()
    for root_value in roots:
        root = Path(root_value)
        if not root.is_absolute():
            root = ROOT / root
        root = root.resolve()
        if not root.exists():
            raise CompletenessError(f"Rust inventory root is absent: {root}")
        candidates: Iterable[Path] = [root] if root.is_file() else root.rglob("*.rs")
        for candidate in candidates:
            if not candidate.is_file():
                continue
            relative = display_path(candidate)
            if any(Path(relative).match(pattern) for pattern in excluded):
                continue
            sources.add(candidate.resolve())
    if not sources:
        raise CompletenessError("Rust inventory roots contain no production sources")
    return [str(path) for path in sorted(sources)]


def live_rust_inventory(roots: Sequence[str], excludes: Sequence[str]) -> dict[tuple[str, str], RustEvidence]:
    items = scope_crosswalk.run_rust_emitter(discover_rust_sources(roots, excludes))
    result: dict[tuple[str, str], RustEvidence] = {}
    for item in items:
        key = (display_path(item.source), item.item)
        if key in result:
            raise CompletenessError(f"duplicate emitted Rust declaration {key}")
        result[key] = RustEvidence(item.ast, digest(item.ast))
    return result


def _duplicates(values: Sequence[object]) -> set[object]:
    seen: set[object] = set()
    duplicates: set[object] = set()
    for value in values:
        if value in seen:
            duplicates.add(value)
        seen.add(value)
    return duplicates


def _targets(row: dict, key: str = "rust") -> list[dict]:
    value = row.get(key, [])
    if isinstance(value, dict):
        return [value]
    return list(value) if isinstance(value, list) else []


def _inventory_difference(label: str, expected: set, reviewed: set, errors: list[str]) -> None:
    missing = sorted(expected - reviewed)
    stale = sorted(reviewed - expected)
    if missing:
        errors.append(f"{label} missing reviewed rows: {missing}")
    if stale:
        errors.append(f"{label} has stale/unknown rows: {stale}")


def validate(
    manifest: dict,
    java_fields: set[str],
    java_methods: set[str],
    java_edges: set[tuple[str, str]],
    rust_items: dict[tuple[str, str], RustEvidence],
) -> list[str]:
    errors: list[str] = []
    if manifest.get("schema_version") != 1:
        errors.append("unsupported completeness schema_version")

    ratchet = manifest.get("ratchet", {})
    actual_counts = {
        "java_fields": len(java_fields),
        "java_methods": len(java_methods),
        "java_call_edges": len(java_edges),
        "rust_declarations": len(rust_items),
    }
    for name, actual in actual_counts.items():
        if ratchet.get(name) != actual:
            errors.append(f"{name} ratchet is {ratchet.get(name)!r}, live inventory is {actual}")

    # Every original field has exactly one game-owned decision.
    field_rows = manifest.get("field_ownership", [])
    reviewed_fields = [str(row.get("java", "")) for row in field_rows]
    duplicates = _duplicates(reviewed_fields)
    if duplicates:
        errors.append(f"duplicate Java field ownership rows: {sorted(duplicates)}")
    _inventory_difference("Java field inventory", java_fields, set(reviewed_fields), errors)

    field_targets: dict[tuple[str, str], str] = {}
    field_rows_by_java = {str(row.get("java", "")): row for row in field_rows}
    for row in field_rows:
        java = str(row.get("java", ""))
        classification = row.get("classification")
        reason = row.get("reason")
        if classification not in FIELD_CLASSIFICATIONS:
            errors.append(f"{java}: invalid field classification {classification!r}")
        if not isinstance(reason, str) or not reason.strip():
            errors.append(f"{java}: field ownership decision needs a reason")
        targets = _targets(row)
        if classification in {"state", "constant", "aggregate"} and not targets:
            errors.append(f"{java}: {classification} field needs a Rust target")
        if classification in {"derived", "host", "erased"} and targets:
            errors.append(f"{java}: {classification} field must not claim persistent Rust state")
        for target in targets:
            key = rust_key(target)
            if key in field_targets:
                errors.append(
                    f"Rust state target {key} has two Java owners: {field_targets[key]} and {java}"
                )
            field_targets[key] = java
            evidence = rust_items.get(key)
            if evidence is None:
                errors.append(f"{java}: missing Rust state target {key}")
            type_marker = target.get("type_contains")
            if classification in {"state", "constant", "aggregate"}:
                if not isinstance(type_marker, str) or not type_marker.strip():
                    errors.append(f"{java}: Rust state target {key} needs type_contains")
                elif evidence is not None and type_marker not in evidence.ast:
                    errors.append(
                        f"{java}: Rust state target {key} lacks type marker {type_marker!r}"
                    )
            if classification in {"state", "aggregate"} and not key[1].startswith("field:"):
                errors.append(f"{java}: state target is not a Rust field: {key}")
            if classification == "constant" and not key[1].startswith(("const:", "static:")):
                errors.append(f"{java}: constant target is not a Rust const/static: {key}")

    # Reverse denominator: every emitted production declaration is classified.
    rust_rows = manifest.get("rust_declaration", [])
    rust_row_keys = [rust_key(row) for row in rust_rows]
    duplicates = _duplicates(rust_row_keys)
    if duplicates:
        errors.append(f"duplicate Rust declaration rows: {sorted(duplicates)}")
    _inventory_difference("production Rust inventory", set(rust_items), set(rust_row_keys), errors)
    rust_rows_by_key = {rust_key(row): row for row in rust_rows}
    for row in rust_rows:
        key = rust_key(row)
        classification = row.get("classification")
        owner = row.get("owner")
        reason = row.get("reason")
        if classification not in RUST_CLASSIFICATIONS:
            errors.append(f"{key}: invalid Rust classification {classification!r}")
        evidence = rust_items.get(key)
        locked = row.get("ast_sha256")
        if not isinstance(locked, str) or not SHA256.fullmatch(locked):
            errors.append(f"{key}: Rust declaration needs a lowercase AST SHA-256")
        elif evidence is not None and locked != evidence.ast_sha256:
            errors.append(f"{key}: Rust declaration AST changed")
        if classification == "java-body" and owner not in java_methods:
            errors.append(f"{key}: unknown Java method owner {owner!r}")
        elif classification == "java-field" and owner not in java_fields:
            errors.append(f"{key}: unknown Java field owner {owner!r}")
        elif classification in RUST_ONLY_CLASSIFICATIONS:
            if owner:
                errors.append(f"{key}: Rust-only adaptation must not claim Java owner {owner!r}")
            if not isinstance(reason, str) or not reason.strip():
                errors.append(f"{key}: Rust-only adaptation needs a reason")

    for target, java in field_targets.items():
        declaration = rust_rows_by_key.get(target)
        if declaration is None:
            continue
        if declaration.get("classification") != "java-field" or declaration.get("owner") != java:
            errors.append(
                f"{target}: reverse Rust ownership disagrees with Java field {java}"
            )

    # Every selected direct bytecode edge is manifested exactly once.
    edge_rows = manifest.get("call_edge", [])
    reviewed_edges = [
        (str(row.get("caller", "")), str(row.get("callee", ""))) for row in edge_rows
    ]
    duplicates = _duplicates(reviewed_edges)
    if duplicates:
        errors.append(f"duplicate Java call-edge rows: {sorted(duplicates)}")
    _inventory_difference("Java call-edge inventory", java_edges, set(reviewed_edges), errors)
    for row in edge_rows:
        edge = (str(row.get("caller", "")), str(row.get("callee", "")))
        classification = row.get("classification")
        reason = row.get("reason")
        if classification not in EDGE_CLASSIFICATIONS:
            errors.append(f"{edge}: invalid call-edge classification {classification!r}")
        if not isinstance(reason, str) or not reason.strip():
            errors.append(f"{edge}: call-edge decision needs a reason")
        path = _targets(row, "rust_path")
        if classification in {"path", "dispatch-adapter"} and not path:
            errors.append(f"{edge}: {classification} edge needs a Rust path")
        if classification in {"runtime", "erased", "dead"} and path:
            errors.append(f"{edge}: {classification} edge must not claim a Rust source path")
        for hop in path:
            key = rust_key(hop)
            evidence = rust_items.get(key)
            if evidence is None:
                errors.append(f"{edge}: missing Rust path hop {key}")
                continue
            if key not in rust_rows_by_key:
                errors.append(f"{edge}: Rust path hop lacks reverse ownership: {key}")
            markers = hop.get("contains", [])
            if isinstance(markers, str):
                markers = [markers]
            if not markers:
                errors.append(f"{edge}: Rust path hop {key} needs at least one AST marker")
            for marker in markers:
                if not isinstance(marker, str) or marker not in evidence.ast:
                    errors.append(f"{edge}: Rust path hop {key} lacks AST marker {marker!r}")

    # A field row that went stale should not accidentally satisfy reverse checks.
    for java, row in field_rows_by_java.items():
        if java not in java_fields and _targets(row):
            errors.append(f"{java}: stale field row still owns Rust state")
    return errors


def load_live(manifest: dict) -> tuple[set[str], set[str], set[tuple[str, str]], dict[tuple[str, str], RustEvidence]]:
    game = tomllib.loads((ROOT / "game.toml").read_text(encoding="utf-8"))
    inventory = manifest.get("inventory", {})
    owners = inventory.get("java_classes") or game.get("java", {}).get("baseline_classes", [])
    if not owners:
        raise CompletenessError("no baseline Java classes configured")
    call_scope = inventory.get("call_scope", "game")
    prefixes = inventory.get(
        "platform_prefixes",
        ["java/", "javax/", "com/nokia/", "com/siemens/", "com/motorola/"],
    )
    java_fields, java_methods, java_edges = java_inventory(
        baseline_classes(), owners, call_scope=call_scope, platform_prefixes=prefixes
    )
    roots = inventory.get("rust_roots", [])
    if not roots:
        raise CompletenessError("inventory.rust_roots is empty")
    rust_items = live_rust_inventory(roots, inventory.get("rust_exclude", []))
    return java_fields, java_methods, java_edges, rust_items


def synthetic_fixture() -> tuple[dict, set[str], set[str], set[tuple[str, str]], dict[tuple[str, str], RustEvidence]]:
    field = "a.x:I"
    caller = "a.tick:()V"
    callee = "a.draw:()V"
    state_key = ("port.rs", "field:GameState::x")
    tick_key = ("port.rs", "fn:tick")
    state_ast = "x : i32"
    tick_ast = "fn tick () { draw () ; }"
    rust = {
        state_key: RustEvidence(state_ast, digest(state_ast)),
        tick_key: RustEvidence(tick_ast, digest(tick_ast)),
    }
    manifest = {
        "schema_version": 1,
        "ratchet": {
            "java_fields": 1,
            "java_methods": 2,
            "java_call_edges": 1,
            "rust_declarations": 2,
        },
        "field_ownership": [
            {
                "java": field,
                "classification": "state",
                "reason": "persistent counter",
                "rust": [
                    {"file": state_key[0], "item": state_key[1], "type_contains": "i32"}
                ],
            }
        ],
        "rust_declaration": [
            {
                "file": state_key[0],
                "item": state_key[1],
                "ast_sha256": rust[state_key].ast_sha256,
                "classification": "java-field",
                "owner": field,
            },
            {
                "file": tick_key[0],
                "item": tick_key[1],
                "ast_sha256": rust[tick_key].ast_sha256,
                "classification": "java-body",
                "owner": caller,
            },
        ],
        "call_edge": [
            {
                "caller": caller,
                "callee": callee,
                "classification": "path",
                "reason": "direct translated call",
                "rust_path": [
                    {"file": tick_key[0], "item": tick_key[1], "contains": ["draw"]}
                ],
            }
        ],
    }
    return manifest, {field}, {caller, callee}, {(caller, callee)}, rust


def self_test() -> int:
    manifest, fields, methods, edges, rust = synthetic_fixture()
    if validate(manifest, fields, methods, edges, rust):
        raise CompletenessError("clean synthetic completeness fixture is not green")

    cases = []
    missing_field = copy.deepcopy(manifest)
    missing_field["field_ownership"] = []
    cases.append(("dropped Java field", missing_field, "missing reviewed rows"))

    orphan_rust = copy.deepcopy(manifest)
    orphan_rust["rust_declaration"] = orphan_rust["rust_declaration"][:-1]
    cases.append(("orphan production Rust declaration", orphan_rust, "production Rust inventory"))

    missing_edge = copy.deepcopy(manifest)
    missing_edge["call_edge"] = []
    cases.append(("dropped Java call edge", missing_edge, "call-edge inventory"))

    bad_marker = copy.deepcopy(manifest)
    bad_marker["call_edge"][0]["rust_path"][0]["contains"] = ["wrong_target"]
    cases.append(("wrong Rust path marker", bad_marker, "lacks AST marker"))

    for label, broken, needle in cases:
        errors = validate(broken, fields, methods, edges, rust)
        if not any(needle in error for error in errors):
            raise CompletenessError(f"{label} did not turn the gate red: {errors}")
    print(f"port completeness self-test OK: {len(cases)} injected omissions/drifts go red")
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", nargs="?", type=Path)
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args(argv)
    try:
        if arguments.self_test:
            return self_test()
        if arguments.manifest is None:
            parser.error("MANIFEST is required unless --self-test is used")
        manifest = tomllib.loads(arguments.manifest.read_text(encoding="utf-8"))
        fields, methods, edges, rust = load_live(manifest)
        errors = validate(manifest, fields, methods, edges, rust)
        if errors:
            print("\n".join(errors), file=sys.stderr)
            return 1
        print(
            "port completeness OK: "
            f"{len(fields)} Java fields, {len(edges)} Java call edges, "
            f"{len(rust)} production Rust declarations"
        )
        return 0
    except (CompletenessError, OSError, corpus.CorpusError, scope_crosswalk.ScopeError) as error:
        print(f"port completeness FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
