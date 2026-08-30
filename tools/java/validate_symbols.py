#!/usr/bin/env python3
"""Validate the bytecode-bound semantic naming ledger.

The original tuple ``(owner, name, descriptor)`` remains identity. Canonical
names are reviewed behavior labels, never claims about lost source spelling.
For every class marked ``coverage = "complete"``, this gate requires an exact
field and method closure against the selected original class bytes, verifies
every method fingerprint, and ratchets the named counts against the full
baseline denominator.
"""

from __future__ import annotations

import copy
import re
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore

ROOT = Path(__file__).resolve().parents[2]
LEDGER = ROOT / "java" / "reconstruction" / "symbols.toml"
with (ROOT / "game.toml").open("rb") as handle:
    GAME_CONFIG = tomllib.load(handle)
sys.path.insert(0, str(ROOT / "tools" / "corpus"))

import classfile  # noqa: E402
import corpus  # noqa: E402

IDENTIFIER = re.compile(r"^[A-Za-z_$][A-Za-z0-9_$]*$")
PLACEHOLDER = re.compile(r"^(?:f\d+[A-Za-z_$]?|m\d+[A-Za-z_$]?)$")


class SymbolError(RuntimeError):
    pass


def load_ledger() -> dict:
    with LEDGER.open("rb") as handle:
        return tomllib.load(handle)


def baseline_classes() -> dict[str, classfile.ClassInfo]:
    baseline = corpus.load_manifest()["baseline"]
    payload = next(build.payload for build in corpus.builds() if build.build_id == baseline)
    result = {}
    for member, data in corpus.jar_members(payload):
        if member.endswith(".class"):
            info = classfile.parse_class(member, data)
            result[info.internal_name] = info
    return result


def parameter_count(descriptor: str) -> int:
    if not descriptor.startswith("(") or ")" not in descriptor:
        raise SymbolError(f"invalid method descriptor: {descriptor}")
    index = 1
    count = 0
    while descriptor[index] != ")":
        while descriptor[index] == "[":
            index += 1
        if descriptor[index] == "L":
            end = descriptor.find(";", index)
            if end < 0:
                raise SymbolError(f"invalid object descriptor: {descriptor}")
            index = end + 1
        elif descriptor[index] in "BCDFIJSZ":
            index += 1
        else:
            raise SymbolError(f"invalid parameter descriptor: {descriptor}")
        count += 1
    return count


def semantic_name(name: str, context: str) -> None:
    if not IDENTIFIER.match(name):
        raise SymbolError(f"{context}: not a Java identifier: {name!r}")
    if len(name) == 1 or PLACEHOLDER.match(name):
        raise SymbolError(f"{context}: placeholder/obfuscated name: {name!r}")


def unique_rows(rows: list[dict], kind: str) -> dict[tuple[str, str, str], dict]:
    result = {}
    for row in rows:
        key = (row["owner"], row["original"], row["descriptor"])
        if key in result:
            raise SymbolError(f"duplicate {kind} identity: {key}")
        if not row.get("evidence", "").strip():
            raise SymbolError(f"{kind} {key}: missing naming evidence")
        result[key] = row
    return result


def require_baseline_closure(document: dict, expected: list[str]) -> None:
    configured = document.get("baseline_classes", [])
    if configured != expected or len(configured) != len(set(configured)):
        raise SymbolError(
            "symbols.toml baseline_classes must exactly equal game.toml "
            "[java].baseline_classes"
        )


