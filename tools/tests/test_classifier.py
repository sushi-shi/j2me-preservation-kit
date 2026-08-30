from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

CORPUS_DIR = Path(__file__).resolve().parents[1] / "corpus"
sys.path.insert(0, str(CORPUS_DIR))
MODULE_PATH = CORPUS_DIR / "classify.py"
SPEC = importlib.util.spec_from_file_location("classifier_under_test", MODULE_PATH)
assert SPEC and SPEC.loader
CLASSIFIER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CLASSIFIER
SPEC.loader.exec_module(CLASSIFIER)


class ClassifierTests(unittest.TestCase):
    def test_negative_control_perturbs_parsed_fingerprints(self) -> None:
        method = CLASSIFIER.classfile.MethodSymbol(
            ordinal=0,
            name="method",
            descriptor="()V",
            access_flags=1,
            code_size=3,
            code_sha256="1" * 64,
            opcode_sha256="2" * 64,
            shape_sha256="3" * 64,
        )
        info = CLASSIFIER.classfile.ClassInfo(
            member_path="Game.class",
            internal_name="Game",
            access_flags=1,
            super_name="java/lang/Object",
            interfaces=[],
            major_version=46,
            minor_version=0,
            class_sha256="4" * 64,
            shape_sha256="5" * 64,
            fields=[],
            methods=[method],
        )
        before = (
            info.class_sha256,
            info.shape_sha256,
            method.code_sha256,
            method.opcode_sha256,
            method.shape_sha256,
        )

        CLASSIFIER.perturb_class_fingerprint(info)

        after = (
            info.class_sha256,
            info.shape_sha256,
            method.code_sha256,
            method.opcode_sha256,
            method.shape_sha256,
        )
        self.assertTrue(all(left != right for left, right in zip(before, after)))


if __name__ == "__main__":
    unittest.main()
