# J2ME Preservation Kit — repository instructions

This repository is the reusable, resource-free foundation for recovering a
surviving J2ME game as an actual canonical Java application and a faithful native
Rust port. When stamped for a game, `__TITLE__` is the target and
`__SLUG__-game-xlat` is implementation #1: the strict transliteration used as an
executable specification.

These instructions apply to the entire repository unless a more specific
`AGENTS.md` below a directory says otherwise.

## Non-negotiable boundaries

- Keep the repository resource-free. Never add original JAR/JAD/ZIP files,
  extracted proprietary assets, copyrighted source dumps, emulator captures, or
  any other copied game bytes. They belong in the separate private resources
  repository and materialize only under git-ignored `_originals/`.
- Do not weaken, bypass, or silently skip provenance, oracle, AST, ownership, or
  can-fail gates. Missing original resources are a failure for corpus-dependent
  checks, never a green skip.
- Do not create remotes, publish repositories, push branches, or upload game
  material unless the user explicitly asks for that external action.
- Preserve unrelated user changes. This is commonly a dirty recovery worktree;
  never use destructive Git commands to make it look clean.

## What belongs in the kit

The kit is the source of truth for reusable JVM/MIDP/Canvas primitives,
serialization helpers, process-oracle plumbing, Java and Rust AST walkers,
crosswalk validation, content-addressed gate routing, schemas, tests, recipes,
and workflow documentation. A reusable improvement discovered in a game must be
ported back here with its test and documentation in the same logical change.

Game repositories alone own facts that cannot be generalized without weakening
evidence: selected original hashes, build/lineage/content ledgers, recovered and
canonical game source, semantic symbol names, device/build variants, language
pack choices, game oracle adapters and vectors, and exact Java-to-Rust node
crosswalk rows.

Generated games are standalone repositories. Stamp reusable sources into them;
do not make their builds depend on `../_template` or a network checkout.
Portable Rust crates are the exception to source stamping: generated games
consume their published Git repository at an exact `rev`, and must not receive
copies of `_template/crates/j2me-*`.

## Recovery authority

- The selected original classfiles are the behavioral and binary authority.
- Keep a complete, compiling Java ME application under `java/`. It is evidence
  and tooling even when production never runs it.
- Decompilers, source-named builds, editor backups, and other releases are
  evidence, not automatic authority. Record every reconciliation and variant
  decision against original bytecode.
- Recover semantic names for every class, field, parameter, local, loop/catch
  variable, and meaningful constant. Do not claim lost author spellings when the
  original classfiles contain no local-variable debug tables.
- Integrate language packs only when their payload identity and slot shape are
  proven. Filename labels alone are insufficient.

## Strict Rust transliteration

- Translate one original method body (or one tightly coupled field/body tranche)
  at a time. Do not count wrappers, callees, fields, or helpers that have not
  independently passed admission.
- Preserve Java evaluation order, receiver/argument timing, integer promotion,
  wrapping, narrowing, shifts, division/remainder, null behavior, array bounds,
  exception cut points, partial mutation, iteration order, and observed defects.
- Keep unreviewed game callees behind explicit callbacks/adapters. A wrapper may
  be admitted without smuggling its callee into coverage.
- Model Java static and instance state with explicit, uniquely owned Rust state.
  Every production Rust declaration must have a Java owner or a documented
  Rust-only adaptation.
- Use ordinary Rust for JVM, MIDP, Canvas, transliteration, tools, and hosts.
  Apply `#![no_std]` only to deliberately portable serialization/codec crates.
- Do not refactor the strict transliteration into idiomatic Rust. A later
  implementation #2 may be idiomatic only after implementation #1 is an exact,
  executable oracle.

## Method admission contract

A method is reviewed only when all of the following are present and green:

1. exact original owner/name/descriptor plus code and opcode hashes;
2. a canonical Java body whose complete `javac` semantic AST is hash-bound;
3. a production Rust body whose complete `syn` semantic AST is hash-bound;
4. a written, non-overlapping semantic crosswalk that owns every Java and Rust
   node exactly once, including one-sided adaptations;
5. differential cases against the selected recovered JAR, canonical Java, and
   Rust, plus every applicable independent source-named build;
6. hostile edge cases that expose nulls, bounds, overflows, failures, operand
   timing, and partial state—not merely normal return values;
7. focused Rust tests and can-fail proofs showing the relevant gates turn red;
8. ratcheted method/field/declaration counts and current status documentation.

An AST node-count match is not equivalence by itself. The semantic crosswalk and
live behavioral oracle are both mandatory.

Java differential harnesses must stay below the JVM's 65,535-byte `Code`
attribute limit per method. Keep `main` as a small ordered dispatcher and put
each admission's cases in a bounded `trace…` helper (passing an explicit shared
context or the exact reflected fields it needs). Never grow one monolithic
driver method; `javac: code too large` is an oracle-architecture failure, not a
reason to drop cases. Preserve output order when extracting an existing tranche.

## Gate workflow

- Use `just check-affected` for the inner loop. Gate groups are selected from
  exact content fingerprints declared in `tools/gates/gates.toml`; Git status is
  irrelevant and failed fingerprints are never cached.
- Use `just watch-affected` when continuously admitting methods.
- Update the dependency manifest whenever a new gate or input surface lands. A
  fast loop that omits a real dependency is a correctness defect.
- Run `just check` at milestones and before claiming completion. It remains the
  full umbrella for every gate and can-fail proof and synchronizes fingerprints
  only after all checks pass.

## Working conventions

- Prefer `rg`/`rg --files` for discovery.
- Use `apply_patch` for deliberate source edits; use formatters only for
  mechanical formatting.
- Keep generated output in ignored `target/`, `_reference/`, or `_temp/` paths.
- Pin dependencies and external oracle revisions. Never make correctness depend
  on an unversioned service or latest release.
- Update local documentation when behavior, coverage, commands, or directory
  ownership changes. Documentation is part of the preservation evidence.

## Completion standard

Do not describe a port as complete because it compiles, reaches a frame, or has
many passing tests. Completion requires the full original method and field
denominators to be admitted, every production Rust declaration to be owned, all
applicable build/language variants reconciled, the complete gate umbrella green,
and the resulting game verified through frame/input/playtest milestones. Report
partial coverage honestly until then.
