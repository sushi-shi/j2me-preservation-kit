#!/usr/bin/env python3
"""Run every declared per-class admission without per-class Just boilerplate."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "port"))

from admission import (  # noqa: E402
    AdmissionError,
    discover_plans,
    load_plan,
    require_complete_class_plan_closure,
)


def run(command: tuple[str, ...] | list[str]) -> None:
    subprocess.run(command, cwd=ROOT, check=True)


def run_plan(path: Path) -> None:
    plan = load_plan(path)
    run(("python3", "tools/port/validate_variants.py", plan["variant_manifest"]))
    run(("python3", "tools/ast/run_live_crosswalk.py", str(path)))
    run(("python3", "tools/ast/run_live_crosswalk.py", str(path), "--self-test"))
    oracle = plan.get("oracle")
    if oracle is not None:
        run(oracle["command"])
        run(oracle["canfail_command"])


def main() -> int:
    explicit = bool(sys.argv[1:])
    paths = [Path(argument) for argument in sys.argv[1:]] if explicit else discover_plans(ROOT)
    try:
        if not explicit:
            require_complete_class_plan_closure(ROOT, paths)
        if paths:
            run(("python3", "tools/java/validate_symbols.py"))
            run(("python3", "tools/java/validate_symbols.py", "--self-test"))
        for path in paths:
            run_plan(path)
        print(f"admissions OK: {len(paths)} plan(s)")
        return 0
    except (AdmissionError, OSError, subprocess.CalledProcessError) as error:
        print(f"admissions FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
