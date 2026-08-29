#!/usr/bin/env python3
"""Reusable line-protocol differential oracle for J2ME ports.

Each implementation is a process that reads one case per stdin line and writes
exactly one observation per stdout line. A game-specific wrapper owns case
generation, compilation, authority selection, and reviewed variant exclusions;
this module owns process execution, cardinality checks, comparison, diagnostics,
and the mandatory injected-mismatch proof.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import tomllib
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ProcessSpec:
    label: str
    command: tuple[str, ...]
    cwd: Path | None = None
    env: Mapping[str, str] | None = None


@dataclass(frozen=True)
class Mismatch:
    index: int
    case: str
    reference_label: str
    reference: str
    candidate_label: str
    candidate: str


@dataclass(frozen=True)
class OracleReport:
    case_count: int
    labels: tuple[str, ...]
    mismatches: tuple[Mismatch, ...]


AllowedMismatch = Callable[[int, str, str, str, str, str], bool]


class OracleError(RuntimeError):
    pass


def run_process(spec: ProcessSpec, cases: Sequence[str]) -> list[str]:
    if not spec.command:
        raise OracleError(f"{spec.label}: empty command")
    environment = os.environ.copy()
    if spec.env:
        environment.update(spec.env)
    completed = subprocess.run(
        spec.command,
        cwd=spec.cwd,
        env=environment,
        input="\n".join(cases) + "\n",
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        rendered = " ".join(spec.command)
        raise OracleError(
            f"{spec.label}: command failed with {completed.returncode}: {rendered}\n"
            f"{completed.stderr}"
        )
    return completed.stdout.splitlines()


def compare_outputs(
    cases: Sequence[str],
    outputs: Mapping[str, Sequence[str]],
    *,
    reference_label: str,
    allowed_mismatch: AllowedMismatch | None = None,
) -> OracleReport:
    if not cases:
        raise OracleError("oracle case set is empty")
    if reference_label not in outputs:
        raise OracleError(f"reference implementation is absent: {reference_label}")
    if len(outputs) < 2:
        raise OracleError("an oracle requires at least two implementations")

    lengths = {label: len(values) for label, values in outputs.items()}
    wrong = {label: length for label, length in lengths.items() if length != len(cases)}
    if wrong:
        raise OracleError(
            f"output cardinality mismatch: cases={len(cases)}, outputs={wrong}"
        )

    reference = outputs[reference_label]
    mismatches: list[Mismatch] = []
    for candidate_label, candidate in outputs.items():
        if candidate_label == reference_label:
            continue
        for index, (case, left, right) in enumerate(
            zip(cases, reference, candidate, strict=True)
        ):
            if left == right:
                continue
            if allowed_mismatch and allowed_mismatch(
                index,
                case,
                reference_label,
                left,
                candidate_label,
                right,
            ):
                continue
            mismatches.append(
                Mismatch(
                    index=index,
                    case=case,
                    reference_label=reference_label,
                    reference=left,
                    candidate_label=candidate_label,
                    candidate=right,
                )
            )
    return OracleReport(
        case_count=len(cases),
        labels=tuple(outputs),
        mismatches=tuple(mismatches),
    )


def run_oracle(
    cases: Sequence[str],
    implementations: Sequence[ProcessSpec],
    *,
    reference_label: str,
    allowed_mismatch: AllowedMismatch | None = None,
    self_test: bool = False,
) -> OracleReport:
    labels = [implementation.label for implementation in implementations]
    if len(labels) != len(set(labels)):
        raise OracleError(f"implementation labels are not unique: {labels}")
    outputs = {
        implementation.label: run_process(implementation, cases)
        for implementation in implementations
    }
    report = compare_outputs(
        cases,
        outputs,
        reference_label=reference_label,
        allowed_mismatch=allowed_mismatch,
    )
    if report.mismatches:
        return report
    if not self_test:
        return report

    candidate_label = next(label for label in reversed(labels) if label != reference_label)
    injected_index = len(cases) // 2
    mutated = {label: list(values) for label, values in outputs.items()}
    mutated[candidate_label][injected_index] = "__J2ME_ORACLE_INJECTED_MISMATCH__"
    can_fail = compare_outputs(
        cases,
        mutated,
        reference_label=reference_label,
        allowed_mismatch=allowed_mismatch,
    )
    if len(can_fail.mismatches) != 1:
        raise OracleError(
            "oracle self-test expected exactly one mismatch, found "
            f"{len(can_fail.mismatches)}"
        )
    mismatch = can_fail.mismatches[0]
    if mismatch.index != injected_index or mismatch.candidate_label != candidate_label:
        raise OracleError("oracle self-test rejected the wrong observation")
    return report


def _load_config(path: Path) -> tuple[list[str], list[ProcessSpec], str]:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != 1:
        raise OracleError("oracle config schema_version must be 1")
    base = path.parent
    cases_path = base / data["cases"]
    cases = [
        line
        for raw in cases_path.read_text(encoding="utf-8").splitlines()
        if (line := raw.strip()) and not line.startswith("#")
    ]
    expected = data.get("expected_case_count")
    if expected is not None and expected != len(cases):
        raise OracleError(
            f"case ratchet mismatch: expected {expected}, loaded {len(cases)}"
        )
    implementations = [
        ProcessSpec(
            label=row["label"],
            command=tuple(row["command"]),
            cwd=(base / row["cwd"]).resolve() if row.get("cwd") else base,
            env=row.get("env"),
        )
        for row in data.get("implementation", [])
    ]
    return cases, implementations, data["reference"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", type=Path)
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args()
    try:
        cases, implementations, reference = _load_config(arguments.config.resolve())
        report = run_oracle(
            cases,
            implementations,
            reference_label=reference,
            self_test=arguments.self_test,
        )
    except OracleError as error:
        parser.error(str(error))
    if report.mismatches:
        for mismatch in report.mismatches[:12]:
            print(
                f"case {mismatch.index} {mismatch.case}: "
                f"{mismatch.reference_label}={mismatch.reference!r}, "
                f"{mismatch.candidate_label}={mismatch.candidate!r}"
            )
        return 1
    suffix = "; injected mismatch rejected" if arguments.self_test else ""
    print(
        f"line oracle OK: {report.case_count} cases across "
        f"{', '.join(report.labels)}{suffix}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
