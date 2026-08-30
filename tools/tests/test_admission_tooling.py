from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from tools.port.admission import (
    AdmissionError,
    load_plan,
    require_complete_class_plan_closure,
)
from tools.port.scaffold_admission import render_crosswalk, render_variants


PLAN = '''schema_version = 1
id = "crc_helper"
label = "Fixture CRC"
owner = "e"
java_owner = "CrcHelper"
java_source = "java/src/CrcHelper.java"
crosswalk_manifest = "transliteration/audits/crc.crosswalk.toml"
variant_manifest = "java/reconstruction/variants/crc.toml"

[[body]]
original_name = "a"
descriptor = "()V"
java_item = "run()"
rust = [{ file = "transliteration/game.rs", item = "fn:run" }]
'''


class AdmissionToolingTests(unittest.TestCase):
    def plan(self) -> dict:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        path = Path(temporary.name) / "plan.toml"
        path.write_text(PLAN, encoding="utf-8")
        return load_plan(path)

    def test_plan_rejects_paths_that_escape_the_repository(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "bad.toml"
            path.write_text(PLAN.replace("java/src/CrcHelper.java", "../outside.java"))
            with self.assertRaisesRegex(AdmissionError, "inside the game repository"):
                load_plan(path)

    def test_crosswalk_skeleton_keeps_the_whole_game_denominator(self) -> None:
        rendered = render_crosswalk(self.plan(), "build", 200)
        self.assertIn("total_body_count = 200", rendered)
        self.assertIn('java_item = "e.a:()V"', rendered)
        self.assertIn("crosswalked_body_count = 0", rendered)

    def test_variant_skeleton_is_mechanical_and_noncommon_rows_stay_red(self) -> None:
        plan = self.plan()
        live = {
            "a:()V": {
                "one": ("a", "()V", "a" * 64),
                "two": ("a", "()V", "b" * 64),
            }
        }
        rendered = render_variants(plan, ["one", "two"], live)
        self.assertIn('classification = "REVIEW_REQUIRED"', rendered)
        self.assertIn("live builds differ", rendered)

    def test_repository_discovery_cannot_shrink_complete_class_denominator(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            reconstruction = root / "java" / "reconstruction"
            admissions = reconstruction / "admissions"
            admissions.mkdir(parents=True)
            (reconstruction / "symbols.toml").write_text(
                '''[coverage]\ncomplete_classes = 1\n[[class]]\noriginal = "e"\ncoverage = "complete"\n''',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(AdmissionError, "exactly close"):
                require_complete_class_plan_closure(root, [])
            plan_path = admissions / "crc.toml"
            plan_path.write_text(PLAN, encoding="utf-8")
            require_complete_class_plan_closure(root, [plan_path])


if __name__ == "__main__":
    unittest.main()
