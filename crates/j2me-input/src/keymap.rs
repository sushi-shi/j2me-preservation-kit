//! [`Keymap`]: a preset plus optional per-key overrides, resolving a physical
//! [`KeyCode`] to the raw Nokia code the game expects.

use crate::config::{self, ConfigError};
use crate::preset::Preset;
use std::collections::HashMap;
use winit::keyboard::KeyCode;

/// A resolved key map: a base [`Preset`] with an override layer on top.
///
/// Look-up order for a physical key is: an explicit override wins (including an
/// explicit *unbind* to `None`); otherwise the preset decides; a key neither
/// side binds resolves to `None` and the host drops it (the device queue rejects
/// out-of-vocabulary keys — R10).
#[derive(Clone, Debug, Default)]
pub struct Keymap {
    preset: Preset,
    /// `Some(code)` rebinds a key; `None` explicitly unbinds it.
    overrides: HashMap<KeyCode, Option<i32>>,
}

impl Keymap {
    /// A map that uses `preset` with no overrides.
    pub fn new(preset: Preset) -> Self {
        Self {
            preset,
            overrides: HashMap::new(),
        }
    }

    /// The base preset this map falls back to.
    pub fn preset(&self) -> Preset {
        self.preset
    }

    /// The raw Nokia code for `key`, or `None` if nothing binds it.
    pub fn nokia_code(&self, key: KeyCode) -> Option<i32> {
        match self.overrides.get(&key) {
            Some(&binding) => binding,
            None => self.preset.nokia_code(key),
        }
    }

    /// Override `key` to deliver `code`. Chainable.
    pub fn bind(&mut self, key: KeyCode, code: i32) -> &mut Self {
        self.overrides.insert(key, Some(code));
        self
    }

    /// Explicitly unbind `key` so it resolves to `None` even if the preset binds
    /// it. Chainable.
    pub fn unbind(&mut self, key: KeyCode) -> &mut Self {
        self.overrides.insert(key, None);
        self
    }

    /// Build a map from an optional config, falling back to `default_preset`
    /// when the config is absent or does not set its own `preset`.
    ///
    /// This is the entry point a native host uses: pass the game's chosen
    /// default preset plus whatever a player's `[keymap]` table (from
    /// `game.toml`, a `keymap.toml`, or an env var) contained, if any.
    pub fn from_config(config: Option<&str>, default_preset: Preset) -> Result<Self, ConfigError> {
        let Some(text) = config else {
            return Ok(Keymap::new(default_preset));
        };
        let parsed = config::parse(text)?;
        let mut map = Keymap::new(parsed.preset.unwrap_or(default_preset));
        for (key, binding) in parsed.bindings {
            map.overrides.insert(key, binding);
        }
        Ok(map)
    }

    /// Build a map from a config string, using the config's own `preset` or the
    /// library default ([`Preset::Mobile`]) when it sets none.
    pub fn from_config_str(config: &str) -> Result<Self, ConfigError> {
        Keymap::from_config(Some(config), Preset::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nokia;
    use winit::keyboard::KeyCode as K;

    #[test]
    fn default_is_the_mobile_preset_with_no_overrides() {
        let km = Keymap::default();
        assert_eq!(km.preset(), Preset::Mobile);
        assert_eq!(km.nokia_code(K::KeyW), Some(nokia::UP));
    }

    #[test]
    fn override_wins_over_the_preset() {
        let mut km = Keymap::new(Preset::Mobile);
        assert_eq!(km.nokia_code(K::KeyW), Some(nokia::UP)); // preset default
        km.bind(K::KeyW, nokia::FIRE);
        assert_eq!(km.nokia_code(K::KeyW), Some(nokia::FIRE)); // override applied
    }

    #[test]
    fn explicit_unbind_beats_a_preset_binding() {
        let mut km = Keymap::new(Preset::Mobile);
        km.unbind(K::KeyW);
        assert_eq!(km.nokia_code(K::KeyW), None);
    }

    #[test]
    fn unmapped_key_resolves_to_none() {
        let km = Keymap::new(Preset::Standard);
        assert_eq!(km.nokia_code(K::KeyI), None);
    }

    #[test]
    fn from_config_absent_uses_the_default_preset() {
        let km = Keymap::from_config(None, Preset::Standard).unwrap();
        assert_eq!(km.preset(), Preset::Standard);
        // Standard does not bind WASD.
        assert_eq!(km.nokia_code(K::KeyW), None);
    }

    #[test]
    fn config_preset_overrides_the_default() {
        let km =
            Keymap::from_config(Some("[keymap]\npreset = \"mobile\"\n"), Preset::Standard).unwrap();
        assert_eq!(km.preset(), Preset::Mobile);
        assert_eq!(km.nokia_code(K::KeyW), Some(nokia::UP));
    }

    #[test]
    fn config_bindings_layer_over_the_chosen_preset() {
        let text = "\
[keymap]
preset = \"standard\"
KeyH = SoftLeft   # add a key Standard doesn't have
F1   = none       # drop Standard's left soft key
";
        let km = Keymap::from_config(Some(text), Preset::Mobile).unwrap();
        assert_eq!(km.preset(), Preset::Standard);
        assert_eq!(km.nokia_code(K::KeyH), Some(nokia::SOFT_LEFT));
        assert_eq!(km.nokia_code(K::F1), None); // unbound by the override
        assert_eq!(km.nokia_code(K::ArrowUp), Some(nokia::UP)); // preset intact
    }

    #[test]
    fn from_config_str_defaults_to_mobile() {
        let km = Keymap::from_config_str("[keymap]\nKeyH = Fire\n").unwrap();
        assert_eq!(km.preset(), Preset::Mobile);
        assert_eq!(km.nokia_code(K::KeyH), Some(nokia::FIRE));
        assert_eq!(km.nokia_code(K::KeyA), Some(nokia::LEFT)); // mobile default kept
    }

    #[test]
    fn a_bad_config_surfaces_the_error() {
        assert!(Keymap::from_config_str("[keymap]\nBogusKey = Fire\n").is_err());
    }
}
