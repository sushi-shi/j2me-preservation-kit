from __future__ import annotations

import importlib.util
import pathlib
import sys
import unittest


DEVICE_DIR = pathlib.Path(__file__).parents[1] / "device"
CORPUS_DIR = pathlib.Path(__file__).parents[1] / "corpus"
sys.path.insert(0, str(CORPUS_DIR))
SCRIPT = DEVICE_DIR / "audit_evidence.py"
SPEC = importlib.util.spec_from_file_location("audit_device_evidence", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class DeviceEvidenceTests(unittest.TestCase):
    def test_method_findings_are_call_scoped_and_keep_candidates_unclaimed(self):
        method = MODULE.classfile.MethodSymbol(
            ordinal=0,
            name="method",
            descriptor="(I)V",
            access_flags=1,
            calls=["javax/microedition/lcdui/Canvas.getGameAction:(I)I"],
            loaded_constants=["not automatically an argument", 23],
            numeric_immediates=[-6, -7],
        )
        findings = MODULE.method_findings("s", method)
        self.assertEqual(["canvas-game-action"], [item["kind"] for item in findings])
        self.assertEqual(
            ["not automatically an argument"], findings[0]["string_candidates"]
        )
        self.assertEqual([23, -6, -7], findings[0]["numeric_candidates"])

    def test_descriptor_continuations_and_axes_are_reported(self):
        attributes = MODULE.parse_descriptor(
            b"MicroEdition-Profile: MIDP-2.0\r\n"
            b"Nokia-MIDlet-Category: Ga\r\n me\r\n"
            b"MIDlet-Name: ignored\r\n"
        )
        self.assertEqual("Game", attributes["Nokia-MIDlet-Category"])
        findings = MODULE.header_findings(attributes)
        self.assertEqual(
            [("system", "MicroEdition-Profile"), ("vendor", "Nokia-MIDlet-Category")],
            [(item["axis"], item["key"]) for item in findings],
        )


if __name__ == "__main__":
    unittest.main()
