#!/usr/bin/env python3
"""Optional linked-binary production reachability and oracle-leak gate.

This is intentionally an opt-in Linux/ELF profile. Source inventories prove
ownership; this gate additionally proves that production targets are connected
to configured executable roots and oracle-only targets are not. Parser floors
are project ratchets: a thinner ``readelf``/``objdump`` parse fails instead of
silently producing a smaller, easier graph.
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Sequence

ROOT = Path(__file__).resolve().parents[2]
FUNCTION_LABEL = re.compile(r"^([0-9a-f]+) <(.*)>:$")
SYMBOL_REFERENCE = re.compile(r"<(.*)>")
COMMENT_ADDRESS = re.compile(r"#\s*([0-9a-f]+)\s*<")
FUNCTION_OFFSET = re.compile(r"[+-]0x[0-9a-f]+$")


class ReachabilityError(RuntimeError):
    pass


def transitive_closure(graph: dict[str, set[str]], roots: Sequence[str]) -> set[str]:
    reached: set[str] = set()
    pending = list(roots)
    while pending:
        current = pending.pop()
        if current in reached:
            continue
        reached.add(current)
        pending.extend(graph.get(current, ()))
    return reached


def matching_symbols(symbols: set[str], pattern: str, mode: str) -> set[str]:
    if mode == "exact":
        return {symbol for symbol in symbols if symbol == pattern}
    if mode == "prefix":
        return {symbol for symbol in symbols if symbol.startswith(pattern)}
    if mode == "contains":
        return {symbol for symbol in symbols if pattern in symbol}
    raise ReachabilityError(f"unknown symbol match mode {mode!r}")


def validate_expectations(
    graph: dict[str, set[str]], roots: Sequence[str], targets: Sequence[dict], expected_count: int
) -> list[str]:
    errors: list[str] = []
    unknown_roots = sorted(set(roots) - set(graph))
    if unknown_roots:
        errors.append(f"unknown linked production roots: {unknown_roots}")
    reached = transitive_closure(graph, roots)
    if len(reached) != expected_count:
        errors.append(
            f"reachable-symbol ratchet is {expected_count}, linked closure is {len(reached)}"
        )
    seen: set[tuple[str, str]] = set()
    for row in targets:
        pattern = row.get("symbol", "")
        mode = row.get("match", "exact")
        key = (mode, pattern)
        if key in seen:
            errors.append(f"duplicate native target {key}")
        seen.add(key)
        reason = row.get("reason")
        if not isinstance(reason, str) or not reason.strip():
            errors.append(f"native target {key} needs a reason")
        matches = matching_symbols(set(graph), pattern, mode)
        if not matches:
            errors.append(f"native target {key} matches no linked symbol")
            continue
        reached_matches = matches & reached
        expectation = row.get("expect")
        category = row.get("category")
        if expectation == "reachable" and not reached_matches:
            errors.append(f"production native target {key} is not reachable")
        elif expectation == "unreachable" and reached_matches:
            errors.append(f"oracle/dead native target {key} leaked into production closure")
        elif expectation not in {"reachable", "unreachable"}:
            errors.append(f"native target {key} has invalid expectation {expectation!r}")
        if category == "oracle-only" and expectation != "unreachable":
            errors.append(f"oracle-only native target {key} must be unreachable")
        if category == "production" and expectation != "reachable":
            errors.append(f"production native target {key} must be reachable")
        if category not in {"production", "oracle-only", "retail-dead", "compiler-indirect"}:
            errors.append(f"native target {key} has invalid category {category!r}")
    return errors


def relative_relocations(
    binary: Path, relocation_type: str, minimum: int
) -> dict[int, int]:
    output = subprocess.run(
        ["readelf", "-rW", str(binary)],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    result: dict[int, int] = {}
    typed = 0
    malformed: list[str] = []
    for line in output.splitlines():
        columns = line.split()
        if len(columns) < 3 or columns[2] != relocation_type:
            continue
        typed += 1
        try:
            result[int(columns[0], 16)] = int(columns[3], 16)
        except (IndexError, ValueError):
            if len(malformed) < 5:
                malformed.append(line.strip())
    if malformed:
        raise ReachabilityError(
            f"readelf found {typed} {relocation_type} rows but could not parse "
            f"{len(malformed)}; first rows: {malformed}"
        )
    if len(result) < minimum:
        raise ReachabilityError(
            f"readelf parsed {len(result)} {relocation_type} relocations, floor is {minimum}"
        )
    return result


def binary_graph(
    binary: Path,
    *,
    relocation_type: str,
    minimum_relocations: int,
    minimum_functions: int,
    minimum_resolved_edges: int,
) -> dict[str, set[str]]:
    relocations = relative_relocations(binary, relocation_type, minimum_relocations)
    graph: dict[str, set[str]] = {}
    address_to_function: dict[int, str] = {}
    address_references: list[tuple[str, int]] = []
    current: str | None = None

    process = subprocess.Popen(
        ["objdump", "-Cd", str(binary)],
        stdout=subprocess.PIPE,
        text=True,
        errors="replace",
    )
    assert process.stdout is not None
    for line in process.stdout:
        label = FUNCTION_LABEL.match(line)
        if label:
            current = label.group(2)
            graph.setdefault(current, set())
            address_to_function[int(label.group(1), 16)] = current
            continue
        if current is None:
            continue
        reference = SYMBOL_REFERENCE.search(line)
        if reference:
            target = FUNCTION_OFFSET.sub("", reference.group(1))
            if target != current:
                graph[current].add(target)
        address = COMMENT_ADDRESS.search(line)
        if address:
            address_references.append((current, int(address.group(1), 16)))
    if process.wait() != 0:
        raise ReachabilityError("objdump failed")
    if len(address_to_function) < minimum_functions:
        raise ReachabilityError(
            f"objdump parsed {len(address_to_function)} functions, floor is {minimum_functions}"
        )

    resolved = 0
    for caller, referenced_address in address_references:
        target_address = relocations.get(referenced_address)
        target = address_to_function.get(target_address) if target_address is not None else None
        if target is not None:
            graph[caller].add(target)
            resolved += 1
    if resolved < minimum_resolved_edges:
        raise ReachabilityError(
            f"resolved {resolved} GOT edges, floor is {minimum_resolved_edges}"
        )
    return graph


def _declaration_key(row: dict) -> tuple[str, str]:
    path = Path(row.get("file", ""))
    if not path.is_absolute():
        path = ROOT / path
    try:
        file = path.resolve().relative_to(ROOT).as_posix()
    except ValueError:
        file = str(path.resolve())
    return file, str(row.get("item", ""))


def validate_manifest_links(manifest: dict, targets: Sequence[dict]) -> list[str]:
    errors: list[str] = []
    declarations = {
        _declaration_key(row): row for row in manifest.get("rust_declaration", [])
    }
    for row in targets:
        key = _declaration_key(row)
        declaration = declarations.get(key)
        if declaration is None:
            errors.append(f"native target {key} lacks reverse Rust ownership")
            continue
        if row.get("category") == "oracle-only" and declaration.get(
            "classification"
        ) != "oracle-infrastructure":
            errors.append(
                f"native oracle target {key} is not classified oracle-infrastructure"
            )
    return errors


def self_test() -> int:
    graph = {
        "game::main": {"game::tick"},
        "game::tick": {"runtime::paint"},
        "runtime::paint": set(),
        "oracle::fixture": set(),
    }
    targets = [
        {
            "symbol": "game::tick",
            "expect": "reachable",
            "category": "production",
            "reason": "frame callback",
        },
        {
            "symbol": "oracle::fixture",
            "expect": "unreachable",
            "category": "oracle-only",
            "reason": "differential harness only",
        },
    ]
    clean = validate_expectations(graph, ["game::main"], targets, 3)
    if clean:
        raise ReachabilityError(f"clean synthetic graph is not green: {clean}")
    graph["game::tick"].add("oracle::fixture")
    broken = validate_expectations(graph, ["game::main"], targets, 4)
    if not any("leaked into production" in error for error in broken):
        raise ReachabilityError(f"injected oracle edge did not turn red: {broken}")
    print("native reachability self-test OK: one oracle edge goes red")
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", nargs="?", type=Path)
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args(argv)
    try:
        if arguments.self_test:
            return self_test()
        if arguments.manifest is None:
            parser.error("MANIFEST is required unless --self-test is used")
        manifest = tomllib.loads(arguments.manifest.read_text(encoding="utf-8"))
        native = manifest.get("native", {})
        if native.get("enabled") is not True:
            raise ReachabilityError(
                "native reachability is not enabled; do not add this command as a green skip"
            )
        for tool in ("objdump", "readelf"):
            if shutil.which(tool) is None:
                raise ReachabilityError(f"required binary tool is missing: {tool}")
        command = native.get("build_command", [])
        if not command or not all(isinstance(value, str) and value for value in command):
            raise ReachabilityError("native.build_command must be a non-empty argv array")
        subprocess.run(command, cwd=ROOT, check=True)
        binary = ROOT / native.get("binary", "")
        if not binary.is_file():
            raise ReachabilityError(f"native binary is absent: {binary}")
        floors = native.get("floor", {})
        for name in ("relocations", "functions", "resolved_edges"):
            if not isinstance(floors.get(name), int) or floors[name] <= 0:
                raise ReachabilityError(f"native.floor.{name} must be positive")
        graph = binary_graph(
            binary,
            relocation_type=native.get("relocation_type", "R_X86_64_RELATIVE"),
            minimum_relocations=floors["relocations"],
            minimum_functions=floors["functions"],
            minimum_resolved_edges=floors["resolved_edges"],
        )
        targets = native.get("target", [])
        errors = validate_manifest_links(manifest, targets)
        expected = native.get("reachable_symbol_count")
        if not isinstance(expected, int) or expected <= 0:
            errors.append("native.reachable_symbol_count must be positive")
            expected = -1
        errors.extend(validate_expectations(graph, native.get("roots", []), targets, expected))
        if errors:
            print("\n".join(errors), file=sys.stderr)
            return 1
        print(
            f"native reachability OK: {expected} linked symbols reachable; "
            f"{len(targets)} classified targets"
        )
        return 0
    except (OSError, ReachabilityError, subprocess.CalledProcessError) as error:
        print(f"native reachability FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
