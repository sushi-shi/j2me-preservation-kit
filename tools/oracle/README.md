# Reusable line oracle

`line_oracle.py` is the shared comparison engine. Each implementation reads one
case per stdin line and emits exactly one observation per stdout line. A game
may drive it through a small TOML file for fixed cases, or import `run_oracle`
from a game-specific generator when the case matrix is large.

The game-specific layer owns:

- compilation and launch commands for recovered JAR(s), canonical Java, and Rust;
- deterministic case generation and its count ratchet;
- the reference label;
- exact member/build variant exclusions, validated against that game's ledger.

The shared layer owns process failures, one-output-per-case cardinality,
comparison diagnostics, and `--self-test`, which injects one changed observation
and requires exactly that mismatch to be detected.

Minimal fixed-case configuration:

```toml
schema_version = 1
cases = "cases.txt"
expected_case_count = 3
reference = "canonical-java"

[[implementation]]
label = "canonical-java"
command = ["tools/oracle/run-canonical-java"]

[[implementation]]
label = "rust"
command = ["cargo", "run", "-q", "-p", "__SLUG__-oracle"]
```

Do not encode reviewed divergences as broad output filters. A game-specific
callback must name the exact command/member and prove that classification exists
in the build-variant ledger.

---

## Two oracles

There are now **two** independent oracles in this directory; they answer
different questions and are used together.

1. **`line_oracle.py`** — the unit/parser **line-differential** oracle described
   above. One case per stdin line, one observation per stdout line, comparing
   canonical Java against the Rust port at the granularity of individual cases.

2. **The FreeJ2ME-Plus frame oracle** — an **exact-pixel** oracle for the
   rendered 2D screen, made of three parts:
   - `capture_reference.sh` — clones, patches, and builds FreeJ2ME-Plus in a
     scratch dir (nothing third-party is vendored), then drives the *original
     bytecode* through shared routes and writes reference PNGs + a provenance
     manifest;
   - `HeadlessCapture.java` — a headless capture frontend for FreeJ2ME-Plus (its
     public API only; it does not modify emulator behaviour) that runs the routes
     without an X display;
   - `compare_frames.py` — decodes both sides' PNGs **in-process** (never shells
     out to an image tool) and compares them exactly, `differing_pixels == 0`
     being the only clean state; carries an inline self-test, fail-closed capture
     provenance, and non-vacuity/liveness floors.

   Both are **config-driven**: every per-game knob (the JAR, canvas geometry, the
   `--sound` flag, the emulator patch set, and the pinned emulator commit) is read
   from **`game.toml [oracle]`**, so the scripts carry no game-specific literal and
   stamp into a new 2D J2ME port unchanged.

Supporting files for the frame oracle: `patches/` (game-agnostic FreeJ2ME-Plus
determinism fixes selected by `game.toml [oracle].patches`), `routes/` (per-game
keystroke scripts — see `routes/README.md`; the template ships only an
`.example`), and `agreement.toml` (the per-label exact-agreement ratchet).

The frame oracle is **2D only**: a pure-`Graphics` game has nothing 3D to
disagree about, so every frame is compared byte-for-byte with no tolerance.
