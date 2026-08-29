//! Built-in keyboard presets and the key-name parser used by config overrides.
//!
//! Both presets map a physical [`KeyCode`] to the SAME raw Nokia codes the phone
//! itself sent, so the game is driven identically either way — the mobile cluster
//! is host ergonomics, not a behavioural change.
//!
//! | keyboard                               | Nokia key      | code    |
//! |----------------------------------------|----------------|---------|
//! | Arrow Up/Down/Left/Right               | D-pad          | -1..=-4 |
//! | Enter, Space, Numpad Enter, Numpad 5   | Fire / select  | -5      |
//! | F1 / F2                                | soft keys      | -6/-7   |
//! | 0-9 (top row or numpad)                | number keys    | 48..=57 |
//! | Numpad `*` / `[`                       | star `*`       | 42      |
//! | `]` / `\`                              | pound `#`      | 35      |
//!
//! [`Preset::Mobile`] adds a left-hand cluster on top of every binding above
//! (mirroring gothic-mobile's and stalker-mobile's keymaps so the ports feel the
//! same): W/A/S/D move, Q/E are the soft keys, X fires/confirms, and R/F reach
//! the phone's `*`/`#`. It is a strict superset — every Standard binding still
//! works — so it is the library default.
//!
//! | mobile add-on | Nokia key    | code |
//! |---------------|--------------|------|
//! | W / S / A / D | D-pad        | -1/-2/-3/-4 |
//! | X             | Fire         | -5   |
//! | Q / E         | soft keys    | -6/-7 |
//! | R             | star `*`     | 42   |
//! | F             | pound `#`    | 35   |

use crate::nokia;
use winit::keyboard::KeyCode;

/// A built-in set of default bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Preset {
    /// Arrows for the D-pad, Enter/Space for Fire, F1/F2 for the soft keys,
    /// number row + numpad for the digits, brackets for `*`/`#`.
    Standard,
    /// Everything in [`Standard`](Preset::Standard) plus the WASD/Q-E/X/R-F
    /// mobile cluster. A strict superset, and the default.
    #[default]
    Mobile,
}

impl Preset {
    /// The raw Nokia code this preset assigns to `key`, or `None` if the preset
    /// leaves the key unbound.
    pub fn nokia_code(self, key: KeyCode) -> Option<i32> {
        match self {
            Preset::Standard => standard(key),
            Preset::Mobile => mobile_cluster(key).or_else(|| standard(key)),
        }
    }

    /// Parse a preset name (case-insensitive): `standard`/`default`/`desktop`/
    /// `arrows` -> [`Standard`](Preset::Standard); `mobile`/`wasd`/`phone` ->
    /// [`Mobile`](Preset::Mobile). Returns `None` for anything else.
    pub fn from_name(name: &str) -> Option<Preset> {
        match name.trim().to_ascii_lowercase().as_str() {
            "standard" | "default" | "desktop" | "arrows" => Some(Preset::Standard),
            "mobile" | "wasd" | "phone" => Some(Preset::Mobile),
            _ => None,
        }
    }
}

/// The arrows/Enter/F-key/digit/bracket base shared by both presets.
fn standard(key: KeyCode) -> Option<i32> {
    use KeyCode as K;
    Some(match key {
        K::ArrowUp => nokia::UP,
        K::ArrowDown => nokia::DOWN,
        K::ArrowLeft => nokia::LEFT,
        K::ArrowRight => nokia::RIGHT,

        K::Enter | K::NumpadEnter | K::Space | K::Numpad5 => nokia::FIRE,

        K::F1 => nokia::SOFT_LEFT,
        K::F2 => nokia::SOFT_RIGHT,

        K::Digit0 | K::Numpad0 => 48,
        K::Digit1 | K::Numpad1 => 49,
        K::Digit2 | K::Numpad2 => 50,
        K::Digit3 | K::Numpad3 => 51,
        K::Digit4 | K::Numpad4 => 52,
        // Numpad5 is Fire above; the top-row 5 stays a digit.
        K::Digit5 => 53,
        K::Digit6 | K::Numpad6 => 54,
        K::Digit7 | K::Numpad7 => 55,
        K::Digit8 | K::Numpad8 => 56,
        K::Digit9 | K::Numpad9 => 57,

        K::NumpadMultiply | K::BracketLeft => nokia::STAR,
        K::BracketRight | K::Backslash => nokia::POUND,

        _ => return None,
    })
}

