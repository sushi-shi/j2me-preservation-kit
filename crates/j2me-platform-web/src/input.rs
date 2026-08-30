use j2me_device::HandsetKey;

/// Browser `KeyboardEvent.code` to the same semantic vocabulary used by the
/// native keymap. The device profile performs the second raw-code hop.
pub fn handset_key(code: &str) -> Option<HandsetKey> {
    Some(match code {
        "ArrowUp" | "KeyW" => HandsetKey::Up,
        "ArrowDown" | "KeyS" => HandsetKey::Down,
        "ArrowLeft" | "KeyA" => HandsetKey::Left,
        "ArrowRight" | "KeyD" => HandsetKey::Right,
        "Enter" | "Space" | "NumpadEnter" | "KeyX" => HandsetKey::Fire,
        "F1" | "KeyQ" => HandsetKey::SoftLeft,
        "F2" | "KeyE" => HandsetKey::SoftRight,
        "KeyR" | "NumpadMultiply" => HandsetKey::Star,
        "KeyF" => HandsetKey::Pound,
        code if code.len() == 6 && code.starts_with("Digit") => HandsetKey::Digit(
            code.as_bytes()[5]
                .checked_sub(b'0')
                .filter(|digit| *digit <= 9)?,
        ),
        code if code.len() == 7 && code.starts_with("Numpad") => HandsetKey::Digit(
            code.as_bytes()[6]
                .checked_sub(b'0')
                .filter(|digit| *digit <= 9)?,
        ),
        _ => return None,
    })
}

pub fn raw_code(code: &str, profile: &j2me_device::InputFragment) -> Option<i32> {
    profile.key_code(handset_key(code)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_mapping_stops_before_phone_codes() {
        assert_eq!(handset_key("KeyW"), Some(HandsetKey::Up));
        assert_eq!(handset_key("Digit7"), Some(HandsetKey::Digit(7)));
        assert_eq!(handset_key("Escape"), None);
    }
}
