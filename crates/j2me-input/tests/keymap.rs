//! End-to-end checks through the public API only — using the crate's re-exported
//! [`KeyCode`], so an integration test (or a real host) needs no direct `winit`
//! dependency of its own.

use j2me_input::{nokia, KeyCode, Keymap, Preset};

#[test]
fn standard_preset_maps_the_expected_keys() {
    let km = Keymap::new(Preset::Standard);
    assert_eq!(km.nokia_code(KeyCode::ArrowUp), Some(nokia::UP));
    assert_eq!(km.nokia_code(KeyCode::ArrowDown), Some(nokia::DOWN));
    assert_eq!(km.nokia_code(KeyCode::ArrowLeft), Some(nokia::LEFT));
    assert_eq!(km.nokia_code(KeyCode::ArrowRight), Some(nokia::RIGHT));
    assert_eq!(km.nokia_code(KeyCode::Enter), Some(nokia::FIRE));
    assert_eq!(km.nokia_code(KeyCode::Space), Some(nokia::FIRE));
    assert_eq!(km.nokia_code(KeyCode::F1), Some(nokia::SOFT_LEFT));
    assert_eq!(km.nokia_code(KeyCode::F2), Some(nokia::SOFT_RIGHT));
    assert_eq!(km.nokia_code(KeyCode::Digit3), Some(51));
    assert_eq!(km.nokia_code(KeyCode::NumpadMultiply), Some(nokia::STAR));
    assert_eq!(km.nokia_code(KeyCode::BracketRight), Some(nokia::POUND));
    // Standard leaves the mobile cluster alone.
    assert_eq!(km.nokia_code(KeyCode::KeyW), None);
}

#[test]
fn mobile_preset_covers_move_fire_softkeys_and_digits() {
    let km = Keymap::new(Preset::Mobile);
    // Move.
    assert_eq!(km.nokia_code(KeyCode::KeyW), Some(nokia::UP));
    assert_eq!(km.nokia_code(KeyCode::KeyA), Some(nokia::LEFT));
    assert_eq!(km.nokia_code(KeyCode::KeyS), Some(nokia::DOWN));
    assert_eq!(km.nokia_code(KeyCode::KeyD), Some(nokia::RIGHT));
    // Fire.
    assert_eq!(km.nokia_code(KeyCode::KeyX), Some(nokia::FIRE));
    // Soft keys.
    assert_eq!(km.nokia_code(KeyCode::KeyQ), Some(nokia::SOFT_LEFT));
    assert_eq!(km.nokia_code(KeyCode::KeyE), Some(nokia::SOFT_RIGHT));
    // Symbol keys and digits still work (superset of Standard).
    assert_eq!(km.nokia_code(KeyCode::KeyR), Some(nokia::STAR));
    assert_eq!(km.nokia_code(KeyCode::KeyF), Some(nokia::POUND));
    assert_eq!(km.nokia_code(KeyCode::Digit0), Some(48));
    assert_eq!(km.nokia_code(KeyCode::Digit9), Some(57));
}

#[test]
fn a_config_override_wins_over_the_preset() {
    // Bind H to Fire and unbind Q, on top of the Standard preset.
    let text = "\
[keymap]
preset = \"standard\"
KeyH = Fire
KeyQ = none
";
    let km = Keymap::from_config(Some(text), Preset::Mobile).unwrap();
    assert_eq!(km.preset(), Preset::Standard);
    assert_eq!(km.nokia_code(KeyCode::KeyH), Some(nokia::FIRE));
    assert_eq!(km.nokia_code(KeyCode::KeyQ), None);
    // Preset bindings the config didn't touch are intact.
    assert_eq!(km.nokia_code(KeyCode::ArrowUp), Some(nokia::UP));
}

#[test]
fn an_unmapped_key_yields_none() {
    let km = Keymap::from_config_str("[keymap]\nKeyH = Fire\n").unwrap();
    assert_eq!(km.nokia_code(KeyCode::KeyN), None);
}

#[test]
fn an_invalid_config_is_rejected() {
    assert!(Keymap::from_config_str("[keymap]\nWobble = Fire\n").is_err());
    assert!(Keymap::from_config_str("[keymap]\nKeyH = 12345\n").is_err());
}