/// The mobile-only left-hand cluster layered over [`standard`].
fn mobile_cluster(key: KeyCode) -> Option<i32> {
    use KeyCode as K;
    Some(match key {
        K::KeyW => nokia::UP,
        K::KeyS => nokia::DOWN,
        K::KeyA => nokia::LEFT,
        K::KeyD => nokia::RIGHT,
        K::KeyX => nokia::FIRE,
        K::KeyQ => nokia::SOFT_LEFT,
        K::KeyE => nokia::SOFT_RIGHT,
        K::KeyR => nokia::STAR,
        K::KeyF => nokia::POUND,
        _ => return None,
    })
}

/// Resolve a key name from a config file to a physical [`KeyCode`].
///
/// Accepts winit variant names (`KeyW`, `ArrowUp`, `F1`, `NumpadMultiply`, ...)
/// and friendly aliases (`W`, `Up`, `Space`, `Esc`, `0`, `KP3`, ...). Separators
/// `_`, `-`, and spaces are ignored and matching is case-insensitive. Returns
/// `None` for an unrecognised name (the caller reports it as a config error).
pub fn key_from_name(name: &str) -> Option<KeyCode> {
    let canon: String = name
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | ' '))
        .flat_map(char::to_lowercase)
        .collect();

    // Single character: a letter or a digit.
    if canon.len() == 1 {
        let b = canon.as_bytes()[0];
        if b.is_ascii_lowercase() {
            return letter_key(b);
        }
        if b.is_ascii_digit() {
            return digit_key(b - b'0');
        }
    }

    // `keyW`.
    if let Some(rest) = canon.strip_prefix("key") {
        if rest.len() == 1 && rest.as_bytes()[0].is_ascii_lowercase() {
            return letter_key(rest.as_bytes()[0]);
        }
    }
    // `digit3`, `numpad3` / `kp3`.
    if let Some(n) = canon.strip_prefix("digit").and_then(single_digit) {
        return digit_key(n);
    }
    if let Some(n) = canon
        .strip_prefix("numpad")
        .and_then(single_digit)
        .or_else(|| canon.strip_prefix("kp").and_then(single_digit))
    {
        return numpad_key(n);
    }
    // `f1`..`f12`.
    if let Some(rest) = canon.strip_prefix('f') {
        if let Some(fk) = function_key(rest) {
            return Some(fk);
        }
    }

    use KeyCode as K;
    Some(match canon.as_str() {
        "arrowup" | "up" => K::ArrowUp,
        "arrowdown" | "down" => K::ArrowDown,
        "arrowleft" | "left" => K::ArrowLeft,
        "arrowright" | "right" => K::ArrowRight,
        "space" | "spacebar" => K::Space,
        "enter" | "return" => K::Enter,
        "numpadenter" | "kpenter" => K::NumpadEnter,
        "escape" | "esc" => K::Escape,
        "tab" => K::Tab,
        "backspace" => K::Backspace,
        "delete" | "del" => K::Delete,
        "insert" | "ins" => K::Insert,
        "home" => K::Home,
        "end" => K::End,
        "pageup" | "pgup" => K::PageUp,
        "pagedown" | "pgdn" => K::PageDown,
        "numpadmultiply" | "numpadstar" | "kpmultiply" | "kpstar" => K::NumpadMultiply,
        "numpadadd" | "numpadplus" | "kpadd" | "kpplus" => K::NumpadAdd,
        "numpadsubtract" | "numpadminus" | "kpsubtract" | "kpminus" => K::NumpadSubtract,
        "numpaddivide" | "kpdivide" => K::NumpadDivide,
        "numpaddecimal" | "kpdecimal" => K::NumpadDecimal,
        "bracketleft" | "leftbracket" | "lbracket" | "openbracket" => K::BracketLeft,
        "bracketright" | "rightbracket" | "rbracket" | "closebracket" => K::BracketRight,
        "backslash" => K::Backslash,
        "slash" => K::Slash,
        "semicolon" => K::Semicolon,
        "quote" | "apostrophe" => K::Quote,
        "comma" => K::Comma,
        "period" | "dot" => K::Period,
        "minus" | "dash" => K::Minus,
        "equal" | "equals" => K::Equal,
        "backquote" | "grave" | "backtick" => K::Backquote,
        "shiftleft" | "lshift" | "leftshift" => K::ShiftLeft,
        "shiftright" | "rshift" | "rightshift" => K::ShiftRight,
        "controlleft" | "lctrl" | "lcontrol" | "leftctrl" => K::ControlLeft,
        "controlright" | "rctrl" | "rcontrol" | "rightctrl" => K::ControlRight,
        "altleft" | "lalt" | "leftalt" => K::AltLeft,
        "altright" | "ralt" | "rightalt" => K::AltRight,
        _ => return None,
    })
}

