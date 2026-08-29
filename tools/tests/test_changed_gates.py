from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


RUNNER = Path(__file__).parents[1] / "gates" / "check_changed.py"


def load_runner():
    spec = importlib.util.spec_from_file_location("changed_gate_runner", RUNNER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class ChangedGateTests(unittest.TestCase):
    def test_generated_python_caches_do_not_dirty_directory_inputs(self):
        runner = load_runner()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sources = root / "tools"
            cache = sources / "__pycache__"
            cache.mkdir(parents=True)
            source = sources / "validator.py"
            source.write_text("pass\n", encoding="utf-8")
            (cache / "validator.cpython-314.pyc").write_bytes(b"first")
            (sources / "standalone.pyc").write_bytes(b"second")

            files, existed = runner.expand_pattern(root, "tools")
            self.assertTrue(existed)
            self.assertEqual(files, [source])

    def test_only_changed_successful_fingerprints_are_cached(self):
        runner = load_runner()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            gate_dir = root / "tools" / "gates"
            gate_dir.mkdir(parents=True)
            (root / "Justfile").write_text("default:\n    @true\n", encoding="utf-8")
            (gate_dir / "check_changed.py").write_text("# test runner identity\n")
            runner.__file__ = str(gate_dir / "check_changed.py")
            (root / "source.txt").write_text("one", encoding="utf-8")
            increment = (
                "from pathlib import Path;"
                "p=Path('marker.txt');"
                "p.write_text(str(int(p.read_text())+1) if p.exists() else '1')"
            )
            command = json.dumps([sys.executable, "-c", increment])
            config = gate_dir / "gates.toml"
            config.write_text(
                "schema_version = 1\n"
                "project_root = '../..'\n"
                "state_file = 'target/gates/state.json'\n"
                "[[gate]]\n"
                "name = 'sample'\n"
                "inputs = ['source.txt']\n"
                f"commands = [{command}]\n",
                encoding="utf-8",
            )

            self.assertEqual(runner.main(["--config", str(config)]), 0)
            self.assertEqual((root / "marker.txt").read_text(), "1")
            self.assertEqual(runner.main(["--config", str(config)]), 0)
            self.assertEqual((root / "marker.txt").read_text(), "1")

            (root / "source.txt").write_text("two", encoding="utf-8")
            self.assertEqual(runner.main(["--config", str(config)]), 0)
            self.assertEqual((root / "marker.txt").read_text(), "2")

    def test_failed_fingerprint_is_not_cached(self):
        runner = load_runner()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            gate_dir = root / "tools" / "gates"
            gate_dir.mkdir(parents=True)
            (root / "Justfile").write_text("default:\n    @true\n", encoding="utf-8")
            (gate_dir / "check_changed.py").write_text("# test runner identity\n")
            runner.__file__ = str(gate_dir / "check_changed.py")
            (root / "source.txt").write_text("one", encoding="utf-8")
            config = gate_dir / "gates.toml"
            config.write_text(
                "schema_version = 1\n"
                "project_root = '../..'\n"
                "state_file = 'target/gates/state.json'\n"
                "[[gate]]\n"
                "name = 'failing'\n"
                "inputs = ['source.txt']\n"
                f"commands = [[{json.dumps(sys.executable)}, '-c', 'raise SystemExit(7)']]\n",
                encoding="utf-8",
            )

            self.assertEqual(runner.main(["--config", str(config)]), 7)
            state = root / "target" / "gates" / "state.json"
            self.assertFalse(state.exists())
            self.assertEqual(runner.main(["--config", str(config)]), 7)


if __name__ == "__main__":
    unittest.main()
