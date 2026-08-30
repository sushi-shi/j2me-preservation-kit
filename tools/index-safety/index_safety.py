#!/usr/bin/env python3
"""Advisory ratchet over the signed-index landmine scanner.

The J2ME ports keep hitting one runtime-crash class: an array indexed by a
SIGNED value that can be `-1` (or a `0xFFFF`-as-`i16` masked short) cast to
`usize` -> `usize::MAX` -> out-of-bounds panic. The reads are FAITHFUL; they
crash only because upstream state (often a recorded "collapse") left a sentinel
`-1` in a def/row cell. The per-node crosswalk cannot see across that boundary,
so `tools/index-safety/j2me-index-safety` (a syn AST pass with lexical guard
analysis) is a dedicated instrument for exactly this shape.

This driver runs that scanner over the transliteration crate, then applies the
ratchet: a site is silenced only by an `// index-safe: <reason>` annotation on
or above the line (detected by the scanner) OR an entry in the allowlist TOML
recording WHY it is not a landmine (a guard the scanner missed, or a faithful
collapse-landmine tracked in D##/G##). It reports only the surviving,
UN-silenced sites and prints their count, driving a burn-down. It is ADVISORY:
it always exits 0 (some faithful sites are genuinely un-fixable reads) and is
kept out of the hard `check` fail-path.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
import tomllib
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATE = Path(__file__).resolve().parent / "j2me-index-safety"
DEFAULT_ALLOWLIST = Path(__file__).resolve().parent / "allowlist.toml"


class IndexSafetyError(RuntimeError):
    pass


@dataclass(frozen=True)
class Finding:
    file: str
    line: int
    expr: str
    source_kind: str
    source_detail: str
    annotated: bool

    @property
    def base(self) -> str:
        return self.expr.split("[", 1)[0]


def _norm_expr(expr: str) -> str:
    return "".join(expr.split())


def _rel(path: Path) -> str:
    path = path if path.is_absolute() else (ROOT / path)
    try:
        return path.resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return path.as_posix()


def discover_roots() -> list[Path]:
    """Locate the game's transliteration source without per-game config."""
    translit = ROOT / "transliteration"
    if not translit.exists():
        return []
    roots: list[Path] = []
    for pattern in ("crates/*-xlat/src", "*-xlat/src", "game-xlat/src", "crates/*/src"):
        for candidate in sorted(translit.glob(pattern)):
            if candidate.is_dir() and candidate not in roots:
                # Only crates that look like the hand-written transliteration.
                if "xlat" in candidate.parent.name or candidate.parent.name == "game-xlat":
                    roots.append(candidate)
    return roots


def rust_files(roots: list[Path]) -> list[Path]:
    files: list[Path] = []
    for root in roots:
        root = root if root.is_absolute() else (ROOT / root)
        if root.is_file() and root.suffix == ".rs":
            files.append(root)
        elif root.is_dir():
            files.extend(sorted(root.rglob("*.rs")))
    if not files:
        raise IndexSafetyError(f"no Rust sources under {[str(r) for r in roots]}")
    return files


