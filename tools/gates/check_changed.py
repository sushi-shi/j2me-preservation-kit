#!/usr/bin/env python3
"""Run only gate groups whose content-addressed inputs changed.

The router is deliberately independent of Git.  A newly stamped or heavily
dirty port gets the same answer as a clean checkout: every configured input is
hashed, and a gate is clean only when that exact fingerprint previously passed.
"""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import shlex
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


SCHEMA_VERSION = 1


@dataclass(frozen=True)
class Gate:
    name: str
    description: str
    inputs: tuple[str, ...]
    commands: tuple[tuple[str, ...], ...]


@dataclass(frozen=True)
class Configuration:
    path: Path
    root: Path
    state_path: Path
    gates: tuple[Gate, ...]


def parse_configuration(path: Path) -> Configuration:
    path = path.resolve()
    with path.open("rb") as handle:
        document = tomllib.load(handle)
    if document.get("schema_version") != SCHEMA_VERSION:
        raise ValueError(
            f"{path}: schema_version must be {SCHEMA_VERSION}"
        )
    root = (path.parent / document.get("project_root", "../..")).resolve()
    state_path = root / document.get(
        "state_file", "target/gates/affected-state.json"
    )
    gates: list[Gate] = []
    names: set[str] = set()
    for raw in document.get("gate", []):
        name = raw.get("name", "")
        if not name or name in names:
            raise ValueError(f"{path}: gate names must be nonempty and unique: {name!r}")
        names.add(name)
        inputs = tuple(raw.get("inputs", []))
        commands = tuple(tuple(command) for command in raw.get("commands", []))
        if not inputs:
            raise ValueError(f"{path}: gate {name!r} has no inputs")
        if not commands or any(not command for command in commands):
            raise ValueError(f"{path}: gate {name!r} has no executable commands")
        if any(not isinstance(part, str) for command in commands for part in command):
            raise ValueError(f"{path}: gate {name!r} command arguments must be strings")
        gates.append(
            Gate(
                name=name,
                description=raw.get("description", ""),
                inputs=inputs,
                commands=commands,
            )
        )
    if not gates:
        raise ValueError(f"{path}: at least one [[gate]] is required")
    return Configuration(path, root, state_path, tuple(gates))


def relative_path(root: Path, path: Path) -> str:
    try:
        return path.relative_to(root).as_posix()
    except ValueError as error:
        raise ValueError(f"gate input escapes project root: {path}") from error


def expand_pattern(root: Path, pattern: str) -> tuple[list[Path], bool]:
    candidate = Path(pattern)
    if candidate.is_absolute() or ".." in candidate.parts:
        raise ValueError(f"gate input must be a project-relative path/glob: {pattern}")
    has_magic = any(character in pattern for character in "*?[")
    matches = list(root.glob(pattern)) if has_magic else [root / candidate]
    files: set[Path] = set()
    existed = False
    for match in matches:
        if not match.exists() and not match.is_symlink():
            continue
        existed = True
        if match.is_dir() and not match.is_symlink():
            files.update(
                child
                for child in match.rglob("*")
                if child.is_file() or child.is_symlink()
            )
        else:
            files.add(match)
    return sorted(files), existed


def sha256_file(path: Path) -> str:
    if path.is_symlink():
        return "symlink:" + hashlib.sha256(os.readlink(path).encode()).hexdigest()
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def gate_file_hashes(
    configuration: Configuration,
    gate: Gate,
    digest_cache: dict[Path, str],
) -> dict[str, str]:
    root = configuration.root
    automatic = (
        configuration.path,
        Path(__file__).resolve(),
        root / "Justfile",
    )
    result: dict[str, str] = {}
    for path in automatic:
        if path.exists() or path.is_symlink():
            digest_cache.setdefault(path, sha256_file(path))
            result[relative_path(root, path)] = digest_cache[path]
    for pattern in gate.inputs:
        files, existed = expand_pattern(root, pattern)
        if not existed:
            result[f"<missing:{pattern}>"] = "missing"
        for path in files:
            resolved = path.resolve() if not path.is_symlink() else path.absolute()
            if configuration.state_path == resolved:
                continue
            digest_cache.setdefault(path, sha256_file(path))
            result[relative_path(root, path)] = digest_cache[path]
    return dict(sorted(result.items()))


