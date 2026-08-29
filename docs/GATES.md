# Gates and the can-fail discipline (R3) — __TITLE__

> No gate is trusted until it has been shown to go red on an injected defect.

The full discipline (the can-fail rule, the four vacuous-gate shapes, the
independent two-implementation oracle pattern, and the anti-bog protocol) lives
in the j2me home's `docs/GATES.md`. This file is this game's live gate ledger:
every gate the project has today, with its command and its can-fail proof. Add a
row when you add a gate.

## Current gates

| Gate | Command | Can-fail proof |
| --- | --- | --- |
| Originals provenance | `just originals-verify` | `just originals-verify-canfail` (proven RED on a one-byte payload corruption) |
| Codec remains `no_std` | `just codec-no-std` | The crate is compiled with default features disabled; filesystem/runtime code is outside its dependency graph. |
| Generic line oracle | `python3 -m unittest tools.tests.test_line_oracle` | `just oracle-harness-canfail` runs two independent processes, injects one changed observation, and requires exactly one mismatch. |
| Per-node AST crosswalk | `just crosswalk-check` (+ `python3 -m unittest tools.tests.test_crosswalk_validator`) | `just crosswalk-canfail` (`--self-test`: dropped decision, coarse blanket, and bytecode/Java-AST/Rust-AST digest perturbations each go red, plus a one-node evidence drift breaks the node lock). `just crosswalk-fixture-canfail` proves the coarse-blanket and div-vs-`sm()` bug rows go red. |

## Content-addressed affected-gate loop

`just check-affected` hashes the inputs declared for each group in
`tools/gates/gates.toml` and executes only fingerprints that have not already
passed. `just watch-affected` watches the same dependency surface and reruns a
group after an ordinary file create/write/remove changes that surface. Failed
fingerprints are never cached. The router is an iteration accelerator, not a
replacement for `just check`: the full battery still runs at milestones and at
completion, then records all successful current fingerprints.

Declare every file a gate reads, including transitive authority and ratchet
inputs that are not named on its command line. For example, if an AST-authority
validator reads the method-audit manifest to compare its admitted-body count,
that manifest belongs in the AST gate's `inputs` as well as in the method-audit
gate. Otherwise a method-only edit can reuse a stale successful fingerprint.
When a full or newly triggered gate exposes such an omission, fix the dependency
manifest before recording the new fingerprint.

## Rules (restated)

1. Every gate ships with a can-fail proof (`--self-test` / an in-test negative
   control), proven RED by a one-unit perturbation you then reverse (never
   `git checkout`).
2. Ban the four vacuous shapes: comparing against a quantity the tool never
   returns; an assertion whose subject can vanish while it holds; a skip that
   reads as a pass; a ratio of a set against itself.
3. Build the quantity you assert on yourself (pixel masks, sample counts) — never
   parse an image/audio tool's stdout numerically.
4. Corpus-dependent tests fail loudly when `_originals` is absent — never skip to
   green.
5. Retired/ignored tests carry an honest header and run by a named target.
