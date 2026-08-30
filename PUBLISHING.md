# Publishing the J2ME library crates

This workspace publishes **twelve** reusable library crates to crates.io:

| crate | role | inter-crate deps |
|-------|------|------------------|
| `j2me-canvas` | neutral ARGB image + source-over compositing | — |
| `j2me-jvm` | JVM primitive semantics for strict transliteration | — |
| `j2me-codec` | `no_std` bounded readers for J2ME wire formats | — |
| `j2me-device` | composable handset-profile schema | — |
| `j2me-device-nokia` | opt-in Nokia S60 handset behavior | `j2me-device` |
| `j2me-media` | AMR/MIDI/SMAF/WAV and DSP | — |
| `j2me-me` | Java ME / MIDP 2D semantics | `j2me-canvas`, `j2me-device`, `j2me-jvm` |
| `j2me-input` | physical → semantic handset input | `j2me-device`, `j2me-device-nokia` |
| `j2me-nokia` | Nokia UI `DirectGraphics` (opt-in) | `j2me-canvas`, `j2me-jvm`, `j2me-me` |
| `j2me-platform` | resource/RMS/path/presentation/lifecycle host core | `j2me-device`, `j2me-me`, `j2me-media` |
| `j2me-platform-native` | winit/CPAL/native endpoints | input, device, ME, media, platform |
| `j2me-platform-web` | WebAudio/browser endpoints | device, ME, media, platform |

`transliteration/game-xlat` (per-game placeholder) and
`tools/ast/j2me-ast-audit` (internal tool) keep `publish = false` and are **not**
published.

## Before you publish

1. **Set the real repository/owner.** `[workspace.package].repository` and
   `homepage` in `Cargo.toml` are placeholders (`https://github.com/OWNER/j2me`).
   Replace `OWNER` with the real GitHub org/user. The library crates inherit
   this via `repository.workspace = true`, so editing the workspace value updates
   them all.
2. **Create the GitHub repo first** and push, so the `repository` URL resolves
   before the crates reference it.
3. **Check every tabled crate name is free** on crates.io. A taken name is the
   one thing only you can confirm; `cargo package` cannot.
4. **Confirm CC0 is intended.** Every crate publishes under `CC0-1.0` (public
   domain). crates.io accepts this SPDX id.
5. **Log in:** `cargo login <token>` (a crates.io API token).
6. **`categories` note:** the slugs used are all valid crates.io categories
   (`game-development`, `emulators`, `encoding`, `no-std`, `rendering`,
   `filesystem`, `parser-implementations`). crates.io ignores unknown category slugs on
   publish, so double-check the live category list if you add more.

## Dry run (no publish)

Run inside the toolchain shell (`nix develop --command bash -lc '<cmd>'`):

```sh
cargo build --workspace && cargo test --workspace
cargo package --workspace --exclude __SLUG__-game-xlat --exclude j2me-ast-audit
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

`cargo package` proves each crate is self-contained: all dependencies carry a
`version`, and every referenced file resolves inside the packaged tarball.

## Publish order (dependency order)

A dependent crate can only be published after every crate it depends on is
already live on crates.io (crates.io resolves the versioned dep against the
registry, not the local `path`). Publish one at a time, in this order:

```sh
# 1. leaf crates — no inter-crate deps
cargo publish -p j2me-canvas
cargo publish -p j2me-jvm
cargo publish -p j2me-codec
cargo publish -p j2me-device
cargo publish -p j2me-media

# 2. phone implementation and MIDP semantics
cargo publish -p j2me-device-nokia
cargo publish -p j2me-me

# 3. shared projections and host core
cargo publish -p j2me-input
cargo publish -p j2me-nokia
cargo publish -p j2me-platform

# 4. platform endpoints
cargo publish -p j2me-platform-native
cargo publish -p j2me-platform-web
```

If crates.io is slow to index a just-published crate, wait a moment before
publishing the next dependent one (or use `cargo publish` again — it is
idempotent per version once indexed).

## How the inter-crate deps are wired

Each inter-crate dependency carries **both** a `path` and a `version`, e.g. in
`j2me-me/Cargo.toml`:

```toml
j2me-canvas = { path = "../j2me-canvas", version = "0.1.0" }
```

- `path` keeps local workspace builds (and any concurrent path-dep consumer)
  working against the live source.
- `version` is what `cargo publish` writes into the published manifest; `path`
  is stripped from the uploaded crate.

Keep the two in sync: when you bump `[workspace.package].version`, update the
`version = "…"` on every inter-crate dependency to match.
