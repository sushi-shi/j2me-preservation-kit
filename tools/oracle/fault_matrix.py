"""Deterministic failure-case matrices for staged host-boundary oracles.

The line-oracle runner compares processes; this helper supplies the cases that
make resource/error ordering observable. It deliberately knows nothing about a
particular API, game, or output format.
"""

from __future__ import annotations

import re
from collections.abc import Sequence

STAGE = re.compile(r"[a-z][a-z0-9-]*\Z")


def staged_fault_cases(
    stages: Sequence[str],
    suffix: str,
    *,
    cleanup_stage: str | None = None,
    cleanup_after: Sequence[str] = (),
) -> tuple[str, ...]:
    """Return Exception/Throwable cases plus cleanup-override combinations.

    ``suffix`` is opaque input appended to every scenario token. If a cleanup
    stage is supplied, every stage in ``cleanup_after`` also receives the two
    combinations that prove cleanup Throwable overrides ordinary failure while
    cleanup Exception does not replace an existing Throwable.
    """

    ordered = tuple(stages)
    if not ordered or len(ordered) != len(set(ordered)):
        raise ValueError("stages must be a non-empty unique sequence")
    if any(STAGE.fullmatch(stage) is None for stage in ordered):
        raise ValueError("stage names must be lowercase token strings")
    cleanup_after_set = set(cleanup_after)
    if cleanup_stage is None:
        if cleanup_after_set:
            raise ValueError("cleanup_after requires cleanup_stage")
    elif cleanup_stage not in ordered:
        raise ValueError("cleanup_stage must be present in stages")
    if not cleanup_after_set.issubset(ordered):
        raise ValueError("cleanup_after contains an unknown stage")
    if cleanup_stage in cleanup_after_set:
        raise ValueError("cleanup_stage cannot clean up after itself")

    cases = []
    for stage in ordered:
        cases.append(f"{stage}-ex{suffix}")
        cases.append(f"{stage}-throw{suffix}")
    if cleanup_stage is not None:
        for stage in ordered:
            if stage not in cleanup_after_set:
                continue
            cases.append(f"{stage}-ex+{cleanup_stage}-throw{suffix}")
            cases.append(f"{stage}-throw+{cleanup_stage}-ex{suffix}")
    return tuple(cases)
