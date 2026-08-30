from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parents[1] / "java" / "validate_symbols.py"
SPEC = importlib.util.spec_from_file_location("symbol_validator_under_test", MODULE_PATH)
assert SPEC and SPEC.loader
VALIDATOR = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = VALIDATOR
SPEC.loader.exec_module(VALIDATOR)


class SymbolValidatorTests(unittest.TestCase):
    def test_descriptor_parameter_count_handles_arrays_and_objects(self) -> None:
        self.assertEqual(0, VALIDATOR.parameter_count("()V"))
        self.assertEqual(4, VALIDATOR.parameter_count("([I[[Ljava/lang/String;JZ)V"))

    def test_placeholder_names_are_rejected(self) -> None:
        for name in ("a", "f37a", "m31a"):
            with self.assertRaises(VALIDATOR.SymbolError):
                VALIDATOR.semantic_name(name, "test")

    def test_symbol_ledger_cannot_shrink_the_game_denominator(self) -> None:
        with self.assertRaises(VALIDATOR.SymbolError):
            VALIDATOR.require_baseline_closure(
                {"baseline_classes": ["Main"]}, ["Main", "GameCanvas"]
            )
        VALIDATOR.require_baseline_closure(
            {"baseline_classes": ["Main", "GameCanvas"]}, ["Main", "GameCanvas"]
        )


if __name__ == "__main__":
    unittest.main()
