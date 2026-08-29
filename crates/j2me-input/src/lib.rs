//! Generic, remappable native-key to Nokia `FullCanvas` key-code mapping for
//! J2ME ports.
//!
//! A transliterated `keyPressed(int)` speaks one fixed, game-agnostic vocabulary
//! of raw Nokia codes: the D-pad is `-1..=-4`, Fire is `-5`, the soft keys are
//! `-6`/`-7`, the digits `0..=9` are `48..=57`, and `*`/`#` are `42`/`35`. This
//! crate is the single place a native host translates a physical [`KeyCode`]
//! (from `winit`) into that vocabulary, so every port inherits sane, remappable
//! controls instead of hand-copying a per-game keymap.
//!
//! # What you get
//!
//! - Two built-in presets ([`Preset::Standard`] arrows + [`Preset::Mobile`], a
//!   WASD/Q-E/X/R-F superset mirroring the ports' mobile clusters).
//! - A player/porter override layer read from an optional `[keymap]` table (in
//!   `game.toml`, a standalone `keymap.toml`, or an env var) — no code edits and
//!   no `serde`/`toml` dependency.
//! - An unknown or unmapped key resolves to `None`, which the host drops (the
//!   device queue rejects out-of-vocabulary keys — R10).
//!
//! # Consuming it from a native host
//!
//! Build a [`Keymap`] once at start-up, then translate each key event through it:
//!
//! ```
//! use j2me_input::{Keymap, KeyCode, Preset};
//!
//! // `keymap` might come from `game.toml`'s `[keymap]` table or a `keymap.toml`;
//! // `None` here just takes the default preset.
//! let player_config: Option<&str> = None;
//! let keymap = Keymap::from_config(player_config, Preset::Mobile)
//!     .expect("keymap config is valid");
//!
//! // In the winit key handler, forward only keys the game understands:
//! # let physical_key = KeyCode::KeyW;
//! if let Some(code) = keymap.nokia_code(physical_key) {
//!     // canvas.key_pressed(code);  // feed the transliterated keyPressed(int)
//!     assert_eq!(code, -1); // W is the D-pad up in the Mobile preset
//! }
//! ```
//!
//! A port that today hand-writes a `nokia_code(KeyCode) -> Option<i32>` (as
//! gothic-mobile and stalker-mobile do) swaps the whole function for a single
//! `Keymap` built at start-up and one `keymap.nokia_code(key)` call at the event
//! site — same codes, now remappable and shared.

pub mod config;
pub mod keymap;
pub mod nokia;
pub mod preset;

pub use config::{ConfigError, KeymapConfig};
pub use keymap::Keymap;
pub use nokia::Action;
pub use preset::{key_from_name, Preset};

/// Re-exported so consumers (and this crate's own doc/integration tests) can name
/// physical keys without taking their own direct `winit` dependency.
pub use winit::keyboard::KeyCode;
