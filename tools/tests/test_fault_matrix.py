from __future__ import annotations

import unittest

from tools.oracle.fault_matrix import staged_fault_cases


class FaultMatrixTests(unittest.TestCase):
    def test_emits_each_failure_kind_and_cleanup_precedence(self) -> None:
        self.assertEqual(
            staged_fault_cases(
                ("open", "send", "close"),
                "|fixture",
                cleanup_stage="close",
                cleanup_after=("send",),
            ),
            (
                "open-ex|fixture",
                "open-throw|fixture",
                "send-ex|fixture",
                "send-throw|fixture",
                "close-ex|fixture",
                "close-throw|fixture",
                "send-ex+close-throw|fixture",
                "send-throw+close-ex|fixture",
            ),
        )

    def test_rejects_ambiguous_or_unknown_stages(self) -> None:
        with self.assertRaises(ValueError):
            staged_fault_cases(("send", "send"), "")
        with self.assertRaises(ValueError):
            staged_fault_cases(
                ("send", "close"),
                "",
                cleanup_stage="close",
                cleanup_after=("missing",),
            )


if __name__ == "__main__":
    unittest.main()
