# Repository structure — __TITLE__

This repository is **resource-free** and dedicated to the public domain (CC0).
It contains only recovered-by-hand reconstruction and our own code — never
literal copied binaries. See the j2me home's `PLAYBOOK.md` for the method.

## Resource storage contract (three layers)

1. **Authored sources, in this repo** — engine/game/tooling code, transcribed
   data, and the reconstruction ledgers (`java/reconstruction/`).
2. **Binary resources, in a private resources location** — the surviving
   distributions. `tools/originals/fetch.py <path-or-url>` (or the env var named
   in `game.toml`) copies them into git-ignored `_originals/` and runs
   `verify.py`. The location is never baked into the repo (R1).
3. **Derived, regenerable** — `_reference/` (catalogs, fingerprints, decompiles)
   and web outputs, rebuilt from layers 1–2, never committed.

## Boundary rules (fill in as phases land)

- The root `Cargo.toml` is the only workspace manifest; one `target/`, one lock.
- In this preservation-kit repository, `crates/j2me-*` are the canonical
  portable sources. `new-game.py` excludes them from generated games and emits
  Git dependencies pinned to one immutable public revision instead.
- `transliteration/game-xlat` (Phase 3) is the per-game executable spec, not the
  shipped engine — production code must not depend on it at runtime; tests may
  (R12). Transliteration is not constrained to `no_std`.
- `crates/` holds the shipped engine libraries (2D or 3D per `game.toml`'s
  `fork`); `apps/` the frontends; `web/` page composition only.
- `j2me-me` owns only Java ME semantics and host operations. `j2me-device`
  defines independently composable phone behavior; concrete profiles stay
  game-owned, while vendor implementations are explicitly named crates.
- `j2me-media` owns portable format parsing, decoding, synthesis, and resampling.
  `j2me-platform` owns JAR/JAD, RMS/path, pixel projection, and focus-policy
  helpers. CPAL/winit and Web APIs live only in `j2me-platform-native` and
  `j2me-platform-web` respectively.
- Repository-owned ignored directories begin with `_`; `.gitignore` matches them
  by name (no trailing-slash globs) so symlinks are covered too (R2).
- `_originals` must be a **real directory, never a symlink** (R2).

## Planned layout (mirrors the sibling ports)

```text
_originals/            immutable surviving jars/zips (git-ignored, sha256-verified)
_reference/            generated catalogs & fingerprints (git-ignored, regenerable)
java/reconstruction/   builds.toml, symbols.toml, admission plans, variants — ledgers
transliteration/       per-game game-xlat executable spec and adapters
crates/                canonical portable j2me-* sources (kit repo only)
apps/                  native desktop host, play/site exporters, web frontend
tools/                 originals/, AST walkers/crosswalk, line oracle, workflows
docs/                  REPOSITORY_STRUCTURE, GATES, FORMATS, STATUS (+ more)
game.toml, device-profiles.toml, Justfile, flake.nix, Cargo.toml
```
