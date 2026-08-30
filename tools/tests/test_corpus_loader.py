from __future__ import annotations

import hashlib
import importlib.util
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parents[1] / "corpus" / "corpus.py"
SPEC = importlib.util.spec_from_file_location("corpus_loader_under_test", MODULE_PATH)
assert SPEC and SPEC.loader
CORPUS = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CORPUS
SPEC.loader.exec_module(CORPUS)


class CorpusLoaderTests(unittest.TestCase):
    def test_resolves_a_hash_identified_jar_nested_in_zip(self) -> None:
        payload = b"synthetic nested jar payload"
        entry = {
            "id": "nested",
            "sha256": hashlib.sha256(payload).hexdigest(),
            "bytes": len(payload),
            "containers": ["inside _originals/carrier.zip"],
        }
        with tempfile.TemporaryDirectory() as temporary:
            originals = Path(temporary)
            with zipfile.ZipFile(originals / "carrier.zip", "w") as archive:
                archive.writestr("some/device/game.jar", payload)
            previous = CORPUS.ORIGINALS
            CORPUS.ORIGINALS = originals
            try:
                self.assertEqual(payload, CORPUS.payload_bytes(entry))
            finally:
                CORPUS.ORIGINALS = previous


if __name__ == "__main__":
    unittest.main()
