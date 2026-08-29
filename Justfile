set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

# --- Fresh clone -------------------------------------------------------------

# Fresh clone to verified: materialize the corpus, then reconcile it. The
# resource location is passed explicitly; it is never baked into the repo (R1).
# Phase 1 adds `classify` + `catalog` here as they land (R13 clean-slate).
bootstrap resources:
    nix run .#fetch-resources -- {{quote(resources)}}
    just originals-verify

# Verify the materialized _originals against builds.toml's sha256/bytes table.
originals-verify:
    python3 tools/originals/verify.py

# Prove the originals-verify gate can fail (playbook R3). Must exit 0.
originals-verify-canfail:
    python3 tools/originals/verify.py --self-test

# Serialization/format primitives are the deliberately portable no_std layer.
codec-no-std:
    cargo check -p j2me-codec --no-default-features

# Prove the reusable line oracle detects one injected observation mismatch.
oracle-harness-canfail:
    python3 -m unittest tools.tests.test_line_oracle.LineOracleTests.test_two_independent_processes_match_and_self_test_bites

# Per-node AST crosswalk verifier. Every Java and Rust node must carry its own
# explicit decision + comment; no operation may blanket a whole body. Runs the
# generic verifier over the shipped paint_radio_row fixture in --strict mode (a
# port repoints these at its own method-audit manifest + live-emitted evidence).
crosswalk-check:
    python3 tools/ast/validate_crosswalk.py \
        tools/ast/fixtures/paint_radio_row.crosswalk.toml \
        --evidence tools/ast/fixtures/paint_radio_row.evidence.toml --strict

# Live "how much is still unchecked" report: decided/total nodes per body.
crosswalk-coverage:
    python3 tools/ast/validate_crosswalk.py \
        tools/ast/fixtures/paint_radio_row.crosswalk.toml \
        --evidence tools/ast/fixtures/paint_radio_row.evidence.toml --coverage

# Prove the crosswalk gate can fail (playbook R3): dropped decision, coarse
# blanket, and bytecode/Java-AST/Rust-AST digest perturbations each go red, and a
# one-node evidence drift breaks the node-inventory lock. Must exit 0.
crosswalk-canfail:
    python3 tools/ast/validate_crosswalk.py --self-test

# Prove the fixture's coarse-blanket and div-vs-call bug rows go red as recorded.
crosswalk-fixture-canfail:
    python3 -m unittest \
        tools.tests.test_crosswalk_validator.CrosswalkValidatorTests.test_coarse_blanket_is_rejected \
        tools.tests.test_crosswalk_validator.CrosswalkValidatorTests.test_operator_parity_catches_div_vs_call

# Regenerate builds.toml provenance from a resources dir (mechanical facts only;
# the judgment calls stay flagged for Phase 1 — see the file header).
gen-builds resources match:
    python3 tools/originals/gen_builds.py \
        --resources {{quote(resources)}} --match {{quote(match)}} \
        --slug "$(python3 -c 'import tomllib;print(tomllib.load(open("game.toml","rb"))["slug"])')" \
        --title "$(python3 -c 'import tomllib;print(tomllib.load(open("game.toml","rb"))["title"])')" \
        --out java/reconstruction/builds.toml

# --- Test batteries ----------------------------------------------------------

# Content-address every gate's declared inputs and run only groups whose exact
# fingerprint has not passed before. Git cleanliness is deliberately irrelevant.
check-affected:
    python3 tools/gates/check_changed.py

# Explain the hash-selected groups and commands without executing them.
check-affected-dry:
    python3 tools/gates/check_changed.py --dry-run

# Keep the same hash router active while editing; a changed input reruns its
# dependent groups after a short debounce.
watch-affected interval="0.5":
    python3 tools/gates/check_changed.py --watch --interval {{quote(interval)}}

test:
    if [ -d tools/tests ]; then python3 -m unittest discover -s tools/tests; fi
    if [ -f Cargo.toml ]; then cargo test --workspace; fi

# Every gate the project has today. Grows as phases land; every gate cited here
# must exist and be proven able to fail (playbook R3, R14).
check:
    just originals-verify
    just originals-verify-canfail
    just codec-no-std
    just oracle-harness-canfail
    just crosswalk-check
    just crosswalk-canfail
    just crosswalk-fixture-canfail
    if [ -d tools/tests ]; then python3 -m unittest discover -s tools/tests; fi
    if [ -f Cargo.toml ]; then cargo fmt --all --check; fi
    if [ -f Cargo.toml ]; then cargo clippy --workspace --all-targets -- -D warnings; fi
    if [ -f Cargo.toml ]; then cargo test --workspace; fi
    python3 tools/gates/check_changed.py --record-all
