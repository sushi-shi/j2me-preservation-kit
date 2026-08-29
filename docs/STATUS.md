# Status — __TITLE__

Living record of what is recovered and verified. Newest first.

## Phase 0 — resource-free foundation (scaffolded)

- Repo initialized resource-free (R1): CC0 `LICENSE`, `.gitignore` written first
  (matches `_originals`/`_reference`/`target` by name, no trailing-slash globs).
- `java/reconstruction/builds.toml` — the provenance authority — auto-generated
  from the resources dir: every unique JAR payload deduped by sha256 (top-level +
  nested-in-zip), with byte length, aliases, containers, MIDlet/device metadata,
  and a 2D/3D probe. Judgment fields (official/archived, true language, baseline)
  flagged for Phase 1.
- `tools/originals/{verify,fetch,gen_builds}.py`: the `originals-verify` gate is
  **proven able to fail** (`just originals-verify-canfail`).
- The generic Phase-3 support is already present: JVM arithmetic/errors/clock,
  neutral ARGB images, MIDP Graphics/Canvas/MMAPI/RMS state, a line-protocol
  differential oracle, complete `javac`/`syn` walkers, and an exhaustive node
  ownership validator. Only `j2me-codec` is constrained to `no_std`; device and
  transliteration crates use ordinary Rust.
- `transliteration/game-xlat` deliberately claims zero recovered methods. Each
  admitted game body must add bytecode/opcode hashes, complete Java/Rust AST
  ownership, semantic adaptations, and live oracle evidence.
- Select bodies only after joining the canonical owner/signature to a real
  original classfile method. Route source-only `ConstantValue` initializer ASTs
  to declaration coverage, and represent every unreviewed callee through an
  explicit callback/adapter boundary so wrapper coverage cannot absorb it.
- Fork: **__FORK__** (see the j2me home's `docs/FORK_2D_3D.md`).

## Difficulty tier & recommended first phases

_Fill from GAMES.md._ Recommended next: **Phase 1** — classify (class/method
fingerprints across the unique payloads, cross-build deltas, lineage/language
reconciliation, pick baseline + naming reference) and format triage (a
malformed-input-rejecting parser per custom format, tested on every blob).

## Open decisions (from the playbook)

1. Baseline vs naming-reference build (decide from Phase 1 fingerprints).
2. True per-build shipped language (decode the string tables; filenames lie).
3. Custom-format semantics (reverse from consumers in Phase 1).
4. Reference oracle source for Phase 3 (FreeJ2ME-Plus capture vs a headless-JVM
   MIDP host vs the decompiled Java).