/// `Some(n)` iff `s` is exactly one ASCII decimal digit.
fn single_digit(s: &str) -> Option<u8> {
    let b = s.as_bytes();
    if b.len() == 1 && b[0].is_ascii_digit() {
        Some(b[0] - b'0')
    } else {
        None
    }
}

/// `f1`..`f12` -> the corresponding function key.
fn function_key(rest: &str) -> Option<KeyCode> {
    use KeyCode as K;
    Some(match rest.parse::<u8>().ok()? {
        1 => K::F1,
        2 => K::F2,
        3 => K::F3,
        4 => K::F4,
        5 => K::F5,
        6 => K::F6,
        7 => K::F7,
        8 => K::F8,
        9 => K::F9,
        10 => K::F10,
        11 => K::F11,
        12 => K::F12,
        _ => return None,
    })
}

/// ASCII lowercase letter byte -> `KeyCode::Key*`.
fn letter_key(b: u8) -> Option<KeyCode> {
    use KeyCode as K;
    Some(match b {
        b'a' => K::KeyA,
        b'b' => K::KeyB,
        b'c' => K::KeyC,
        b'd' => K::KeyD,
        b'e' => K::KeyE,
        b'f' => K::KeyF,
        b'g' => K::KeyG,
        b'h' => K::KeyH,
        b'i' => K::KeyI,
        b'j' => K::KeyJ,
        b'k' => K::KeyK,
        b'l' => K::KeyL,
        b'm' => K::KeyM,
        b'n' => K::KeyN,
        b'o' => K::KeyO,
        b'p' => K::KeyP,
        b'q' => K::KeyQ,
        b'r' => K::KeyR,
        b's' => K::KeyS,
        b't' => K::KeyT,
        b'u' => K::KeyU,
        b'v' => K::KeyV,
        b'w' => K::KeyW,
        b'x' => K::KeyX,
        b'y' => K::KeyY,
        b'z' => K::KeyZ,
        _ => return None,
    })
}

/// `0..=9` -> the top-row `KeyCode::Digit*`.
fn digit_key(n: u8) -> Option<KeyCode> {
    use KeyCode as K;
    Some(match n {
        0 => K::Digit0,
        1 => K::Digit1,
        2 => K::Digit2,
        3 => K::Digit3,
        4 => K::Digit4,
        5 => K::Digit5,
        6 => K::Digit6,
        7 => K::Digit7,
        8 => K::Digit8,
        9 => K::Digit9,
        _ => return None,
    })
}

