#!/usr/bin/env python3
"""Per-node Java/Rust AST crosswalk verifier (schema 2) for the J2ME port kit.

Why this exists
---------------
A crosswalk that pairs *whole bodies* under one blanket label proves almost
nothing. Gothic's coarse audit did exactly that: 365 of 429 bodies mapped the
entire Java body to the entire Rust body under a single "operation", and a real
bug survived it — `paint_radio_row`'s Rust computed ``sprite_meta[1717*11+1721]``
(a flat 2-D index via the ``sm`` accessor) where the bytecode is the *ratio*
``sprite_meta[1717] / sprite_meta[1721]`` (an ``idiv``). Under one blanket
operation nobody ever had to pair that single Rust call node against the Java
``array-index / array-index / DIVIDE`` nodes, so the divergence was invisible and
the body still read as "crosswalked".

The contract this gate enforces
-------------------------------
EVERY node on BOTH sides carries its own explicit decision and a human comment:

* an ``op`` is a *semantically atomic* paired step — it names the Java and Rust
  nodes that implement one operation and carries a ``semantic`` comment; or
* an ``adapt`` is a categorized one-sided node set (host-adapter,
  representation-adapter, erased, …) with a written ``reason``.

A body is only ``crosswalked`` when every Java node and every Rust node is
decided exactly once. Undecided nodes are counted and surfaced — that count is
"how much of this body is still unchecked", and it must reach zero.

The teeth (each proven can-fail by ``--self-test``)
---------------------------------------------------
1. Exhaustive per-node coverage — every node decided exactly once; a missing or
   duplicated owner is reported and (under ``--strict``) fatal.
2. Coarse-blanket rejection — a single ``op`` may not span more than
   ``blanket_max_span`` nodes on either side; a whole-body blanket is REJECTED.
3. Atomic operator-realization parity — inside a paired ``op``, a Java ``DIVIDE``
   / ``REMAINDER`` / shift must be realized on the Rust side (``a / b`` or the
   sanctioned ``java_div`` / ``java_rem`` / ``wrapping_shl`` helper). A Java
   division paired against a bare ``sm(..)`` call — the exact gothic bug — is red.
4. Hash locks — the authoritative bytecode Code-attribute digest, the full javac
   AST digest, each syn AST digest, and the pre-order node-inventory digests are
   all recorded and re-derived from live evidence; a one-node drift breaks a lock.

Evidence vs. manifest
---------------------
The *manifest* records the reviewer's locked decisions and digests. The
*evidence* is what the live tools currently emit — ``JavaAstAuditDump`` for the
canonical Java, ``j2me-ast-audit`` for the Rust, and a classfile reader for the
baseline bytecode. A per-game wrapper feeds live evidence in; this file's
``--evidence`` flag loads a captured evidence table so the generic gate and its
self-test are self-contained. Drift between the two is what turns a lock red.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

HEX64 = re.compile(r"[0-9a-f]{64}")

# One-sided adaptation categories. Java-only nodes are erased or faithful no-ops;
# Rust-only nodes are host/representation adapters or oracle scaffolding.
ADAPT_CATEGORIES = frozenset(
    {
        "erased",
        "faithful-noop",
        "host-adapter",
        "host-observation",
        "representation-adapter",
        "oracle-fixture",
        "oracle-infrastructure",
    }
)

# Java arithmetic node kinds whose Rust realization a paired op must exhibit,
# and the sanctioned Rust spellings (per docs/TRANSLITERATION.md) that realize
# them. A bare operator OR the named helper both count.
DEFAULT_MUST_REALIZE = {
    "DIVIDE": (
        r"BINARY\t/",
        ("java_div", "java_ldiv", "i32_div", "i64_div"),
    ),
    "REMAINDER": (
        r"BINARY\t%",
        ("java_rem", "java_lrem", "i32_rem", "i64_rem"),
    ),
    "LEFT_SHIFT": (
        r"BINARY\t<<",
        ("wrapping_shl", "i32_shl", "i64_shl", "ishl", "lshl"),
    ),
    "RIGHT_SHIFT": (
        r"BINARY\t>>",
        ("wrapping_shr", "i32_shr", "i64_shr", "ishr", "lshr"),
    ),
    "UNSIGNED_RIGHT_SHIFT": (
        r"BINARY\t>>",
        ("i32_ushr", "i64_ushr", "iushr", "lushr"),
    ),
}

DEFAULT_BLANKET_MAX_SPAN = 48


def sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def node_inventory_digest(nodes: list[str]) -> str:
    return sha("\n".join(nodes))


@dataclass(frozen=True)
class RustEvidence:
    file: str
    item: str
    ast_sha256: str
    nodes: tuple[str, ...]


@dataclass(frozen=True)
class BodyEvidence:
    java_item: str
    code_sha256: str
    opcode_sha256: str
    java_ast_sha256: str
    java_nodes: tuple[str, ...]
    rust: tuple[RustEvidence, ...]


@dataclass
class BodyCoverage:
    java_item: str
    java_total: int
    java_decided: int
    rust_total: int
    rust_decided: int
    crosswalked: bool

    @property
    def total(self) -> int:
        return self.java_total + self.rust_total

    @property
    def decided(self) -> int:
        return self.java_decided + self.rust_decided

    @property
    def undecided(self) -> int:
        return self.total - self.decided

    @property
    def complete(self) -> bool:
        return self.undecided == 0


@dataclass
class Report:
    errors: list[str] = field(default_factory=list)
    bodies: list[BodyCoverage] = field(default_factory=list)
    total_body_count: int = 0

    @property
    def node_total(self) -> int:
        return sum(body.total for body in self.bodies)

    @property
    def node_decided(self) -> int:
        return sum(body.decided for body in self.bodies)

    @property
    def node_undecided(self) -> int:
        return self.node_total - self.node_decided

    @property
    def partial_bodies(self) -> list[BodyCoverage]:
        return [body for body in self.bodies if not body.complete]

    @property
    def crosswalked_bodies(self) -> int:
        return sum(1 for body in self.bodies if body.crosswalked)


def load_evidence(data: dict[str, Any]) -> dict[str, BodyEvidence]:
    """Parse an evidence table (captured or wrapper-produced) into a map."""

    result: dict[str, BodyEvidence] = {}
    for body in data.get("body", []):
        rust = tuple(
            RustEvidence(
                file=str(target.get("file", "")),
                item=str(target.get("item", "")),
                ast_sha256=str(target.get("ast_sha256", "")),
                nodes=tuple(target.get("nodes", [])),
            )
            for target in body.get("rust", [])
        )
        item = str(body.get("java_item", ""))
        result[item] = BodyEvidence(
            java_item=item,
            code_sha256=str(body.get("code_sha256", "")),
            opcode_sha256=str(body.get("opcode_sha256", "")),
            java_ast_sha256=str(body.get("java_ast_sha256", "")),
            java_nodes=tuple(body.get("java_nodes", [])),
            rust=rust,
        )
    return result


def _java_refs(mapping: dict[str, Any]) -> list[int]:
    refs = list(mapping.get("java", []))
    for bounds in mapping.get("java_range", []):
        if isinstance(bounds, list) and len(bounds) == 2 and bounds[0] <= bounds[1]:
            refs.extend(range(bounds[0], bounds[1] + 1))
        else:
            refs.append(("bad-range", bounds))
    return refs


def _rust_refs(mapping: dict[str, Any]) -> list[Any]:
    refs = list(mapping.get("rust", []))
    for claim in mapping.get("rust_range", []):
        if (
            isinstance(claim, dict)
            and isinstance(claim.get("target"), int)
            and isinstance(claim.get("start"), int)
            and isinstance(claim.get("end"), int)
            and claim["start"] <= claim["end"]
        ):
            refs.extend(
                f"{claim['target']}:{index}"
                for index in range(claim["start"], claim["end"] + 1)
            )
        else:
            refs.append(("bad-range", claim))
    return refs


def _parse_rust_ref(value: Any, rust_counts: list[int]) -> tuple[int, int] | None:
    if not isinstance(value, str):
        return None
    try:
        target_text, index_text = value.split(":", 1)
        target, index = int(target_text), int(index_text)
    except (ValueError, AttributeError):
        return None
    if target < 0 or target >= len(rust_counts):
        return None
    if index < 0 or index >= rust_counts[target]:
        return None
    return target, index


def _realizes(kind: str, rust_texts: list[str], must_realize: dict) -> bool:
    operator_pattern, helpers = must_realize[kind]
    for text in rust_texts:
        if re.search(operator_pattern, text):
            return True
        if any(helper in text for helper in helpers):
            return True
    return False


def _validate_body(
    body: dict[str, Any],
    evidence: BodyEvidence | None,
    ordinal: int,
    blanket_max_span: int,
    must_realize: dict,
) -> tuple[list[str], BodyCoverage]:
    item = body.get("java_item") or f"body-{ordinal}"
    label = f"body {item}"
    errors: list[str] = []

    java_count = body.get("java_node_count")
    rust_targets = body.get("rust", [])
    rust_counts = [target.get("node_count") for target in rust_targets]
    if not isinstance(java_count, int) or java_count < 0:
        return [f"{label}: invalid java_node_count"], BodyCoverage(item, 0, 0, 0, 0, False)
    if not all(isinstance(count, int) and count >= 0 for count in rust_counts):
        return [f"{label}: invalid rust node_count"], BodyCoverage(
            item, java_count, 0, 0, 0, False
        )

    # --- Hash locks, re-derived from live evidence -------------------------
    for name in ("code_sha256", "opcode_sha256", "java_ast_sha256", "java_nodes_sha256"):
        value = body.get(name)
        if not isinstance(value, str) or HEX64.fullmatch(value) is None:
            errors.append(f"{label}: missing or malformed {name}")
    if evidence is None:
        errors.append(f"{label}: no live AST/bytecode evidence for this Java item")
    else:
        if evidence.code_sha256 != body.get("code_sha256"):
            errors.append(f"{label}: original bytecode Code-attr digest changed")
        if evidence.opcode_sha256 != body.get("opcode_sha256"):
            errors.append(f"{label}: original opcode digest changed")
        if evidence.java_ast_sha256 != body.get("java_ast_sha256"):
            errors.append(f"{label}: canonical Java AST digest changed")
        if node_inventory_digest(list(evidence.java_nodes)) != body.get(
            "java_nodes_sha256"
        ):
            errors.append(f"{label}: Java node inventory changed")
        if len(evidence.java_nodes) != java_count:
            errors.append(
                f"{label}: java_node_count says {java_count}, "
                f"evidence has {len(evidence.java_nodes)}"
            )
        if len(evidence.rust) != len(rust_targets):
            errors.append(
                f"{label}: {len(rust_targets)} Rust targets but evidence has "
                f"{len(evidence.rust)}"
            )
        for index, target in enumerate(rust_targets):
            for name in ("ast_sha256", "nodes_sha256"):
                value = target.get(name)
                if not isinstance(value, str) or HEX64.fullmatch(value) is None:
                    errors.append(f"{label}: Rust target {index} missing/malformed {name}")
            if index < len(evidence.rust):
                rust_ev = evidence.rust[index]
                if rust_ev.item != target.get("item"):
                    errors.append(
                        f"{label}: Rust target {index} is {target.get('item')!r} "
                        f"but evidence has {rust_ev.item!r}"
                    )
                if rust_ev.ast_sha256 != target.get("ast_sha256"):
                    errors.append(
                        f"{label}: reviewed Rust AST digest changed for {rust_ev.item}"
                    )
                if node_inventory_digest(list(rust_ev.nodes)) != target.get(
                    "nodes_sha256"
                ):
                    errors.append(
                        f"{label}: Rust node inventory changed for {rust_ev.item}"
                    )
                if len(rust_ev.nodes) != target.get("node_count"):
                    errors.append(
                        f"{label}: Rust target {index} node_count says "
                        f"{target.get('node_count')}, evidence has {len(rust_ev.nodes)}"
                    )

    # --- Per-node ownership -------------------------------------------------
    java_owners = [0] * java_count
    rust_owners = [[0] * count for count in rust_counts]
    java_texts = list(evidence.java_nodes) if evidence else []
    rust_texts = [list(target.nodes) for target in evidence.rust] if evidence else []

    def claim_java(index: Any, context: str) -> None:
        if not isinstance(index, int) or not 0 <= index < java_count:
            errors.append(f"{label}: {context} claims out-of-range Java node {index!r}")
            return
        java_owners[index] += 1

    def claim_rust(value: Any, context: str) -> tuple[int, int] | None:
        parsed = _parse_rust_ref(value, rust_counts)
        if parsed is None:
            errors.append(f"{label}: {context} claims invalid Rust node {value!r}")
            return None
        rust_owners[parsed[0]][parsed[1]] += 1
        return parsed

    operations = body.get("op", [])
    adaptations = body.get("adapt", [])
    if not operations and not adaptations and (java_count or any(rust_counts)):
        errors.append(f"{label}: body has nodes but no op/adapt decisions")

    for index, op in enumerate(operations):
        context = f"op {index} ({op.get('semantic', 'unnamed')!r})"
        if not op.get("semantic"):
            errors.append(f"{label}: {context} needs a semantic comment")
        java_refs = _java_refs(op)
        rust_refs = _rust_refs(op)
        if not java_refs or not rust_refs:
            errors.append(f"{label}: {context} must pair both Java and Rust nodes")
        # Coarse-blanket rejection: an atomic step is bounded in size.
        blanket_ok = bool(op.get("blanket_ok"))
        if len(java_refs) > blanket_max_span or len(rust_refs) > blanket_max_span:
            if blanket_ok and op.get("blanket_reason"):
                pass
            else:
                errors.append(
                    f"{label}: {context} is a coarse blanket — spans "
                    f"{len(java_refs)} Java / {len(rust_refs)} Rust nodes "
                    f"(> {blanket_max_span}); decompose into atomic steps"
                )
        op_java_kinds: list[str] = []
        for ref in java_refs:
            claim_java(ref, context)
            if isinstance(ref, int) and 0 <= ref < len(java_texts):
                op_java_kinds.append(java_texts[ref].split("\t", 1)[0])
        op_rust_texts: list[str] = []
        for ref in rust_refs:
            parsed = claim_rust(ref, context)
            if parsed is not None and parsed[0] < len(rust_texts):
                target_nodes = rust_texts[parsed[0]]
                if parsed[1] < len(target_nodes):
                    op_rust_texts.append(target_nodes[parsed[1]])
        # Atomic operator-realization parity (the paint_radio_row bug-catcher).
        if evidence is not None and not op.get("shape_note"):
            for kind in must_realize:
                needed = op_java_kinds.count(kind)
                if needed and not _realizes(kind, op_rust_texts, must_realize):
                    errors.append(
                        f"{label}: {context} pairs a Java {kind} with no Rust "
                        f"realization (expected {must_realize[kind][0]!r} or one of "
                        f"{must_realize[kind][1]}); Rust nodes are {op_rust_texts!r}"
                    )

    for index, adapt in enumerate(adaptations):
        context = f"adapt {index}"
        category = adapt.get("category")
        if category not in ADAPT_CATEGORIES:
            errors.append(
                f"{label}: {context} category must be one of "
                f"{sorted(ADAPT_CATEGORIES)}"
            )
        if not adapt.get("reason"):
            errors.append(f"{label}: {context} needs a reason")
        java_refs = _java_refs(adapt)
        rust_refs = _rust_refs(adapt)
        if java_refs and rust_refs:
            errors.append(f"{label}: {context} is one-sided but names both sides")
        if not java_refs and not rust_refs:
            errors.append(f"{label}: {context} names no nodes")
        for ref in java_refs:
            claim_java(ref, context)
        for ref in rust_refs:
            claim_rust(ref, context)

    for index, owners in enumerate(java_owners):
        if owners > 1:
            errors.append(f"{label}: Java node {index} has {owners} owners")
    for target, owners in enumerate(rust_owners):
        for index, count in enumerate(owners):
            if count > 1:
                errors.append(
                    f"{label}: Rust target {target} node {index} has {count} owners"
                )

    java_decided = sum(1 for owners in java_owners if owners >= 1)
    rust_decided = sum(1 for owners in rust_owners for count in owners if count >= 1)
    coverage = BodyCoverage(
        java_item=item,
        java_total=java_count,
        java_decided=java_decided,
        rust_total=sum(rust_counts),
        rust_decided=rust_decided,
        crosswalked=body.get("semantic_status") == "crosswalked",
    )

    if not body.get("review"):
        errors.append(f"{label}: missing human review prose")
    if coverage.crosswalked and not coverage.complete:
        errors.append(
            f"{label}: semantic_status=crosswalked but {coverage.undecided} node(s) "
            "are undecided"
        )
    return errors, coverage


def validate(
    manifest: dict[str, Any],
    evidence: dict[str, BodyEvidence],
    *,
    strict: bool = False,
) -> Report:
    report = Report()
    if manifest.get("schema_version") != 2:
        report.errors.append("schema_version must be 2 (per-node crosswalk)")

    policy = manifest.get("policy", {})
    blanket_max_span = policy.get("blanket_max_span", DEFAULT_BLANKET_MAX_SPAN)
    if not isinstance(blanket_max_span, int) or blanket_max_span < 1:
        report.errors.append("policy.blanket_max_span must be a positive integer")
        blanket_max_span = DEFAULT_BLANKET_MAX_SPAN
    must_realize = dict(DEFAULT_MUST_REALIZE)

    bodies = manifest.get("body", [])
    report.total_body_count = manifest.get("total_body_count", len(bodies))
    if manifest.get("reviewed_body_count") != len(bodies):
        report.errors.append(
            f"reviewed_body_count says {manifest.get('reviewed_body_count')}, "
            f"manifest has {len(bodies)} bodies"
        )
    if isinstance(report.total_body_count, int) and report.total_body_count < len(bodies):
        report.errors.append("total_body_count is below the reviewed body count")

    seen: set[str] = set()
    for ordinal, body in enumerate(bodies):
        item = body.get("java_item")
        if item in seen:
            report.errors.append(f"duplicate reviewed body {item!r}")
            continue
        if isinstance(item, str):
            seen.add(item)
        errors, coverage = _validate_body(
            body, evidence.get(item), ordinal, blanket_max_span, must_realize
        )
        report.errors.extend(errors)
        report.bodies.append(coverage)

    declared_crosswalked = manifest.get("crosswalked_body_count")
    actual_crosswalked = report.crosswalked_bodies
    if declared_crosswalked is not None and declared_crosswalked != actual_crosswalked:
        report.errors.append(
            f"crosswalked_body_count says {declared_crosswalked}, manifest has "
            f"{actual_crosswalked} fully-decided crosswalked bodies"
        )

    if strict and report.node_undecided:
        report.errors.append(
            f"--strict: {report.node_undecided} node(s) across "
            f"{len(report.partial_bodies)} body/bodies remain undecided"
        )
    return report


def format_coverage(report: Report) -> str:
    lines: list[str] = []
    lines.append(
        f"per-node crosswalk coverage: {report.node_decided}/{report.node_total} nodes "
        f"decided across {len(report.bodies)} reviewed body/bodies "
        f"({report.total_body_count} total bytecode bodies); "
        f"{report.crosswalked_bodies} fully crosswalked"
    )
    if report.node_undecided:
        lines.append(f"UNCHECKED: {report.node_undecided} node(s) still undecided")
    for body in sorted(report.bodies, key=lambda b: (-b.undecided, b.java_item)):
        flag = "OK " if body.complete else "PARTIAL"
        line = (
            f"{flag}\t{body.decided}/{body.total}\t"
            f"(java {body.java_decided}/{body.java_total}, "
            f"rust {body.rust_decided}/{body.rust_total})\t{body.java_item}"
        )
        if not body.complete:
            line += f"\t<= {body.undecided} UNDECIDED"
        lines.append(line)
    return "\n".join(lines)


# --------------------------------------------------------------------------
# Self-test: prove the gate can fail (playbook R3).
# --------------------------------------------------------------------------

FIXTURE_DIR = Path(__file__).resolve().parent / "fixtures"
DEFAULT_MANIFEST = FIXTURE_DIR / "paint_radio_row.crosswalk.toml"
DEFAULT_EVIDENCE = FIXTURE_DIR / "paint_radio_row.evidence.toml"


def self_test(manifest: dict, evidence_data: dict) -> int:
    """Perturb each protection in turn; require the exact red that guards it."""

    base_evidence = load_evidence(evidence_data)
    baseline = validate(manifest, base_evidence, strict=True)
    if baseline.errors:
        print("self-test baseline is already red:", file=sys.stderr)
        print("\n".join(baseline.errors), file=sys.stderr)
        return 1

    def drop_a_decision(data: dict) -> None:
        data["body"][0]["op"] = data["body"][0]["op"][:-1]

    def coarsen(data: dict) -> None:
        body = data["body"][0]
        rust_last = len(base_evidence[body["java_item"]].rust[0].nodes) - 1
        body["op"] = [
            {
                "semantic": "one blanket over the whole body",
                "java_range": [[0, body["java_node_count"] - 1]],
                "rust_range": [{"target": 0, "start": 0, "end": rust_last}],
            }
        ]
        body["adapt"] = []

    def flip_bytecode(data: dict) -> None:
        data["body"][0]["code_sha256"] = "0" * 64

    def flip_java_ast(data: dict) -> None:
        data["body"][0]["java_ast_sha256"] = "0" * 64

    def flip_rust_ast(data: dict) -> None:
        data["body"][0]["rust"][0]["ast_sha256"] = "0" * 64

    manifest_checks = [
        (drop_a_decision, "undecided"),
        (coarsen, "coarse blanket"),
        (flip_bytecode, "bytecode Code-attr digest changed"),
        (flip_java_ast, "canonical Java AST digest changed"),
        (flip_rust_ast, "reviewed Rust AST digest changed"),
    ]
    for mutate, expected in manifest_checks:
        data = copy.deepcopy(manifest)
        mutate(data)
        report = validate(data, base_evidence, strict=True)
        if not report.errors:
            print(f"self-test: perturbation {expected!r} stayed green", file=sys.stderr)
            return 1
        if not any(expected in error for error in report.errors):
            print(
                f"self-test: perturbation red lacks {expected!r}:\n"
                + "\n".join(report.errors),
                file=sys.stderr,
            )
            return 1

    # A one-node drift in the LIVE evidence must break the node-inventory lock.
    drifted = copy.deepcopy(evidence_data)
    drifted_body = drifted["body"][0]
    drifted_body["java_nodes"][-1] = drifted_body["java_nodes"][-1] + " DRIFT"
    report = validate(manifest, load_evidence(drifted), strict=True)
    if not any("Java node inventory changed" in error for error in report.errors):
        print("self-test: a one-node Java drift did not break the lock", file=sys.stderr)
        return 1

    print(
        "crosswalk self-test ok: 5 manifest perturbations (dropped decision, coarse "
        "blanket, bytecode/Java-AST/Rust-AST digest) each went red, and a one-node "
        "evidence drift broke the node-inventory lock"
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path, nargs="?", default=DEFAULT_MANIFEST)
    parser.add_argument("--evidence", type=Path, default=DEFAULT_EVIDENCE)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--strict", action="store_true", help="require 100%% node coverage (0 undecided)"
    )
    parser.add_argument(
        "--coverage", action="store_true", help="print the per-body coverage report"
    )
    arguments = parser.parse_args()

    manifest = tomllib.loads(arguments.manifest.read_text(encoding="utf-8"))
    evidence_data = tomllib.loads(arguments.evidence.read_text(encoding="utf-8"))

    if arguments.self_test:
        return self_test(manifest, evidence_data)

    evidence = load_evidence(evidence_data)
    report = validate(manifest, evidence, strict=arguments.strict)
    if arguments.coverage or report.errors:
        print(format_coverage(report))
    if report.errors:
        print("---", file=sys.stderr)
        for error in report.errors[:80]:
            print(error, file=sys.stderr)
        return 1
    if not arguments.coverage:
        print(format_coverage(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
