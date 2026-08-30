from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "port"))

import native_reachability  # noqa: E402
import validate_completeness  # noqa: E402
import validate_variants  # noqa: E402


class PortCompletenessTests(unittest.TestCase):
    def setUp(self) -> None:
        (
            self.manifest,
            self.fields,
            self.methods,
            self.edges,
            self.rust,
        ) = validate_completeness.synthetic_fixture()

    def errors(self, manifest: dict | None = None) -> list[str]:
        return validate_completeness.validate(
            manifest or self.manifest,
            self.fields,
            self.methods,
            self.edges,
            self.rust,
        )

    def test_complete_fixture_is_green(self) -> None:
        self.assertEqual(self.errors(), [])

    def test_dropped_java_field_is_red(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["field_ownership"] = []
        self.assertTrue(
            any("Java field inventory missing" in error for error in self.errors(broken))
        )

    def test_orphan_rust_declaration_is_red(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["rust_declaration"] = broken["rust_declaration"][:-1]
        self.assertTrue(
            any("production Rust inventory missing" in error for error in self.errors(broken))
        )

    def test_changed_rust_ast_is_red(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["rust_declaration"][1]["ast_sha256"] = "0" * 64
        self.assertTrue(any("AST changed" in error for error in self.errors(broken)))

    def test_call_path_must_name_a_live_ast_marker(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["call_edge"][0]["rust_path"][0]["contains"] = ["not_the_callee"]
        self.assertTrue(
            any("lacks AST marker" in error for error in self.errors(broken))
        )

    def test_one_rust_state_field_cannot_have_two_java_owners(self) -> None:
        extra = "a.y:I"
        fields = self.fields | {extra}
        broken = copy.deepcopy(self.manifest)
        broken["ratchet"]["java_fields"] = 2
        duplicate = copy.deepcopy(broken["field_ownership"][0])
        duplicate["java"] = extra
        broken["field_ownership"].append(duplicate)
        errors = validate_completeness.validate(
            broken, fields, self.methods, self.edges, self.rust
        )
        self.assertTrue(any("has two Java owners" in error for error in errors))

    def test_state_representation_type_is_a_live_ast_witness(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["field_ownership"][0]["rust"][0]["type_contains"] = "u32"
        self.assertTrue(any("lacks type marker" in error for error in self.errors(broken)))


class NativeReachabilityTests(unittest.TestCase):
    def test_oracle_target_must_stay_out_of_production_closure(self) -> None:
        graph = {
            "game::main": {"game::tick"},
            "game::tick": set(),
            "oracle::fixture": set(),
        }
        targets = [
            {
                "symbol": "oracle::fixture",
                "expect": "unreachable",
                "category": "oracle-only",
                "reason": "test process only",
            }
        ]
        self.assertEqual(
            native_reachability.validate_expectations(
                graph, ["game::main"], targets, 2
            ),
            [],
        )
        graph["game::tick"].add("oracle::fixture")
        errors = native_reachability.validate_expectations(
            graph, ["game::main"], targets, 3
        )
        self.assertTrue(any("leaked into production" in error for error in errors))


class VariantLedgerTests(unittest.TestCase):
    def test_reviewed_variant_groups_are_exact(self) -> None:
        manifest, live = validate_variants.synthetic_fixture()
        self.assertEqual(validate_variants.validate(manifest, live), [])
        live["1"]["sony"] = ("c", "(I)V", "e" * 64)
        self.assertTrue(
            any(
                "variant observations changed" in error
                for error in validate_variants.validate(manifest, live)
            )
        )

    def test_common_cannot_hide_absent_signature(self) -> None:
        manifest = {
            "schema_version": 1,
            "builds": ["a", "b"],
            "expected_build_count": 2,
            "expected_method_keys": 1,
            "method": [
                {
                    "key": "tick:()V",
                    "classification": "common",
                    "observation": [
                        {
                            "builds": ["a"],
                            "present": True,
                            "name": "tick",
                            "descriptor": "()V",
                            "shape_sha256": "a" * 64,
                        },
                        {"builds": ["b"], "present": False},
                    ],
                }
            ],
        }
        live = {
            "tick:()V": {
                "a": ("tick", "()V", "a" * 64),
                "b": None,
            }
        }
        errors = validate_variants.validate(manifest, live)
        self.assertTrue(any("common hides" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
