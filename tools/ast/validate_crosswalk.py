#!/usr/bin/env python3
"""Validate exhaustive Java/Rust AST node ownership in a crosswalk manifest."""

from __future__ import annotations

import argparse
import copy
import re
import tomllib
from pathlib import Path
from typing import Any


HASH = re.compile(r"[0-9a-f]{64}")


def _claim(
    counts: list[int],
    index: int,
    context: str,
    errors: list[str],
) -> None:
    if index < 0 or index >= len(counts):
        errors.append(f"{context}: node {index} is outside 0..{len(counts) - 1}")
    else:
        counts[index] += 1


def _claim_java(
    mapping: dict[str, Any],
    counts: list[int],
    context: str,
    errors: list[str],
) -> None:
    for index in mapping.get("java_nodes", []):
        _claim(counts, index, context, errors)
    for bounds in mapping.get("java_node_ranges", []):
        if len(bounds) != 2 or bounds[0] > bounds[1]:
            errors.append(f"{context}: invalid Java node range {bounds!r}")
            continue
        for index in range(bounds[0], bounds[1] + 1):
            _claim(counts, index, context, errors)


def _claim_rust(
    mapping: dict[str, Any],
    counts: list[list[int]],
    context: str,
    errors: list[str],
) -> None:
    for encoded in mapping.get("rust_nodes", []):
        try:
            target_text, index_text = encoded.split(":", 1)
            target = int(target_text)
            index = int(index_text)
        except (AttributeError, TypeError, ValueError):
            errors.append(f"{context}: invalid Rust node {encoded!r}")
            continue
        if target < 0 or target >= len(counts):
            errors.append(f"{context}: invalid Rust target {target!r}")
            continue
        _claim(counts[target], index, context, errors)
    for claim in mapping.get("rust_node_ranges", []):
        target = claim.get("target")
        start = claim.get("start")
        end = claim.get("end")
        if not isinstance(target, int) or target < 0 or target >= len(counts):
            errors.append(f"{context}: invalid Rust target {target!r}")
            continue
        if not isinstance(start, int) or not isinstance(end, int) or start > end:
            errors.append(f"{context}: invalid Rust node range {claim!r}")
            continue
        for index in range(start, end + 1):
            _claim(counts[target], index, context, errors)


def _hash(value: object, context: str, errors: list[str]) -> None:
    if not isinstance(value, str) or HASH.fullmatch(value) is None:
        errors.append(f"{context}: missing or invalid SHA-256")


def _validate_row(kind: str, row: dict[str, Any], ordinal: int) -> list[str]:
    label = row.get("java_item") or row.get("java_name") or f"row-{ordinal}"
    context = f"{kind} {label}"
    errors: list[str] = []
    java_count = row.get("java_node_count")
    rust_counts = row.get("rust_node_counts")
    if not isinstance(java_count, int) or java_count < 0:
        return [f"{context}: invalid java_node_count"]
    if not isinstance(rust_counts, list) or not all(
        isinstance(count, int) and count >= 0 for count in rust_counts
    ):
        return [f"{context}: invalid rust_node_counts"]

    _hash(row.get("java_ast_sha256"), f"{context} Java AST", errors)
    _hash(row.get("java_nodes_sha256"), f"{context} Java nodes", errors)
    rust = row.get("rust", [])
    if len(rust) != len(rust_counts):
        errors.append(
            f"{context}: {len(rust_counts)} Rust node counts but {len(rust)} AST rows"
        )
    for target, rust_row in enumerate(rust):
        _hash(rust_row.get("ast_sha256"), f"{context} Rust AST {target}", errors)
        _hash(rust_row.get("nodes_sha256"), f"{context} Rust nodes {target}", errors)
        if target < len(rust_counts) and rust_row.get("node_count") != rust_counts[target]:
            errors.append(f"{context}: Rust node-count disagreement at target {target}")

    java_claims = [0] * java_count
    rust_claims = [[0] * count for count in rust_counts]
    mappings = [*row.get("operation", []), *row.get("adaptation", [])]
    if not mappings and (java_count or any(rust_counts)):
        errors.append(f"{context}: no semantic mappings")
    for mapping_index, mapping in enumerate(mappings):
        mapping_context = f"{context} mapping {mapping_index}"
        if not mapping.get("semantic") and not mapping.get("reason"):
            errors.append(f"{mapping_context}: missing semantic/reason text")
        _claim_java(mapping, java_claims, mapping_context, errors)
        _claim_rust(mapping, rust_claims, mapping_context, errors)

    for index, owners in enumerate(java_claims):
        if owners != 1:
            errors.append(f"{context}: Java node {index} has {owners} owners")
    for target, claims in enumerate(rust_claims):
        for index, owners in enumerate(claims):
            if owners != 1:
                errors.append(
                    f"{context}: Rust target {target} node {index} has {owners} owners"
                )
    if row.get("semantic_status") != "crosswalked":
        errors.append(f"{context}: semantic_status must be crosswalked")
    if not row.get("semantic_review"):
        errors.append(f"{context}: missing semantic_review")
    return errors


def validate_manifest(data: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if data.get("schema_version") != 1:
        errors.append("schema_version must be 1")
    bodies = data.get("body", [])
    fields = data.get("field", [])
    if data.get("reviewed_body_count") != len(bodies):
        errors.append("reviewed_body_count does not equal the number of body rows")
    if data.get("semantic_reviewed_body_count") != len(bodies):
        errors.append("semantic_reviewed_body_count does not equal body rows")
    if data.get("semantic_reviewed_field_count") != len(fields):
        errors.append("semantic_reviewed_field_count does not equal field rows")
    total_bodies = data.get("total_body_count")
    total_fields = data.get("total_field_count")
    if not isinstance(total_bodies, int) or total_bodies < len(bodies):
        errors.append("total_body_count is below reviewed coverage")
    if not isinstance(total_fields, int) or total_fields < len(fields):
        errors.append("total_field_count is below reviewed coverage")
    for ordinal, body in enumerate(bodies):
        _hash(body.get("code_sha256"), f"body {ordinal} bytecode", errors)
        _hash(body.get("opcode_sha256"), f"body {ordinal} opcodes", errors)
        errors.extend(_validate_row("body", body, ordinal))
    for ordinal, field in enumerate(fields):
        errors.extend(_validate_row("field", field, ordinal))
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args()
    data = tomllib.loads(arguments.manifest.read_text(encoding="utf-8"))
    errors = validate_manifest(data)
    if errors:
        for error in errors[:50]:
            print(error)
        return 1
    if arguments.self_test:
        mutated = copy.deepcopy(data)
        rows = [*mutated.get("body", []), *mutated.get("field", [])]
        if not rows:
            parser.error("self-test requires at least one crosswalk row")
        rows[0].setdefault("adaptation", []).append(
            {"reason": "injected duplicate", "java_nodes": [0]}
        )
        injected = validate_manifest(mutated)
        if not any("Java node 0 has 2 owners" in error for error in injected):
            parser.error("self-test failed to reject an overlapping Java node owner")
        print("crosswalk self-test OK: overlapping Java ownership was rejected")
        return 0
    print(
        "crosswalk OK: "
        f"{len(data.get('body', []))}/{data['total_body_count']} bodies and "
        f"{len(data.get('field', []))}/{data['total_field_count']} fields"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