def gate_fingerprint(gate: Gate, files: dict[str, str]) -> str:
    payload = {
        "schema_version": SCHEMA_VERSION,
        "name": gate.name,
        "inputs": gate.inputs,
        "commands": gate.commands,
        "files": files,
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def load_state(path: Path) -> dict:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {"schema_version": SCHEMA_VERSION, "gates": {}}
    except (json.JSONDecodeError, OSError) as error:
        raise ValueError(f"cannot read gate state {path}: {error}") from error
    if document.get("schema_version") != SCHEMA_VERSION:
        return {"schema_version": SCHEMA_VERSION, "gates": {}}
    if not isinstance(document.get("gates"), dict):
        raise ValueError(f"invalid gate state {path}: gates must be an object")
    return document


def save_state(path: Path, state: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(state, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )
    os.replace(temporary, path)


def change_summary(previous: dict[str, str] | None, current: dict[str, str]) -> str:
    if previous is None:
        return "no prior successful fingerprint"
    previous_keys = set(previous)
    current_keys = set(current)
    added = sorted(current_keys - previous_keys)
    removed = sorted(previous_keys - current_keys)
    modified = sorted(
        path for path in previous_keys & current_keys if previous[path] != current[path]
    )
    parts: list[str] = []
    for label, paths in (("added", added), ("modified", modified), ("removed", removed)):
        if paths:
            preview = ", ".join(paths[:4])
            if len(paths) > 4:
                preview += f", +{len(paths) - 4} more"
            parts.append(f"{label}: {preview}")
    return "; ".join(parts) if parts else "gate definition changed"


def selected_gates(configuration: Configuration, only: set[str]) -> tuple[Gate, ...]:
    known = {gate.name for gate in configuration.gates}
    unknown = only - known
    if unknown:
        raise ValueError(f"unknown gate group(s): {', '.join(sorted(unknown))}")
    return tuple(
        gate for gate in configuration.gates if not only or gate.name in only
    )


def run_once(
    configuration: Configuration,
    *,
    force_all: bool,
    dry_run: bool,
    only: set[str],
    record_only: bool,
) -> int:
    state = load_state(configuration.state_path)
    state_gates = state["gates"]
    digest_cache: dict[Path, str] = {}
    ran = 0
    for gate in selected_gates(configuration, only):
        files = gate_file_hashes(configuration, gate, digest_cache)
        fingerprint = gate_fingerprint(gate, files)
        previous = state_gates.get(gate.name)
        clean = previous is not None and previous.get("fingerprint") == fingerprint
        if clean and not force_all:
            print(f"[clean] {gate.name}")
            continue
        reason = "forced full run" if force_all else change_summary(
            previous.get("files") if previous else None, files
        )
        if record_only:
            print(f"[record] {gate.name}: {reason}")
        else:
            print(f"[run] {gate.name}: {reason}")
        if gate.description:
            print(f"      {gate.description}")
        if dry_run:
            for command in gate.commands:
                print(f"      $ {shlex.join(command)}")
            ran += 1
            continue
        if not record_only:
            for command in gate.commands:
                print(f"      $ {shlex.join(command)}", flush=True)
                result = subprocess.run(command, cwd=configuration.root)
                if result.returncode != 0:
                    print(
                        f"gate group {gate.name!r} failed with status "
                        f"{result.returncode}; its fingerprint was not cached",
                        file=sys.stderr,
                    )
                    return result.returncode
        state_gates[gate.name] = {
            "fingerprint": fingerprint,
            "files": files,
            "recorded_at_unix": int(time.time()),
        }
        save_state(configuration.state_path, state)
        ran += 1
    if ran == 0:
        print("affected gates OK: every configured input matches its last successful hash")
    elif dry_run:
        print(f"affected gates dry-run: {ran} group(s) would run")
    elif record_only:
        print(f"affected gate cache synchronized for {ran} group(s)")
    else:
        print(f"affected gates OK: {ran} changed group(s) passed")
    return 0


def quick_snapshot(configuration: Configuration, only: set[str]) -> tuple:
    paths: set[Path] = {
        configuration.path,
        Path(__file__).resolve(),
        configuration.root / "Justfile",
    }
    missing: list[str] = []
    for gate in selected_gates(configuration, only):
        for pattern in gate.inputs:
            files, existed = expand_pattern(configuration.root, pattern)
            paths.update(files)
            if not existed:
                missing.append(pattern)
    facts = []
    for path in sorted(paths):
        try:
            status = path.lstat()
        except FileNotFoundError:
            facts.append((str(path), "missing"))
            continue
        facts.append(
            (
                str(path),
                status.st_mode,
                status.st_size,
                status.st_mtime_ns,
                status.st_ctime_ns,
                os.readlink(path) if path.is_symlink() else "",
            )
        )
    return tuple(facts), tuple(sorted(missing))


def watch(
    configuration: Configuration,
    *,
    interval: float,
    only: set[str],
) -> int:
    result = run_once(
        configuration,
        force_all=False,
        dry_run=False,
        only=only,
        record_only=False,
    )
    if result != 0:
        return result
    snapshot = quick_snapshot(configuration, only)
    print(f"watching gate inputs every {interval:g}s; Ctrl-C stops", flush=True)
    try:
        while True:
            time.sleep(interval)
            current = quick_snapshot(configuration, only)
            if current == snapshot:
                continue
            time.sleep(min(interval, 0.2))
            result = run_once(
                configuration,
                force_all=False,
                dry_run=False,
                only=only,
                record_only=False,
            )
            snapshot = quick_snapshot(configuration, only)
            if result != 0:
                print("watch remains active; fix the failure to retry on the next edit")
    except KeyboardInterrupt:
        print("\nwatch stopped")
    return 0


def lock_state(configuration: Configuration):
    lock_path = configuration.state_path.with_suffix(
        configuration.state_path.suffix + ".lock"
    )
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    handle = lock_path.open("a+")
    try:
        fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError as error:
        handle.close()
        raise RuntimeError("another affected-gate runner is active") from error
    return handle


def argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--config",
        type=Path,
        default=Path("tools/gates/gates.toml"),
        help="gate dependency manifest (default: tools/gates/gates.toml)",
    )
    parser.add_argument("--all", action="store_true", help="force every group")
    parser.add_argument("--dry-run", action="store_true", help="explain without running")
    parser.add_argument(
        "--only", action="append", default=[], metavar="GROUP", help="restrict groups"
    )
    parser.add_argument("--watch", action="store_true", help="rerun on input changes")
    parser.add_argument("--interval", type=float, default=0.5)
    parser.add_argument(
        "--record-all",
        action="store_true",
        help="record current hashes without execution; only for the final line of a successful full check",
    )
    return parser


def main(argv: Iterable[str] | None = None) -> int:
    arguments = argument_parser().parse_args(argv)
    if arguments.interval <= 0:
        raise SystemExit("--interval must be positive")
    if arguments.watch and (arguments.dry_run or arguments.all or arguments.record_all):
        raise SystemExit("--watch cannot be combined with --dry-run, --all, or --record-all")
    if arguments.record_all and arguments.dry_run:
        raise SystemExit("--record-all cannot be combined with --dry-run")
    try:
        configuration = parse_configuration(arguments.config)
        lock_handle = lock_state(configuration)
        with lock_handle:
            only = set(arguments.only)
            if arguments.watch:
                return watch(configuration, interval=arguments.interval, only=only)
            return run_once(
                configuration,
                force_all=arguments.all or arguments.record_all,
                dry_run=arguments.dry_run,
                only=only,
                record_only=arguments.record_all,
            )
    except (OSError, RuntimeError, ValueError) as error:
        print(f"check-changed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