def validate(document: dict) -> str:
    if document.get("schema_version") != 1:
        raise SymbolError("symbols.toml schema_version must be 1")
    require_baseline_closure(
        document, GAME_CONFIG.get("java", {}).get("baseline_classes", [])
    )
    original = baseline_classes()
    class_rows = document.get("class", [])
    classes = {row["original"]: row for row in class_rows}
    if len(classes) != len(class_rows):
        raise SymbolError("duplicate class owner in naming ledger")
    fields = unique_rows(document.get("field", []), "field")
    methods = unique_rows(document.get("method", []), "method")

    configured = set(document.get("baseline_classes", []))
    if not configured or not configured.issubset(original):
        raise SymbolError("ledger baseline class closure is empty or absent from bytecode")
    if not set(classes).issubset(configured):
        raise SymbolError("[[class]] row names an owner outside baseline_classes")

    all_fields = sum(len(original[name].fields) for name in configured)
    all_methods = sum(len(original[name].methods) for name in configured)
    coverage = document.get("coverage", {})
    if coverage.get("baseline_fields") != all_fields:
        raise SymbolError(f"baseline field denominator drift: {all_fields}")
    if coverage.get("baseline_methods") != all_methods:
        raise SymbolError(f"baseline method denominator drift: {all_methods}")
    if coverage.get("named_fields") != len(fields):
        raise SymbolError("named_fields ratchet differs from ledger rows")
    if coverage.get("named_methods") != len(methods):
        raise SymbolError("named_methods ratchet differs from ledger rows")
    if not fields or not methods:
        raise SymbolError("naming ledger is vacuous")

    for owner, class_row in classes.items():
        info = original[owner]
        source = ROOT / class_row["source"]
        if not source.is_file():
            raise SymbolError(f"{owner}: canonical source is missing: {source}")
        source_text = source.read_text(encoding="utf-8")
        if (class_row.get("coverage") == "complete"
                and "JADX INFO: renamed from:" in source_text):
            raise SymbolError(f"{owner}: decompiler rename marker remains in complete source")
        semantic_name(class_row["canonical"], f"class {owner}")

        actual_fields = {(owner, row.name, row.descriptor) for row in info.fields}
        actual_methods = {(owner, row.name, row.descriptor) for row in info.methods}
        named_fields = {key for key in fields if key[0] == owner}
        named_methods = {key for key in methods if key[0] == owner}
        if class_row.get("coverage") == "complete":
            if named_fields != actual_fields:
                raise SymbolError(f"{owner}: field naming closure differs from bytecode")
            if named_methods != actual_methods:
                raise SymbolError(f"{owner}: method naming closure differs from bytecode")
        elif not named_fields.issubset(actual_fields) or not named_methods.issubset(actual_methods):
            raise SymbolError(f"{owner}: ledger names a member absent from bytecode")

        field_by_key = {(row.name, row.descriptor): row for row in info.fields}
        method_by_key = {(row.name, row.descriptor): row for row in info.methods}
        for key in sorted(named_fields):
            row = fields[key]
            semantic_name(row["canonical"], f"field {key}")
            if row["canonical"] not in source_text:
                raise SymbolError(f"field {key}: canonical name absent from source")
            if (key[1], key[2]) not in field_by_key:
                raise SymbolError(f"field {key}: absent from original class")
        for key in sorted(named_methods):
            row = methods[key]
            if row["canonical"] not in {"<init>", "<clinit>"}:
                semantic_name(row["canonical"], f"method {key}")
                if row["canonical"] not in source_text:
                    raise SymbolError(f"method {key}: canonical name absent from source")
            parameters = row.get("parameters", [])
            if len(parameters) != parameter_count(key[2]):
                raise SymbolError(f"method {key}: parameter-name count differs from descriptor")
            for name in parameters + row.get("locals", []):
                semantic_name(name, f"method {key} local")
                if name not in source_text:
                    raise SymbolError(f"method {key}: local {name!r} absent from source")
            symbol = method_by_key[(key[1], key[2])]
            for digest in ("code_sha256", "opcode_sha256", "shape_sha256"):
                if row.get(digest) != getattr(symbol, digest):
                    raise SymbolError(f"method {key}: {digest} differs from bytecode")

    complete = sum(1 for row in classes.values() if row.get("coverage") == "complete")
    if coverage.get("complete_classes") != complete:
        raise SymbolError("complete_classes ratchet differs from class rows")
    return (
        f"symbols OK: {len(fields)}/{all_fields} fields and "
        f"{len(methods)}/{all_methods} methods semantically named; "
        f"{complete}/{len(configured)} classes have exact member closure"
    )


def self_test() -> int:
    clean = load_ledger()
    validate(clean)
    missing = copy.deepcopy(clean)
    missing["method"].pop()
    wrong_hash = copy.deepcopy(clean)
    wrong_hash["method"][0]["code_sha256"] = "0" * 64
    caught = 0
    for dirty in (missing, wrong_hash):
        try:
            validate(dirty)
        except SymbolError:
            caught += 1
    if caught != 2:
        print("symbols self-test FAIL: a ledger perturbation was accepted", file=sys.stderr)
        return 1
    print("symbols self-test OK: a removed row and changed bytecode hash both go red")
    return 0


def main(argv: list[str]) -> int:
    try:
        if argv == ["--self-test"]:
            return self_test()
        print(validate(load_ledger()))
        return 0
    except (KeyError, OSError, SymbolError) as error:
        print(f"symbols FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
