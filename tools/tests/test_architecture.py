import importlib.util
import pathlib
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).parents[1] / "architecture" / "validate_layers.py"
SPEC = importlib.util.spec_from_file_location("validate_layers", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class ArchitectureTests(unittest.TestCase):
    def test_shipped_graph_is_valid(self):
        root = pathlib.Path(__file__).parents[2]
        self.assertEqual([], MODULE.validate(root))

    def test_unclassified_reusable_crate_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            for name in MODULE.ALLOWED_INTERNAL:
                crate = root / "crates" / name
                (crate / "src").mkdir(parents=True)
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "{name}"\nversion = "0.0.0"\n', encoding="utf-8"
                )
            crate = root / "crates" / "j2me-surprise"
            (crate / "src").mkdir(parents=True)
            (crate / "Cargo.toml").write_text(
                '[package]\nname = "j2me-surprise"\nversion = "0.0.0"\n', encoding="utf-8"
            )
            self.assertTrue(any("unclassified" in error for error in MODULE.validate(root)))


if __name__ == "__main__":
    unittest.main()