/// `0..=9` -> the `KeyCode::Numpad*` equivalent.
fn numpad_key(n: u8) -> Option<KeyCode> {
    use KeyCode as K;
    Some(match n {
        0 => K::Numpad0,
        1 => K::Numpad1,
        2 => K::Numpad2,
        3 => K::Numpad3,
        4 => K::Numpad4,
        5 => K::Numpad5,
        6 => K::Numpad6,
        7 => K::Numpad7,
        8 => K::Numpad8,
        9 => K::Numpad9,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode as K;

    #[test]
    fn standard_maps_arrows_enter_softkeys_and_symbols() {
        let p = Preset::Standard;
        assert_eq!(p.nokia_code(K::ArrowUp), Some(nokia::UP));
        assert_eq!(p.nokia_code(K::ArrowDown), Some(nokia::DOWN));
        assert_eq!(p.nokia_code(K::ArrowLeft), Some(nokia::LEFT));
        assert_eq!(p.nokia_code(K::ArrowRight), Some(nokia::RIGHT));
        assert_eq!(p.nokia_code(K::Enter), Some(nokia::FIRE));
        assert_eq!(p.nokia_code(K::Space), Some(nokia::FIRE));
        assert_eq!(p.nokia_code(K::F1), Some(nokia::SOFT_LEFT));
        assert_eq!(p.nokia_code(K::F2), Some(nokia::SOFT_RIGHT));
        assert_eq!(p.nokia_code(K::Digit0), Some(48));
        assert_eq!(p.nokia_code(K::Digit9), Some(57));
        assert_eq!(p.nokia_code(K::BracketLeft), Some(nokia::STAR));
        assert_eq!(p.nokia_code(K::BracketRight), Some(nokia::POUND));
    }

    #[test]
    fn standard_leaves_the_mobile_cluster_unbound() {
        let p = Preset::Standard;
        for k in [
            K::KeyW,
            K::KeyA,
            K::KeyS,
            K::KeyD,
            K::KeyQ,
            K::KeyE,
            K::KeyX,
        ] {
            assert_eq!(p.nokia_code(k), None, "{k:?} must be unbound in Standard");
        }
    }

    #[test]
    fn mobile_covers_move_fire_softkeys_and_symbols() {
        let p = Preset::Mobile;
        // Movement.
        assert_eq!(p.nokia_code(K::KeyW), Some(nokia::UP));
        assert_eq!(p.nokia_code(K::KeyS), Some(nokia::DOWN));
        assert_eq!(p.nokia_code(K::KeyA), Some(nokia::LEFT));
        assert_eq!(p.nokia_code(K::KeyD), Some(nokia::RIGHT));
        // Fire.
        assert_eq!(p.nokia_code(K::KeyX), Some(nokia::FIRE));
        // Soft keys.
        assert_eq!(p.nokia_code(K::KeyQ), Some(nokia::SOFT_LEFT));
        assert_eq!(p.nokia_code(K::KeyE), Some(nokia::SOFT_RIGHT));
        // Symbol keys.
        assert_eq!(p.nokia_code(K::KeyR), Some(nokia::STAR));
        assert_eq!(p.nokia_code(K::KeyF), Some(nokia::POUND));
    }

    #[test]
    fn mobile_is_a_strict_superset_of_standard() {
        // Every arrow/enter/etc. binding still resolves under Mobile.
        for k in [K::ArrowUp, K::Enter, K::Space, K::F1, K::F2, K::Digit5] {
            assert_eq!(Preset::Mobile.nokia_code(k), Preset::Standard.nokia_code(k));
        }
    }

    #[test]
    fn unmapped_key_yields_none_in_both_presets() {
        for p in [Preset::Standard, Preset::Mobile] {
            assert_eq!(p.nokia_code(K::KeyI), None);
            assert_eq!(p.nokia_code(K::Home), None);
        }
    }

    #[test]
    fn preset_names_parse() {
        assert_eq!(Preset::from_name("standard"), Some(Preset::Standard));
        assert_eq!(Preset::from_name("Desktop"), Some(Preset::Standard));
        assert_eq!(Preset::from_name("MOBILE"), Some(Preset::Mobile));
        assert_eq!(Preset::from_name("wasd"), Some(Preset::Mobile));
        assert_eq!(Preset::from_name("xyzzy"), None);
        assert_eq!(Preset::default(), Preset::Mobile);
    }

    #[test]
    fn key_names_parse_variants_and_aliases() {
        assert_eq!(key_from_name("KeyW"), Some(K::KeyW));
        assert_eq!(key_from_name("w"), Some(K::KeyW));
        assert_eq!(key_from_name("W"), Some(K::KeyW));
        assert_eq!(key_from_name("ArrowUp"), Some(K::ArrowUp));
        assert_eq!(key_from_name("up"), Some(K::ArrowUp));
        assert_eq!(key_from_name("arrow_up"), Some(K::ArrowUp));
        assert_eq!(key_from_name("F1"), Some(K::F1));
        assert_eq!(key_from_name("f11"), Some(K::F11));
        assert_eq!(key_from_name("Space"), Some(K::Space));
        assert_eq!(key_from_name("esc"), Some(K::Escape));
        assert_eq!(key_from_name("Digit0"), Some(K::Digit0));
        assert_eq!(key_from_name("0"), Some(K::Digit0));
        assert_eq!(key_from_name("Numpad5"), Some(K::Numpad5));
        assert_eq!(key_from_name("kp5"), Some(K::Numpad5));
        assert_eq!(key_from_name("BracketLeft"), Some(K::BracketLeft));
        assert_eq!(key_from_name("NumpadMultiply"), Some(K::NumpadMultiply));
        assert_eq!(key_from_name("f"), Some(K::KeyF));
        assert_eq!(key_from_name("totally-unknown"), None);
    }
}
