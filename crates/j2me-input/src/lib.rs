//! Generic physical-key to semantic handset-key mapping for J2ME ports.
//!
//! This crate performs only the first hop from winit [`KeyCode`] to
//! [`HandsetKey`]. A game-owned `j2me-device` input fragment performs the second
//! hop to the device-specific integer delivered to `keyPressed(int)`. The
//! explicitly named [`nokia`] module is a compatibility wrapper; generic MIDP
//! code never selects it implicitly.
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
//! ```no_run
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
//! # let profile: j2me_device::InputFragment = unimplemented!();
//! if let Some(code) = keymap.raw_code(physical_key, &profile) {
//!     // canvas.key_pressed(code);  // feed the transliterated keyPressed(int)
//!     assert_eq!(code, -1); // W is the D-pad up in the Mobile preset
//! }
//! ```
//!
//! A port that hand-writes a raw key-code function replaces it with one
//! [`Keymap`] plus the reviewed game-owned input fragment.

pub mod config;
pub mod keymap;
pub mod nokia;
pub mod preset;

pub use config::{ConfigError, KeymapConfig};
pub use j2me_device::HandsetKey;
pub use keymap::{KeyBinding, Keymap};
pub use nokia::Action;
pub use preset::{key_from_name, Preset};

/// Re-exported so consumers (and this crate's own doc/integration tests) can name
/// physical keys without taking their own direct `winit` dependency.
pub use winit::keyboard::KeyCode;
