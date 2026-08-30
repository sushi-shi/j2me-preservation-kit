# Gates and the can-fail discipline (R3) — __TITLE__

> No gate is trusted until it has been shown to go red on an injected defect.

The full discipline lives in the j2me home's `docs/GATES.md`. This is the
generated game's live Phase-0/1 ledger; add rows as later recovery phases land.

## Current gates

| Gate | Command | Can-fail proof |
| --- | --- | --- |
| Originals provenance | `just originals-verify` | `just originals-verify-canfail` corrupts one byte in memory and requires rejection. |
| Corpus classifier | `just classify` | `just classify-canfail` perturbs one baseline class and requires its fingerprint to change without contaminating other builds. |
| Resource catalog | `just catalog` | `just catalog-canfail` perturbs one resource occurrence and requires it to move to a new content-hash bucket. |
| Device-evidence audit | `just device-evidence` | `just device-evidence-canfail` removes one device-sensitive API edge and requires exactly one review finding to disappear; the report never auto-selects a handset. |
| Generic line oracle | `python3 -m unittest tools.tests.test_line_oracle` | `just oracle-harness-canfail` changes one observation and requires exactly one mismatch. |
| Per-node AST crosswalk | `just crosswalk-check` | `just crosswalk-canfail` and `just crosswalk-fixture-canfail` inject missing/coarse/wrong ownership decisions and require rejection. |
| Whole-port completeness tooling | `just port-completeness <manifest>`; `just variants-check <manifest>`; optional `just native-reachability <manifest>` once configured | `just port-gates-canfail` drops field/call/declaration ownership, leaks an oracle target, and changes a reviewed variant shape. |
| Runtime dependency boundary | `just architecture-check` | `just architecture-canfail` injects forbidden portable/host dependency edges. |
| Device profiles | `just device-profiles-check` | `just device-profiles-canfail` removes the required system axis and assigns a build to an unknown phone profile. |
| Consumer Rust workspace | `cargo clippy --workspace --all-targets -- -D warnings` plus `cargo test --workspace` | The scaffold regression in the j2me home proves there is no copied `crates/j2me-*` tree or path dependency and locks the public runtime Git revision. |

## Content-addressed affected-gate loop

`just check-affected` hashes the inputs declared in
`tools/gates/gates.toml` and executes only fingerprints that have not already
passed. `just watch-affected` watches the same dependency surface. Failed
fingerprints are never cached. The router accelerates iteration; `just check`
remains the milestone battery.

Declare every file a gate reads, including transitive authority and ratchet
inputs. Missing dependencies can reuse stale successful fingerprints and are
therefore correctness defects.

## Rules

1. Every gate ships with a one-unit negative control that is proven red.
2. Reject vacuous checks: missing subjects, green skips, self-comparisons, and
   assertions about quantities the tool never returns.
3. Corpus-dependent checks fail loudly when `_originals` is absent.
4. Add the numeric-shape and canonical-Java gates to the umbrella only after
   filling `game.toml [java]` and recovering the complete canonical source tree.
