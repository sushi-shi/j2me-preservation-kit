import importlib.util
import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).parents[1] / "device" / "validate_profiles.py"
SPEC = importlib.util.spec_from_file_location("validate_profiles", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class DeviceProfileTests(unittest.TestCase):
    def test_empty_template_catalog_and_build_matrix_are_valid(self):
        root = pathlib.Path(__file__).parents[2]
        self.assertEqual(
            [],
            MODULE.validate(
                root / "device-profiles.toml",
                root / "java" / "reconstruction" / "builds.toml",
            ),
        )


if __name__ == "__main__":
    unittest.main()
