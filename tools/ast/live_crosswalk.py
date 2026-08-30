"""Reusable live-evidence coordinator for per-game schema-2 crosswalk gates."""

from __future__ import annotations

import copy
import json
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence

ROOT = Path(__file__).resolve().parents[2]
AST_TOOLS = ROOT / "tools" / "ast"
CORPUS_TOOLS = ROOT / "tools" / "corpus"
sys.path.insert(0, str(AST_TOOLS))
sys.path.insert(0, str(CORPUS_TOOLS))

import classfile  # noqa: E402
import corpus  # noqa: E402
import scope_crosswalk  # noqa: E402
import validate_crosswalk  # noqa: E402


@dataclass(frozen=True)
class RustTarget:
    file: str
    item: str


@dataclass(frozen=True)
class BodySpec:
    key: str
    original_owner: str
    original_name: str
    descriptor: str
    java_source: str
    java_owner: str
    java_item: str
    rust: tuple[RustTarget, ...]


class LiveCrosswalkError(RuntimeError):
    pass


def load_toml(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def baseline_classes() -> tuple[str, dict[str, classfile.ClassInfo]]:
    corpus_manifest = corpus.load_manifest()
    baseline = corpus_manifest["baseline"]
    payload = next(build.payload for build in corpus.builds() if build.build_id == baseline)
    classes = {}
    for member, data in corpus.jar_members(payload):
        if member.endswith(".class"):
            info = classfile.parse_class(member, data)
            classes[info.internal_name] = info
    return baseline, classes


def require_spec_keys(manifest: dict, specs: Sequence[BodySpec]) -> None:
    manifest_keys = [body.get("java_item") for body in manifest.get("body", [])]
    spec_keys = [spec.key for spec in specs]
    if manifest_keys != spec_keys:
        raise LiveCrosswalkError(
            "manifest rows must exactly match the ordered reviewed-body specification"
        )


def live_evidence(manifest_path: Path, specs: Sequence[BodySpec]) -> tuple[dict, dict]:
    manifest = load_toml(manifest_path)
    baseline, classes = baseline_classes()
    if manifest.get("build") != baseline:
        raise LiveCrosswalkError(
            f"crosswalk build {manifest.get('build')!r} differs from baseline {baseline!r}"
        )

    game = load_toml(ROOT / "game.toml")
    owners = game.get("java", {}).get("baseline_classes", [])
    total_bodies = sum(len(classes[owner].methods) for owner in owners)
    if manifest.get("total_body_count") != total_bodies:
        raise LiveCrosswalkError(
            f"total_body_count is {manifest.get('total_body_count')}, "
            f"bytecode has {total_bodies}"
        )
    require_spec_keys(manifest, specs)

    java_sources = list(dict.fromkeys(str(ROOT / spec.java_source) for spec in specs))
    rust_sources = list(
        dict.fromkeys(str(ROOT / target.file) for spec in specs for target in spec.rust)
    )
    java_items = scope_crosswalk.run_java_emitter(java_sources)
    rust_items = scope_crosswalk.run_rust_emitter(rust_sources)
    evidence = {"body": []}

    for spec in specs:
        original = classes.get(spec.original_owner)
        if original is None:
            raise LiveCrosswalkError(f"original class {spec.original_owner!r} is absent")
        methods = [
            method
            for method in original.methods
            if method.name == spec.original_name and method.descriptor == spec.descriptor
        ]
        if len(methods) != 1:
            raise LiveCrosswalkError(
                f"original {spec.original_owner}.{spec.original_name}{spec.descriptor} "
                f"matched {len(methods)} methods"
            )
        method = methods[0]
        if method.code_sha256 is None or method.opcode_sha256 is None:
            raise LiveCrosswalkError(f"original body {spec.key} has no Code attribute")

        java = scope_crosswalk.select_java(
            java_items, spec.java_owner, spec.java_item
        )
        rust = [
            scope_crosswalk.select_rust(
                rust_items, str(ROOT / target.file), target.item
            )
            for target in spec.rust
        ]
        scope = scope_crosswalk.build_scope(
            java=java,
            rust=rust,
            body_key=spec.key,
            code_sha256=method.code_sha256,
            opcode_sha256=method.opcode_sha256,
        )
        evidence["body"].append(scope["evidence"])
    return manifest, evidence


def checked_report(manifest: dict, evidence: dict) -> validate_crosswalk.Report:
    return validate_crosswalk.validate(
        manifest, validate_crosswalk.load_evidence(evidence), strict=True
    )


def inventory_json(evidence: dict) -> str:
    """Return deterministic, machine-readable live AST evidence for review."""

    review = copy.deepcopy(evidence)
    for body in review.get("body", []):
        java_nodes = body.get("java_nodes", [])
        body["java_node_count"] = len(java_nodes)
        body["java_nodes_sha256"] = validate_crosswalk.node_inventory_digest(java_nodes)
        body["java_nodes"] = [
            {"index": index, "node": node} for index, node in enumerate(java_nodes)
        ]
        for target in body.get("rust", []):
            rust_nodes = target.get("nodes", [])
            target["node_count"] = len(rust_nodes)
            target["nodes_sha256"] = validate_crosswalk.node_inventory_digest(rust_nodes)
            target["nodes"] = [
                {"index": index, "node": node} for index, node in enumerate(rust_nodes)
            ]
    return json.dumps(review, indent=2, sort_keys=True) + "\n"


def run_gate(
    *,
    label: str,
    manifest_path: Path,
    specs: Sequence[BodySpec],
    argv: list[str],
) -> int:
    try:
        manifest, evidence = live_evidence(manifest_path, specs)
        if argv == ["--inventory"]:
            print(inventory_json(evidence), end="")
            return 0
        report = checked_report(manifest, evidence)
        if report.errors:
            raise LiveCrosswalkError("\n".join(report.errors))

        if argv == ["--self-test"]:
            dirty = copy.deepcopy(evidence)
            dirty["body"][0]["rust"][0]["nodes"][0] += "_injected_drift"
            dirty_report = checked_report(manifest, dirty)
            if not any(
                "Rust node inventory changed" in error for error in dirty_report.errors
            ):
                raise LiveCrosswalkError(
                    "injected Rust-node drift did not break the inventory lock"
                )
            print(f"{label} crosswalk self-test OK: one changed Rust node goes red")
            return 0
        if argv:
            raise LiveCrosswalkError(f"unknown arguments: {' '.join(argv)}")

        print(validate_crosswalk.format_coverage(report))
        return 0
    except (
        KeyError,
        LiveCrosswalkError,
        OSError,
        StopIteration,
        scope_crosswalk.ScopeError,
    ) as error:
        print(f"{label} crosswalk FAIL: {error}", file=sys.stderr)
        return 1
