from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "ast"))

from validate_crosswalk import validate_manifest  # noqa: E402


HASH = "0" * 64


def manifest() -> dict:
    return {
        "schema_version": 1,
        "total_body_count": 1,
        "reviewed_body_count": 1,
        "semantic_reviewed_body_count": 1,
        "total_field_count": 0,
        "semantic_reviewed_field_count": 0,
        "body": [
            {
                "java_item": "method()",
                "code_sha256": HASH,
                "opcode_sha256": HASH,
                "java_ast_sha256": HASH,
                "java_nodes_sha256": HASH,
                "java_node_count": 1,
                "rust_node_counts": [1],
                "operation": [
                    {
                        "semantic": "return the same value",
                        "java_nodes": [0],
                        "rust_node_ranges": [{"target": 0, "start": 0, "end": 0}],
                    }
                ],
                "rust": [
                    {
                        "ast_sha256": HASH,
                        "nodes_sha256": HASH,
                        "node_count": 1,
                    }
                ],
                "semantic_status": "crosswalked",
                "semantic_review": "Synthetic unit-test row.",
            }
        ],
    }


class CrosswalkValidatorTests(unittest.TestCase):
    def test_complete_partition_passes(self) -> None:
        self.assertEqual(validate_manifest(manifest()), [])

    def test_duplicate_and_uncovered_nodes_fail(self) -> None:
        duplicate = copy.deepcopy(manifest())
        duplicate["body"][0]["adaptation"] = [
            {"reason": "duplicate", "java_nodes": [0]}
        ]
        self.assertTrue(
            any("Java node 0 has 2 owners" in error for error in validate_manifest(duplicate))
        )

        uncovered = copy.deepcopy(manifest())
        uncovered["body"][0]["operation"] = []
        errors = validate_manifest(uncovered)
        self.assertTrue(any("no semantic mappings" in error for error in errors))
        self.assertTrue(any("Java node 0 has 0 owners" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
