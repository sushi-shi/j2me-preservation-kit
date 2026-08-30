#!/usr/bin/env python3
"""Generate bytecode-backed crosswalk and variant skeletons from an admission plan."""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "corpus"))
sys.path.insert(0, str(ROOT / "tools" / "port"))

import corpus  # noqa: E402
from admission import AdmissionError, load_plan, original_key  # noqa: E402
from validate_variants import collect_live, grouped_observations  # noqa: E402


def quote(value: object) -> str:
    return json.dumps(value, ensure_ascii=False)


def render_crosswalk(plan: dict, baseline: str, total_bodies: int) -> str:
    lines = [
        "schema_version = 2",
        f"build = {quote(baseline)}",
        f"total_body_count = {total_bodies}",
        f"reviewed_body_count = {len(plan['body'])}",
        "crosswalked_body_count = 0",
        "",
        "[policy]",
        "blanket_max_span = 48",
    ]
    for body in plan["body"]:
        lines.extend(("", "[[body]]", f"java_item = {quote(original_key(plan, body))}"))
    return "\n".join(lines) + "\n"


def render_variants(
    plan: dict,
    builds: list[str],
    live: dict[str, dict[str, tuple[str, str, str] | None]],
) -> str:
    lines = [
        "schema_version = 1",
        f"owner = {quote(plan['owner'])}",
        'identity = "signature"',
        f"builds = {quote(builds)}",
        f"expected_build_count = {len(builds)}",
        f"expected_method_keys = {len(live)}",
        "",
        "# Mechanical live observations. Any REVIEW_REQUIRED classification must",
        "# be replaced by a reviewed policy and reason before admission can pass.",
    ]
    for key, observations in sorted(live.items()):
        grouped = grouped_observations(observations)
        common = (
            len(grouped) == 1
            and grouped[0].get("present") is True
            and grouped[0].get("builds") == sorted(builds)
        )
        lines.extend(("", "[[method]]", f"key = {quote(key)}"))
        lines.append('classification = "common"' if common else 'classification = "REVIEW_REQUIRED"')
        if not common:
            lines.append('reason = "REVIEW REQUIRED: live builds differ"')
        lines.append("observation = [")
        for observation in grouped:
            fields = [
                f"builds = {quote(observation['builds'])}",
                f"present = {'true' if observation['present'] else 'false'}",
            ]
            if observation["present"]:
                fields.extend(
                    (
                        f"name = {quote(observation['name'])}",
                        f"descriptor = {quote(observation['descriptor'])}",
                        f"shape_sha256 = {quote(observation['shape_sha256'])}",
                    )
                )
            lines.append("  { " + ", ".join(fields) + " },")
        lines.append("]")
    return "\n".join(lines) + "\n"


def write_new(path: Path, content: str) -> None:
    if path.exists():
        raise AdmissionError(f"refusing to overwrite existing generated review file {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", type=Path)
    parser.add_argument("--dry-run", action="store_true")
    arguments = parser.parse_args()
    try:
        plan = load_plan(arguments.plan)
        manifest = corpus.load_manifest()
        baseline = manifest["baseline"]
        builds = plan.get("builds") or [build.build_id for build in corpus.builds()]
        live = collect_live(plan["owner"], builds, "signature")
        wanted = {
            f"{body['original_name']}:{body['descriptor']}" for body in plan["body"]
        }
        missing = sorted(wanted - set(live))
        if missing:
            raise AdmissionError(f"planned bodies are absent from original class: {missing}")
        with (ROOT / "game.toml").open("rb") as handle:
            game = tomllib.load(handle)
        owners = game.get("java", {}).get("baseline_classes", [])
        classes = {}
        baseline_payload = next(
            build.payload for build in corpus.builds() if build.build_id == baseline
        )
        import classfile

        for member, data in corpus.jar_members(baseline_payload):
            if member.endswith(".class"):
                info = classfile.parse_class(member, data)
                classes[info.internal_name] = info
        total_bodies = sum(len(classes[owner].methods) for owner in owners)
        crosswalk = render_crosswalk(plan, baseline, total_bodies)
        variants = render_variants(plan, list(builds), live)
        if arguments.dry_run:
            print(
                json.dumps(
                    {
                        plan["crosswalk_manifest"]: crosswalk,
                        plan["variant_manifest"]: variants,
                    },
                    indent=2,
                    ensure_ascii=False,
                    sort_keys=True,
                )
            )
            return 0
        write_new(ROOT / plan["crosswalk_manifest"], crosswalk)
        write_new(ROOT / plan["variant_manifest"], variants)
        print(
            f"admission scaffold OK: {plan['id']} ({len(plan['body'])} reviewed bodies, "
            f"{len(live)} owner methods)"
        )
        return 0
    except (AdmissionError, OSError, StopIteration, corpus.CorpusError) as error:
        print(f"admission scaffold FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
