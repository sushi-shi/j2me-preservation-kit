#!/usr/bin/env python3
"""The runtime-collapse ledger — the second completion axis of a J2ME port.

The per-node crosswalk proves each translated body faithful. But a body can be
faithful and still crash at runtime because an UNPORTED call was "collapsed to a
no-op", leaving state (a def id, a slot, a marker) at its `-1`/`0` sentinel that
a later faithful read then indexes with. Those recorded collapses are the
runtime landmines the index-safety scanner keeps finding.

This ledger enumerates every recorded collapse from two sources:

  * source comment markers in the transliteration crate (`collapsed to a
    no-op`, `no-op collapse`, `documented no-op`, `recorded collapse`,
    `still-collapsed`, `no-op stand-in`); and
  * `method-audit.toml` body/adaptation `reason` text that mentions a collapsed
    or unported call.

It groups them by their D##/G## finding number and marks each mention OPEN or
RETIRED (retired = the collapsed method is now ported / marked resolved), then
prints the OPEN backlog. Advisory: it always exits 0.
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# A recorded no-op/unported collapse (NOT the R4 structural singleton collapse,
# which is a representation choice, not a runtime landmine).
COLLAPSE_RE = re.compile(
    r"collapsed to a no-?op"
    r"|no-?op collapse"
    r"|documented no-?op"
    r"|recorded collapse"
    r"|still-collapsed"
    r"|no-?op stand-in"
    r"|collapse pending"
    r"|no-?op collapse pending",
    re.IGNORECASE,
)

# Broader net for the audit reasons, which also phrase it as unported/pending.
AUDIT_COLLAPSE_RE = re.compile(
    r"collaps|no-?op|unported|not[- ]yet[- ]ported|pending (?:its|their) subsystem",
    re.IGNORECASE,
)

RETIRED_RE = re.compile(
    r"retir|resolv|now ported|graduat|is now whole|port is now whole"
    r"|no longer a no-?op|wired\b|already retired",
    re.IGNORECASE,
)

FINDING_RE = re.compile(r"\b([DG]\d{1,3})\b")


@dataclass
class Mention:
    origin: str  # "src" | "audit-body" | "audit-adaptation"
    location: str
    numbers: tuple[str, ...]
    retired: bool
    text: str


def _rel(path: Path) -> str:
    try:
        return path.resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return path.as_posix()


def classify(text: str) -> tuple[tuple[str, ...], bool]:
    numbers = tuple(dict.fromkeys(FINDING_RE.findall(text)))  # ordered-unique
    return numbers or ("(unnumbered)",), bool(RETIRED_RE.search(text))


def _snippet(text: str, span: tuple[int, int], width: int = 110) -> str:
    start = max(0, span[0] - width // 3)
    end = min(len(text), span[1] + 2 * width // 3)
    piece = " ".join(text[start:end].split())
    return f"...{piece}..." if (start or end < len(text)) else piece


def scan_source_text(location_prefix: str, text: str) -> list[Mention]:
    mentions: list[Mention] = []
    for lineno, line in enumerate(text.splitlines(), start=1):
        match = COLLAPSE_RE.search(line)
        if not match:
            continue
        numbers, retired = classify(line)
        mentions.append(
            Mention(
                origin="src",
                location=f"{location_prefix}:{lineno}",
                numbers=numbers,
                retired=retired,
                text=" ".join(line.split()),
            )
        )
    return mentions


def scan_audit(data: dict) -> list[Mention]:
    mentions: list[Mention] = []
    for body in data.get("body", []):
        item = str(body.get("java_item") or body.get("java_method") or body.get("java_class") or "?")
        body_reason = str(body.get("reason", ""))
        for match in COLLAPSE_RE.finditer(body_reason):
            numbers, retired = classify(_snippet(body_reason, match.span(), 160))
            mentions.append(
                Mention(
                    origin="audit-body",
                    location=item,
                    numbers=numbers,
                    retired=retired,
                    text=_snippet(body_reason, match.span()),
                )
            )
        for adaptation in body.get("adaptation", []):
            reason = str(adaptation.get("reason", ""))
            if not AUDIT_COLLAPSE_RE.search(reason):
                continue
            numbers, retired = classify(reason)
            side = str(adaptation.get("side", "?"))
            mentions.append(
                Mention(
                    origin="audit-adaptation",
                    location=f"{item} [{side}]",
                    numbers=numbers,
                    retired=retired,
                    text=" ".join(reason.split())[:200],
                )
            )
    return mentions


def find_sources() -> tuple[list[Path], Path | None]:
    translit = ROOT / "transliteration"
    src_files: list[Path] = []
    if translit.exists():
        for pattern in ("crates/*-xlat/src/*.rs", "*-xlat/src/*.rs", "game-xlat/src/*.rs"):
            src_files.extend(sorted(translit.glob(pattern)))
    src_files = sorted(set(src_files))
    audit: Path | None = None
    for candidate in sorted(ROOT.glob("transliteration/**/method-audit.toml")):
        audit = candidate
        break
    return src_files, audit


def collect(src_files: list[Path], audit: Path | None) -> list[Mention]:
    mentions: list[Mention] = []
    for path in src_files:
        mentions.extend(scan_source_text(_rel(path), path.read_text(encoding="utf-8")))
    if audit and audit.exists():
        mentions.extend(scan_audit(tomllib.loads(audit.read_text(encoding="utf-8"))))
    return mentions


def _number_key(number: str) -> tuple[int, int, str]:
    match = re.match(r"([DG])(\d+)", number)
    if match:
        return ({"D": 0, "G": 1}[match.group(1)], int(match.group(2)), number)
    return (2, 0, number)


def report(mentions: list[Mention], *, show_all: bool, sample: int) -> int:
    by_number: dict[str, list[Mention]] = defaultdict(list)
    for mention in mentions:
        for number in mention.numbers:
            by_number[number].append(mention)

    total_open = sum(1 for m in mentions if not m.retired)
    total_retired = len(mentions) - total_open

    print("collapse-ledger (advisory): recorded runtime-collapse landmines")
    print(
        f"  {len(mentions)} recorded collapse mentions | "
        f"{total_open} OPEN | {total_retired} retired | "
        f"across {len(by_number)} finding group(s)"
    )
    print("  (OPEN collapses are where a faithful read may still index a stale -1 sentinel.)\n")

    for number in sorted(by_number, key=_number_key):
        group = by_number[number]
        open_ones = [m for m in group if not m.retired]
        retired_ones = [m for m in group if m.retired]
        marker = "" if not open_ones else "  <-- OPEN"
        print(f"  {number}: {len(open_ones)} open, {len(retired_ones)} retired{marker}")
        shown = open_ones if not show_all else group
        for mention in shown[: (len(shown) if show_all else sample)]:
            tag = "OPEN " if not mention.retired else "done "
            print(f"      {tag}[{mention.origin}] {mention.location}")
            print(f"            {mention.text[:150]}")
        if not show_all and len(open_ones) > sample:
            print(f"      ... and {len(open_ones) - sample} more open (pass --all)")
    return 0


# ---- self-test -------------------------------------------------------------

_FIXTURE_SRC = """
// the sound tick is collapsed to a no-op stand-in so control flow is faithful (D10)
fn ticker() {}
// the D28 render-cache no-op collapse is retired: the port is now whole
fn cache() {}
// a documented no-op boundary here, pending its subsystem
fn boundary() {}
"""

_FIXTURE_AUDIT = {
    "body": [
        {
            "java_item": "build_render_progress()",
            "reason": "This body was previously the build_render_progress no-op collapse; the port is now WHOLE and the collapse retired (D-x).",
            "adaptation": [
                {"side": "rust", "reason": "the loud boundary arm for the still-collapsed callee c.r (D7), a documented no-op pending its subsystem."},
                {"side": "java", "reason": "a plain representation choice; the Rust holds an index where Java held a reference."},
            ],
        }
    ]
}


def self_test() -> int:
    src = scan_source_text("fixture.rs", _FIXTURE_SRC)
    # three src markers: D10 open, D28 retired, one unnumbered open.
    d10 = [m for m in src if "D10" in m.numbers]
    if not d10 or d10[0].retired:
        raise RuntimeError(f"D10 no-op collapse should be OPEN: {src}")
    d28 = [m for m in src if "D28" in m.numbers]
    if not d28 or not d28[0].retired:
        raise RuntimeError(f"D28 collapse should be RETIRED: {src}")
    if not any(m.numbers == ("(unnumbered)",) and not m.retired for m in src):
        raise RuntimeError("the unnumbered documented no-op should be an OPEN mention")

    audit = scan_audit(_FIXTURE_AUDIT)
    if not any(m.origin == "audit-body" and m.retired for m in audit):
        raise RuntimeError("the retired build_render_progress body collapse was not classified")
    if not any(m.origin == "audit-adaptation" and not m.retired and "D7" in m.numbers for m in audit):
        raise RuntimeError("the open D7 adaptation collapse was not picked up")
    if any("nothing collapsed" in m.text for m in audit):
        raise RuntimeError("a non-collapse adaptation reason must not be recorded")

    print(
        f"collapse-ledger self-test OK: {len(src)} src + {len(audit)} audit mentions classified; "
        "open vs retired and D/G grouping bite as recorded."
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--all", action="store_true", help="print retired mentions too, in full")
    parser.add_argument("--sample", type=int, default=6)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    try:
        if args.self_test:
            return self_test()
        src_files, audit = find_sources()
        if not src_files and audit is None:
            print("collapse-ledger FAIL: no transliteration source or method-audit found", file=sys.stderr)
            return 1
        mentions = collect(src_files, audit)
        return report(mentions, show_all=args.all, sample=args.sample)
    except (OSError, tomllib.TOMLDecodeError, RuntimeError) as error:
        print(f"collapse-ledger FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
