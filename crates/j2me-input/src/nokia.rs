//! The raw Nokia `FullCanvas` key vocabulary the transliterated `keyPressed(int)`
//! receives, plus a small [`Action`] name so config files can say `Fire` instead
//! of `-5`.
//!
//! These codes are **game-agnostic and fixed** by the phone's `FullCanvas`: the
//! four D-pad directions are `-1..=-4`, Fire/select is `-5`, the left and right
//! soft keys are `-6`/`-7`, the digits `0..=9` are their ASCII values `48..=57`,
//! and the two symbol keys `*` and `#` are `42` and `35`. Anything outside this
//! set is rejected by the device event queue (R10), so a caller treats an
//! out-of-vocabulary code the same as an unmapped key.

/// D-pad up (`FullCanvas` `UP`).
pub const UP: i32 = -1;
/// D-pad down.
pub const DOWN: i32 = -2;
/// D-pad left.
pub const LEFT: i32 = -3;
/// D-pad right.
pub const RIGHT: i32 = -4;
/// Fire / centre-select.
pub const FIRE: i32 = -5;
/// Left soft key (`softkey1`).
pub const SOFT_LEFT: i32 = -6;
/// Right soft key (`softkey2`).
pub const SOFT_RIGHT: i32 = -7;
/// Star `*` key (ASCII `42`).
pub const STAR: i32 = 42;
/// Pound `#` key (ASCII `35`).
pub const POUND: i32 = 35;

/// The Nokia code for decimal digit `n` as its ASCII value (`0` -> `48` ..
/// `9` -> `57`). Returns `None` for `n > 9`.
pub const fn digit(n: u8) -> Option<i32> {
    if n <= 9 {
        Some(48 + n as i32)
    } else {
        None
    }
}

/// Is `code` a member of the fixed Nokia key vocabulary the device queue accepts?
///
/// The vocabulary is the D-pad + Fire + two soft keys (`-1..=-7`), the ten digit
/// keys (`48..=57`), and the two symbol keys `*` (`42`) and `#` (`35`). Used to
/// reject a bogus numeric override before it ever reaches the (R10) queue.
pub const fn is_vocabulary(code: i32) -> bool {
    matches!(
        code,
        UP | DOWN | LEFT | RIGHT | FIRE | SOFT_LEFT | SOFT_RIGHT | STAR | POUND
    ) || (48 <= code && code <= 57)
}

/// A human-readable name for a Nokia key, so a `[keymap]` table can bind a key to
/// `Fire` or `SoftLeft` rather than the raw `-5` / `-6`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// D-pad up.
    Up,
    /// D-pad down.
    Down,
    /// D-pad left.
    Left,
    /// D-pad right.
    Right,
    /// Fire / select.
    Fire,
    /// Left soft key.
    SoftLeft,
    /// Right soft key.
    SoftRight,
    /// A number key `0..=9` (invariant: the payload is always `<= 9`).
    Digit(u8),
    /// The `*` key.
    Star,
    /// The `#` key.
    Pound,
}

impl Action {
    /// The raw Nokia code this action delivers.
    pub const fn nokia_code(self) -> i32 {
        match self {
            Action::Up => UP,
            Action::Down => DOWN,
            Action::Left => LEFT,
            Action::Right => RIGHT,
            Action::Fire => FIRE,
            Action::SoftLeft => SOFT_LEFT,
            Action::SoftRight => SOFT_RIGHT,
            // `from_name` is the only constructor and it constrains `n <= 9`.
            Action::Digit(n) => 48 + (n as i32),
            Action::Star => STAR,
            Action::Pound => POUND,
        }
    }

