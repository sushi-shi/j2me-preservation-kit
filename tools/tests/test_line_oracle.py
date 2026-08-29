from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "oracle"))

from line_oracle import OracleError, ProcessSpec, compare_outputs, run_oracle  # noqa: E402


ECHO = (
    "import sys; "
    "[print(line.rstrip('\\n')) for line in sys.stdin]"
)


class LineOracleTests(unittest.TestCase):
    def test_two_independent_processes_match_and_self_test_bites(self) -> None:
        implementations = [
            ProcessSpec("reference", (sys.executable, "-c", ECHO)),
            ProcessSpec("candidate", (sys.executable, "-c", ECHO)),
        ]
        report = run_oracle(
            ["edge -2147483648", "edge 0", "edge 2147483647"],
            implementations,
            reference_label="reference",
            self_test=True,
        )
        self.assertEqual(report.mismatches, ())
        self.assertEqual(report.case_count, 3)

    def test_cardinality_and_empty_case_sets_are_rejected(self) -> None:
        with self.assertRaises(OracleError):
            compare_outputs(
                ["one"],
                {"reference": ["one"], "candidate": []},
                reference_label="reference",
            )
        with self.assertRaises(OracleError):
            compare_outputs(
                [],
                {"reference": [], "candidate": []},
                reference_label="reference",
            )


if __name__ == "__main__":
    unittest.main()
