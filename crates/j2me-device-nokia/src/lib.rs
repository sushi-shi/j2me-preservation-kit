//! Nokia-specific handset behavior. Nothing in `j2me-me` implicitly selects it.

use j2me_device::{HandsetKey, InputFragment};

pub const UP: i32 = -1;
pub const DOWN: i32 = -2;
pub const LEFT: i32 = -3;
pub const RIGHT: i32 = -4;
pub const FIRE: i32 = -5;
pub const SOFT_LEFT: i32 = -6;
pub const SOFT_RIGHT: i32 = -7;
pub const STAR: i32 = 42;
pub const POUND: i32 = 35;

pub const fn digit(n: u8) -> Option<i32> {
    if n <= 9 {
        Some(48 + n as i32)
    } else {
        None
    }
}

pub const fn key_code(key: HandsetKey) -> Option<i32> {
    Some(match key {
        HandsetKey::Up => UP,
        HandsetKey::Down => DOWN,
        HandsetKey::Left => LEFT,
        HandsetKey::Right => RIGHT,
        HandsetKey::Fire => FIRE,
        HandsetKey::SoftLeft => SOFT_LEFT,
        HandsetKey::SoftRight => SOFT_RIGHT,
        HandsetKey::Digit(n) => match digit(n) {
            Some(value) => value,
            None => return None,
        },
        HandsetKey::Star => STAR,
        HandsetKey::Pound => POUND,
    })
}

pub const fn is_key_code(code: i32) -> bool {
    matches!(
        code,
        UP | DOWN | LEFT | RIGHT | FIRE | SOFT_LEFT | SOFT_RIGHT | STAR | POUND
    ) || (48 <= code && code <= 57)
}

/// Series 60 keypad semantics: navigation plus 2/8/4/6/5 as game actions.
pub const fn game_action(code: i32) -> i32 {
    match code {
        UP | 50 => 1,
        LEFT | 52 => 2,
        RIGHT | 54 => 5,
        DOWN | 56 => 6,
        FIRE | 53 => 8,
        _ => 0,
    }
}

/// A composable fragment for game-owned profile catalogs.
pub fn input_fragment() -> InputFragment {
    InputFragment {
        up: UP,
        down: DOWN,
        left: LEFT,
        right: RIGHT,
        fire: FIRE,
        soft_left: SOFT_LEFT,
        soft_right: SOFT_RIGHT,
        star: STAR,
        pound: POUND,
        digits: [48, 49, 50, 51, 52, 53, 54, 55, 56, 57],
        game_action_up: vec![UP, 50],
        game_action_down: vec![DOWN, 56],
        game_action_left: vec![LEFT, 52],
        game_action_right: vec![RIGHT, 54],
        game_action_fire: vec![FIRE, 53],
        pointer_events: false,
        pointer_motion_events: false,
        repeat_events: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implementation_and_fragment_agree() {
        let fragment = input_fragment();
        for key in [HandsetKey::Up, HandsetKey::Fire, HandsetKey::Digit(5)] {
            assert_eq!(fragment.key_code(key), key_code(key));
        }
        for code in [UP, DOWN, LEFT, RIGHT, FIRE, 50, 52, 53, 54, 56, SOFT_LEFT] {
            assert_eq!(fragment.game_action(code), game_action(code));
        }
    }
}
