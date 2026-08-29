from __future__ import annotations

import copy
import sys
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "ast"))

from validate_crosswalk import (  # noqa: E402
    load_evidence,
    node_inventory_digest,
    self_test,
    validate,
)

FIXTURES = ROOT / "tools" / "ast" / "fixtures"
HASH = "a" * 64


def _load(name: str) -> dict:
    return tomllib.loads((FIXTURES / name).read_text(encoding="utf-8"))


EVIDENCE = _load("paint_radio_row.evidence.toml")
FINE = _load("paint_radio_row.crosswalk.toml")
COARSE = _load("paint_radio_row.coarse.toml")
BUGGY = _load("paint_radio_row.buggy.toml")


def _synthetic() -> tuple[dict, dict]:
    """A minimal consistent body: `x = a[1] / a[2]` paired node-for-node."""
    java_nodes = ["VARIABLE\tx\tint", "DIVIDE\t", "ARRAY_ACCESS\t", "ARRAY_ACCESS\t"]
    rust_nodes = ["LOCAL\tx", "CALL\t2", "PATH_EXPR\tj2me_jvm :: java_div", "INDEX\t"]
    evidence = {
        "body": [
            {
                "java_item": "m()",
                "code_sha256": HASH,
                "opcode_sha256": HASH,
                "java_ast_sha256": HASH,
                "java_nodes": java_nodes,
                "rust": [
                    {
                        "file": "x.rs",
                        "item": "fn:m",
                        "ast_sha256": HASH,
                        "nodes": rust_nodes,
                    }
                ],
            }
        ]
    }
    manifest = {
        "schema_version": 2,
        "total_body_count": 1,
        "reviewed_body_count": 1,
        "crosswalked_body_count": 1,
        "body": [
            {
                "java_item": "m()",
                "code_sha256": HASH,
                "opcode_sha256": HASH,
                "java_ast_sha256": HASH,
                "java_nodes_sha256": node_inventory_digest(java_nodes),
                "java_node_count": len(java_nodes),
                "semantic_status": "crosswalked",
                "review": "synthetic division body",
                "rust": [
                    {
                        "file": "x.rs",
                        "item": "fn:m",
                        "ast_sha256": HASH,
                        "nodes_sha256": node_inventory_digest(rust_nodes),
                        "node_count": len(rust_nodes),
                    }
                ],
                "op": [
                    {
                        "semantic": "x = a[1] / a[2] via java_div",
                        "java_range": [[0, 3]],
                        "rust_range": [{"target": 0, "start": 0, "end": 3}],
                    }
                ],
                "adapt": [],
            }
        ],
    }
    return manifest, evidence


class CrosswalkValidatorTests(unittest.TestCase):
    def test_synthetic_partition_is_green(self) -> None:
        manifest, evidence = _synthetic()
        report = validate(manifest, load_evidence(evidence), strict=True)
        self.assertEqual(report.errors, [])
        self.assertEqual(report.node_undecided, 0)
        self.assertEqual(report.crosswalked_bodies, 1)

    def test_fine_fixture_is_green(self) -> None:
        report = validate(FINE, load_evidence(EVIDENCE), strict=True)
        self.assertEqual(report.errors, [])
        self.assertEqual(report.node_undecided, 0)

    def test_coarse_blanket_is_rejected(self) -> None:
        report = validate(COARSE, load_evidence(EVIDENCE))
        self.assertTrue(any("coarse blanket" in e for e in report.errors), report.errors)

    def test_operator_parity_catches_div_vs_call(self) -> None:
        # The recreated paint_radio_row bug: Rust sm(1717,1721) has no division.
        report = validate(BUGGY, load_evidence(EVIDENCE))
        self.assertTrue(
            any("Java DIVIDE with no Rust realization" in e for e in report.errors),
            report.errors,
        )

    def test_undecided_nodes_are_counted_and_body_partial(self) -> None:
        manifest, evidence = _synthetic()
        manifest["body"][0]["op"] = []  # remove the only decision
        manifest["body"][0]["semantic_status"] = "partial"
        manifest["crosswalked_body_count"] = 0
        # Non-strict: partial is reported, not fatal for the missing coverage itself.
        report = validate(manifest, load_evidence(evidence))
        self.assertEqual(report.node_undecided, 8)
        self.assertEqual(len(report.partial_bodies), 1)
        # Strict: undecided nodes are fatal.
        strict = validate(manifest, load_evidence(evidence), strict=True)
        self.assertTrue(any("undecided" in e for e in strict.errors), strict.errors)

    def test_crosswalked_claim_requires_full_coverage(self) -> None:
        manifest, evidence = _synthetic()
        manifest["body"][0]["op"] = [
            {
                "semantic": "partial",
                "java_range": [[0, 1]],
                "rust_range": [{"target": 0, "start": 0, "end": 1}],
            }
        ]
        report = validate(manifest, load_evidence(evidence))
        self.assertTrue(
            any("crosswalked but" in e and "undecided" in e for e in report.errors),
            report.errors,
        )

    def test_duplicate_ownership_fails(self) -> None:
        manifest, evidence = _synthetic()
        manifest["body"][0]["adapt"] = [
            {"category": "erased", "reason": "double claim", "java": [0]}
        ]
        report = validate(manifest, load_evidence(evidence))
        self.assertTrue(
            any("Java node 0 has 2 owners" in e for e in report.errors), report.errors
        )

    def test_one_node_drift_breaks_the_lock(self) -> None:
        manifest, evidence = _synthetic()
        evidence["body"][0]["java_nodes"][-1] += " DRIFT"
        report = validate(manifest, load_evidence(evidence))
        self.assertTrue(
            any("Java node inventory changed" in e for e in report.errors), report.errors
        )

    def test_missing_comment_is_rejected(self) -> None:
        manifest, evidence = _synthetic()
        del manifest["body"][0]["op"][0]["semantic"]
        report = validate(manifest, load_evidence(evidence))
        self.assertTrue(
            any("needs a semantic comment" in e for e in report.errors), report.errors
        )

    def test_shipped_self_test_passes(self) -> None:
        # The can-fail proof the gate ships must itself be green.
        self.assertEqual(self_test(copy.deepcopy(FINE), copy.deepcopy(EVIDENCE)), 0)


if __name__ == "__main__":
    unittest.main()
