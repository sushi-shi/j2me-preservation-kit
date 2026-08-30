#!/usr/bin/env python3
"""Run a live Java/Rust crosswalk from one declarative admission plan."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "ast"))
sys.path.insert(0, str(ROOT / "tools" / "port"))

from admission import AdmissionError, load_plan, original_key  # noqa: E402
from live_crosswalk import BodySpec, RustTarget, run_gate  # noqa: E402


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: run_live_crosswalk.py PLAN [--inventory|--self-test]", file=sys.stderr)
        return 2
    try:
        plan = load_plan(Path(sys.argv[1]))
    except AdmissionError as error:
        print(f"admission crosswalk FAIL: {error}", file=sys.stderr)
        return 1
    specs = tuple(
        BodySpec(
            key=original_key(plan, body),
            original_owner=plan["owner"],
            original_name=body["original_name"],
            descriptor=body["descriptor"],
            java_source=plan["java_source"],
            java_owner=plan["java_owner"],
            java_item=body["java_item"],
            rust=tuple(RustTarget(target["file"], target["item"]) for target in body["rust"]),
        )
        for body in plan["body"]
    )
    return run_gate(
        label=plan["label"],
        manifest_path=ROOT / plan["crosswalk_manifest"],
        specs=specs,
        argv=sys.argv[2:],
    )


if __name__ == "__main__":
    raise SystemExit(main())