def build_detector() -> None:
    manifest = CRATE / "Cargo.toml"
    if not manifest.exists():
        raise IndexSafetyError(f"detector crate is absent: {manifest}")
    result = subprocess.run(
        ["cargo", "build", "-q", "--manifest-path", str(manifest)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise IndexSafetyError(f"cargo build failed:\n{result.stderr}")


def detector_binary() -> Path:
    return CRATE / "target" / "debug" / "j2me-index-safety"


def run_detector(files: list[Path]) -> list[Finding]:
    binary = detector_binary()
    if not binary.exists():
        build_detector()
    result = subprocess.run(
        [str(binary), *[str(f) for f in files]],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise IndexSafetyError(f"scanner failed (exit {result.returncode}):\n{result.stderr}")
    findings: list[Finding] = []
    for line in result.stdout.splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        if len(parts) != 6:
            raise IndexSafetyError(f"malformed scanner row: {line!r}")
        path, line_no, expr, kind, detail, annotated = parts
        findings.append(
            Finding(_rel(Path(path)), int(line_no), expr, kind, detail, annotated == "1")
        )
    return findings


def load_allowlist(path: Path) -> dict[tuple[str, int, str], str]:
    if not path.exists():
        return {}
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    entries: dict[tuple[str, int, str], str] = {}
    for entry in data.get("allow", []):
        file = _rel(Path(str(entry["file"])))
        line = int(entry["line"])
        expr = _norm_expr(str(entry["expr"]))
        reason = str(entry.get("reason", "")).strip()
        if not reason:
            raise IndexSafetyError(f"allowlist entry {file}:{line} {expr} needs a reason")
        entries[(file, line, expr)] = reason
    return entries


def partition(
    findings: list[Finding], allowlist: dict[tuple[str, int, str], str]
) -> tuple[list[Finding], list[Finding], set[tuple[str, int, str]]]:
    """Return (surviving, silenced, allow_keys_hit)."""
    surviving: list[Finding] = []
    silenced: list[Finding] = []
    hit: set[tuple[str, int, str]] = set()
    for finding in findings:
        key = (finding.file, finding.line, _norm_expr(finding.expr))
        if finding.annotated:
            silenced.append(finding)
        elif key in allowlist:
            silenced.append(finding)
            hit.add(key)
        else:
            surviving.append(finding)
    return surviving, silenced, hit


# The families the task calls out as the genuine, un-fixable-by-crosswalk class.
LANDMINE_TABLES = (
    "def_e",
    "def_p",
    "def_r",
    "def_c",
    "def_h",
    "def_u",
    "def_g",
    "def_w",
    "container_contents",
    "npc_defs",
)


def report(
    findings: list[Finding],
    allowlist: dict[tuple[str, int, str], str],
    *,
    sample: int,
    show_all: bool,
) -> int:
    surviving, silenced, hit = partition(findings, allowlist)
    stale = sorted(set(allowlist) - hit)

    annotated = sum(1 for f in silenced if f.annotated)
    print("index-safety (advisory): signed-index landmine scan")
    print(
        f"  {len(findings)} candidate sites | "
        f"{annotated} annotated | {len(hit)} allowlisted | "
        f"{len(surviving)} UN-SILENCED"
    )
    if stale:
        print(f"  note: {len(stale)} allowlist entr{'y' if len(stale)==1 else 'ies'} matched nothing (code moved?):")
        for file, line, expr in stale[:10]:
            print(f"    stale: {file}:{line} {expr}")

    if not surviving:
        print("  no un-silenced landmine sites.")
        return 0

    by_base = Counter(f.base for f in surviving)
    print("\n  un-silenced sites by table (top):")
    for base, count in by_base.most_common(15):
        print(f"    {count:4d}  {base}")

    family = [f for f in surviving if f.base in LANDMINE_TABLES]
    if family:
        print(f"\n  def/creature-table family (the D29 `def_e[row[4]]` crash class), {len(family)} sites:")
        for finding in family[: (len(family) if show_all else sample)]:
            print(f"    {finding.file}:{finding.line}  {finding.expr}   <- {finding.source_detail}")
        if not show_all and len(family) > sample:
            print(f"    ... and {len(family) - sample} more (pass --all)")

    others = [f for f in surviving if f.base not in LANDMINE_TABLES]
    if others:
        print(f"\n  other un-silenced sites, showing {min(sample, len(others))} of {len(others)}:")
        for finding in others[: (len(others) if show_all else sample)]:
            print(f"    {finding.file}:{finding.line}  {finding.expr}   <- {finding.source_detail}")

    return 0


# ---- self-test -------------------------------------------------------------

_FIXTURE = """
pub struct Game;
fn landmine(g: &Game, r: &[i32]) -> i32 {
    // a faithful read of a def-table row whose col can be the -1 sentinel
    g.canvas.def_e[r[4] as usize][13]
}
fn annotated_site(g: &Game, r: &[i32]) -> i32 {
    // index-safe: upstream invariant guarantees r[4] >= 0 here (D-test)
    g.canvas.def_e[r[4] as usize][11]
}
fn guarded_site(g: &Game, r: &[i32]) -> i32 {
    if r[4] < 0 { return 0; }
    g.canvas.def_e[r[4] as usize][12]
}
"""


def self_test() -> int:
    build_detector()
    with tempfile.TemporaryDirectory() as tmp:
        fixture = Path(tmp) / "fixture.rs"
        fixture.write_text(_FIXTURE, encoding="utf-8")
        findings = run_detector([fixture])

        landmines = [f for f in findings if f.line == 5]  # the def_e[r[4]] on line 5
        if not landmines:
            raise IndexSafetyError(f"scanner did not flag the seeded landmine: {findings}")

        guarded = [f for f in findings if f.source_detail == "r [4]" and f.line == 15]
        if guarded:
            raise IndexSafetyError("the `if r[4] < 0 { return }` guard was not honored")

        annotated = [f for f in findings if f.line == 9]
        if not annotated or not annotated[0].annotated:
            raise IndexSafetyError("the `// index-safe:` annotation was not detected")

        # Un-silenced before allowlisting: exactly the bare landmine on line 5.
        surviving, silenced, _ = partition(findings, {})
        if not any(f.line == 5 for f in surviving):
            raise IndexSafetyError("landmine should be un-silenced with an empty allowlist")
        if not any(f.line == 9 and f.annotated for f in silenced):
            raise IndexSafetyError("annotated site should be silenced")

        # Allowlisting the landmine silences it -> zero survivors.
        target = next(f for f in surviving if f.line == 5)
        allowlist = {
            (target.file, target.line, _norm_expr(target.expr)): "faithful (self-test)"
        }
        surviving_after, _, hit = partition(findings, allowlist)
        if any(f.line == 5 for f in surviving_after):
            raise IndexSafetyError("allowlist entry did not silence the landmine")
        if not hit:
            raise IndexSafetyError("allowlist entry was not recorded as hit")

    print(
        "index-safety self-test OK: a seeded `def_e[r[4] as usize]` landmine is flagged, "
        "a `< 0` guard and an `// index-safe:` annotation suppress it, and an allowlist "
        "entry silences it (the ratchet bites and can be released)."
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("roots", nargs="*", type=Path, help="Rust source dirs/files (default: autodiscover)")
    parser.add_argument("--allowlist", type=Path, default=DEFAULT_ALLOWLIST)
    parser.add_argument("--sample", type=int, default=12, help="sites to print per group")
    parser.add_argument("--all", action="store_true", help="print every un-silenced site")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    try:
        if args.self_test:
            return self_test()
        roots = args.roots or discover_roots()
        if not roots:
            raise IndexSafetyError(
                "no transliteration source found; pass the crate src dir explicitly"
            )
        files = rust_files(roots)
        findings = run_detector(files)
        allowlist = load_allowlist(args.allowlist)
        return report(findings, allowlist, sample=args.sample, show_all=args.all)
    except (IndexSafetyError, OSError, tomllib.TOMLDecodeError) as error:
        print(f"index-safety FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