    /// Parse an action name (case-insensitive, `_`/`-`/spaces ignored):
    ///
    /// - `up` / `down` / `left` / `right`
    /// - `fire` / `select` / `ok`
    /// - `softleft` / `soft_left` / `softkey1` / `soft1` / `lsk` / `leftsoft`
    /// - `softright` / `soft_right` / `softkey2` / `soft2` / `rsk` / `rightsoft`
    /// - a digit: `3`, `digit3`, `num3`, `number3`
    /// - `star` / `asterisk` / `*`
    /// - `pound` / `hash` / `#`
    ///
    /// Returns `None` for anything else.
    pub fn from_name(name: &str) -> Option<Action> {
        let raw = name.trim();
        // Symbol spellings are matched before case-folding (they carry no case).
        match raw {
            "*" => return Some(Action::Star),
            "#" => return Some(Action::Pound),
            _ => {}
        }
        let canon: String = raw
            .chars()
            .filter(|c| !matches!(c, '_' | '-' | ' '))
            .flat_map(char::to_lowercase)
            .collect();

        // A bare or prefixed single digit.
        if let Some(n) = one_digit(&canon)
            .or_else(|| canon.strip_prefix("digit").and_then(one_digit))
            .or_else(|| canon.strip_prefix("number").and_then(one_digit))
            .or_else(|| canon.strip_prefix("num").and_then(one_digit))
        {
            return Some(Action::Digit(n));
        }

        Some(match canon.as_str() {
            "up" => Action::Up,
            "down" => Action::Down,
            "left" => Action::Left,
            "right" => Action::Right,
            "fire" | "select" | "ok" => Action::Fire,
            "softleft" | "softkey1" | "soft1" | "lsk" | "leftsoft" => Action::SoftLeft,
            "softright" | "softkey2" | "soft2" | "rsk" | "rightsoft" => Action::SoftRight,
            "star" | "asterisk" => Action::Star,
            "pound" | "hash" | "hashtag" | "octothorpe" => Action::Pound,
            _ => return None,
        })
    }
}

/// `Some(n)` iff `s` is exactly one ASCII decimal digit.
fn one_digit(s: &str) -> Option<u8> {
    let b = s.as_bytes();
    if b.len() == 1 && b[0].is_ascii_digit() {
        Some(b[0] - b'0')
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_helper_maps_ascii_and_rejects_overflow() {
        assert_eq!(digit(0), Some(48));
        assert_eq!(digit(9), Some(57));
        assert_eq!(digit(10), None);
    }

    #[test]
    fn vocabulary_admits_exactly_the_nokia_set() {
        for c in [
            UP, DOWN, LEFT, RIGHT, FIRE, SOFT_LEFT, SOFT_RIGHT, STAR, POUND,
        ] {
            assert!(is_vocabulary(c), "{c} should be in vocabulary");
        }
        for c in 48..=57 {
            assert!(is_vocabulary(c));
        }
        // Just outside the vocabulary in every direction.
        for c in [-8, 0, 1, 8, 47, 58, 34, 36, 41, 43, 999] {
            assert!(!is_vocabulary(c), "{c} should NOT be in vocabulary");
        }
    }

    #[test]
    fn action_codes_match_the_constants() {
        assert_eq!(Action::Fire.nokia_code(), FIRE);
        assert_eq!(Action::SoftLeft.nokia_code(), SOFT_LEFT);
        assert_eq!(Action::SoftRight.nokia_code(), SOFT_RIGHT);
        assert_eq!(Action::Digit(0).nokia_code(), 48);
        assert_eq!(Action::Digit(7).nokia_code(), 55);
        assert_eq!(Action::Star.nokia_code(), 42);
        assert_eq!(Action::Pound.nokia_code(), 35);
    }

    #[test]
    fn action_names_parse_with_aliases() {
        assert_eq!(Action::from_name("Fire"), Some(Action::Fire));
        assert_eq!(Action::from_name("select"), Some(Action::Fire));
        assert_eq!(Action::from_name("soft_left"), Some(Action::SoftLeft));
        assert_eq!(Action::from_name("SOFTKEY2"), Some(Action::SoftRight));
        assert_eq!(Action::from_name("digit3"), Some(Action::Digit(3)));
        assert_eq!(Action::from_name("7"), Some(Action::Digit(7)));
        assert_eq!(Action::from_name("*"), Some(Action::Star));
        assert_eq!(Action::from_name("#"), Some(Action::Pound));
        assert_eq!(Action::from_name("pound"), Some(Action::Pound));
        assert_eq!(Action::from_name("nonsense"), None);
        // A two-digit "digit" is not a single number key.
        assert_eq!(Action::from_name("digit42"), None);
    }
}
