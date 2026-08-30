"""Shared schema and discovery helpers for per-class admission plans."""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

PLAN_ID = re.compile(r"[a-z][a-z0-9_]*\Z")


class AdmissionError(RuntimeError):
    pass


def relative_path(value: object, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise AdmissionError(f"{field} must be a non-empty relative path")
    path = Path(value)
    if path.is_absolute() or ".." in path.parts or path == Path("."):
        raise AdmissionError(f"{field} must stay inside the game repository")
    return path.as_posix()


def command(value: object, field: str) -> tuple[str, ...]:
    if not isinstance(value, list) or not value or not all(
        isinstance(part, str) and part for part in value
    ):
        raise AdmissionError(f"{field} must be a non-empty string array")
    return tuple(value)


def load_plan(path: Path) -> dict:
    try:
        plan = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise AdmissionError(f"cannot read admission plan {path}: {error}") from error
    if plan.get("schema_version") != 1:
        raise AdmissionError("admission plan schema_version must be 1")
    plan_id = plan.get("id")
    if not isinstance(plan_id, str) or PLAN_ID.fullmatch(plan_id) is None:
        raise AdmissionError("admission id must be a lowercase snake_case token")
    for field in ("label", "owner", "java_owner"):
        if not isinstance(plan.get(field), str) or not plan[field]:
            raise AdmissionError(f"{field} must be a non-empty string")
    for field in ("java_source", "crosswalk_manifest", "variant_manifest"):
        plan[field] = relative_path(plan.get(field), field)

    builds = plan.get("builds")
    if builds is not None and (
        not isinstance(builds, list)
        or not builds
        or len(builds) != len(set(builds))
        or not all(isinstance(build, str) and build for build in builds)
    ):
        raise AdmissionError("builds must be a non-empty unique string array")

    bodies = plan.get("body")
    if not isinstance(bodies, list) or not bodies:
        raise AdmissionError("an admission plan needs at least one [[body]]")
    keys = []
    for index, body in enumerate(bodies):
        if not isinstance(body, dict):
            raise AdmissionError(f"body {index} must be a table")
        for field in ("original_name", "descriptor", "java_item"):
            if not isinstance(body.get(field), str) or not body[field]:
                raise AdmissionError(f"body {index}.{field} must be a non-empty string")
        key = f"{body['original_name']}:{body['descriptor']}"
        keys.append(key)
        targets = body.get("rust")
        if not isinstance(targets, list) or not targets:
            raise AdmissionError(f"body {index} needs at least one Rust target")
        for target_index, target in enumerate(targets):
            if not isinstance(target, dict):
                raise AdmissionError(f"body {index}.rust {target_index} must be a table")
            target["file"] = relative_path(
                target.get("file"), f"body {index}.rust {target_index}.file"
            )
            if not isinstance(target.get("item"), str) or not target["item"]:
                raise AdmissionError(
                    f"body {index}.rust {target_index}.item must be non-empty"
                )
    if len(keys) != len(set(keys)):
        raise AdmissionError("admission body owner/signature keys must be unique")

    oracle = plan.get("oracle")
    if oracle is not None:
        if not isinstance(oracle, dict):
            raise AdmissionError("oracle must be a table")
        oracle["command"] = command(oracle.get("command"), "oracle.command")
        oracle["canfail_command"] = command(
            oracle.get("canfail_command"), "oracle.canfail_command"
        )
    plan["_path"] = path
    return plan


def discover_plans(root: Path) -> list[Path]:
    directory = root / "java" / "reconstruction" / "admissions"
    return sorted(directory.glob("*.toml")) if directory.is_dir() else []


def require_complete_class_plan_closure(root: Path, paths: list[Path]) -> None:
    symbols_path = root / "java" / "reconstruction" / "symbols.toml"
    try:
        symbols = tomllib.loads(symbols_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise AdmissionError(f"cannot read naming ledger {symbols_path}: {error}") from error
    expected_count = symbols.get("coverage", {}).get("complete_classes")
    if not isinstance(expected_count, int) or expected_count < 0:
        raise AdmissionError("symbols coverage.complete_classes must be a nonnegative integer")
    expected_owners = {
        row.get("original")
        for row in symbols.get("class", [])
        if row.get("coverage") == "complete"
    }
    expected_owners.discard(None)
    if len(expected_owners) != expected_count:
        raise AdmissionError(
            "symbols complete-class rows do not match coverage.complete_classes"
        )
    plans = [load_plan(path) for path in paths]
    actual_owners = [plan["owner"] for plan in plans]
    if len(actual_owners) != len(set(actual_owners)):
        raise AdmissionError("each complete original class must own exactly one admission plan")
    if len(plans) != expected_count or set(actual_owners) != expected_owners:
        raise AdmissionError(
            "admission plans do not exactly close symbols complete classes: "
            f"expected {sorted(expected_owners)}, found {sorted(actual_owners)}"
        )


def original_key(plan: dict, body: dict) -> str:
    return f"{plan['owner']}.{body['original_name']}:{body['descriptor']}"
