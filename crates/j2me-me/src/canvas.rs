//! MIDP Canvas/Display state and the serialized paint/input queue, plus the
//! [`Displayable`] surface trait.

use std::collections::VecDeque;

pub const UP: i32 = 1;
pub const LEFT: i32 = 2;
pub const RIGHT: i32 = 5;
pub const DOWN: i32 = 6;
pub const FIRE: i32 = 8;
pub const GAME_A: i32 = 9;
pub const GAME_B: i32 = 10;
pub const GAME_C: i32 = 11;
pub const GAME_D: i32 = 12;

pub const KEY_NUM0: i32 = 48;
pub const KEY_NUM1: i32 = 49;
pub const KEY_NUM2: i32 = 50;
pub const KEY_NUM3: i32 = 51;
pub const KEY_NUM4: i32 = 52;
pub const KEY_NUM5: i32 = 53;
pub const KEY_NUM6: i32 = 54;
pub const KEY_NUM7: i32 = 55;
pub const KEY_NUM8: i32 = 56;
pub const KEY_NUM9: i32 = 57;
pub const KEY_STAR: i32 = 42;
pub const KEY_POUND: i32 = 35;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CanvasEvent {
    Paint,
    KeyPressed(i32),
    KeyReleased(i32),
    KeyRepeated(i32),
}

#[derive(Debug)]
pub struct Canvas {
    width: i32,
    height: i32,
    shown: bool,
    full_screen: bool,
    repaint_owed: bool,
    input: VecDeque<CanvasEvent>,
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Self {
        assert!(
            width > 0 && height > 0,
            "Canvas dimensions must be positive"
        );
        Self {
            width,
            height,
            shown: false,
            full_screen: false,
            repaint_owed: false,
            input: VecDeque::new(),
        }
    }

    pub const fn width(&self) -> i32 {
        self.width
    }

    pub const fn height(&self) -> i32 {
        self.height
    }

    pub const fn is_shown(&self) -> bool {
        self.shown
    }

    pub const fn is_full_screen(&self) -> bool {
        self.full_screen
    }

    pub fn set_full_screen_mode(&mut self, enabled: bool) {
        self.full_screen = enabled;
    }

    pub fn request_repaint(&mut self) {
        self.repaint_owed = true;
    }

    pub const fn repaint_owed(&self) -> bool {
        self.repaint_owed
    }

    pub fn key_pressed(&mut self, code: i32) {
        self.input.push_back(CanvasEvent::KeyPressed(code));
    }

    pub fn key_released(&mut self, code: i32) {
        self.input.push_back(CanvasEvent::KeyReleased(code));
    }

    pub fn key_repeated(&mut self, code: i32) {
        self.input.push_back(CanvasEvent::KeyRepeated(code));
    }

    pub fn pending_input_len(&self) -> usize {
        self.input.len()
    }

    /// MIDP serializes callbacks. An owed repaint is dispatched before the next
    /// queued input event, and repeated repaint requests coalesce.
    pub fn poll_event(&mut self) -> Option<CanvasEvent> {
        if self.repaint_owed {
            self.repaint_owed = false;
            Some(CanvasEvent::Paint)
        } else {
            self.input.pop_front()
        }
    }

    #[must_use = "the host must paint when this returns true"]
    pub fn service_repaints(&mut self) -> bool {
        std::mem::take(&mut self.repaint_owed)
    }

    fn show_notify(&mut self) {
        self.shown = true;
        self.request_repaint();
    }

    fn hide_notify(&mut self) {
        self.shown = false;
    }

    /// Common keypad and d-pad mapping. Per-device overrides belong in the
    /// game's host adapter and must be oracle-backed.
    pub const fn common_game_action(code: i32) -> i32 {
        match code {
            KEY_NUM2 | -1 => UP,
            KEY_NUM8 | -2 => DOWN,
            KEY_NUM4 | -3 => LEFT,
            KEY_NUM6 | -4 => RIGHT,
            KEY_NUM5 | -5 => FIRE,
            _ => 0,
        }
    }
}

/// The base type of anything `Display.setCurrent` can show — the MIDP
/// `Displayable` surface.
pub trait Displayable {
    fn width(&self) -> i32;
    fn height(&self) -> i32;
}

impl Displayable for Canvas {
    fn width(&self) -> i32 {
        self.width
    }
    fn height(&self) -> i32 {
        self.height
    }
}

#[derive(Debug, Default)]
pub struct Display {
    has_current: bool,
}

impl Display {
    pub const fn has_current(&self) -> bool {
        self.has_current
    }

    pub fn set_current(&mut self, previous: Option<&mut Canvas>, next: &mut Canvas) {
        if let Some(previous) = previous {
            previous.hide_notify();
        }
        next.show_notify();
        self.has_current = true;
    }

    pub fn clear_current(&mut self, current: &mut Canvas) {
        current.hide_notify();
        self.has_current = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_is_serialized_before_input_and_coalesces() {
        let mut canvas = Canvas::new(128, 128);
        canvas.key_pressed(KEY_NUM5);
        canvas.request_repaint();
        canvas.request_repaint();
        assert_eq!(canvas.poll_event(), Some(CanvasEvent::Paint));
        assert_eq!(canvas.poll_event(), Some(CanvasEvent::KeyPressed(KEY_NUM5)));
        assert_eq!(canvas.poll_event(), None);
    }

    #[test]
    fn display_lifecycle_schedules_initial_paint() {
        let mut display = Display::default();
        let mut canvas = Canvas::new(176, 220);
        display.set_current(None, &mut canvas);
        assert!(display.has_current());
        assert!(canvas.is_shown());
        assert_eq!(canvas.poll_event(), Some(CanvasEvent::Paint));
        display.clear_current(&mut canvas);
        assert!(!canvas.is_shown());
    }
}

// Behavioral tests for the game-action map, the raw key queue, the serial
// paint/input ordering, and the `Display::set_current` / `clear_current` show
// and hide paths.
#[cfg(test)]
mod behavior_tests {
    use super::*;

    #[test]
    fn game_action_maps_keypad_and_dpad_and_zero_otherwise() {
        // ITU keypad.
        assert_eq!(Canvas::common_game_action(KEY_NUM2), UP);
        assert_eq!(Canvas::common_game_action(KEY_NUM8), DOWN);
        assert_eq!(Canvas::common_game_action(KEY_NUM4), LEFT);
        assert_eq!(Canvas::common_game_action(KEY_NUM6), RIGHT);
        assert_eq!(Canvas::common_game_action(KEY_NUM5), FIRE);
        // d-pad.
        assert_eq!(Canvas::common_game_action(-1), UP);
        assert_eq!(Canvas::common_game_action(-5), FIRE);
        // A code with no game action returns 0 (e.g. the remapped soft key -8,
        // and '#', which the game handles by raw code, not by action).
        assert_eq!(Canvas::common_game_action(-8), 0);
        assert_eq!(Canvas::common_game_action(KEY_POUND), 0);
        assert_eq!(Canvas::common_game_action(0), 0);
    }

    #[test]
    fn raw_key_codes_pass_through_untouched() {
        // The game remaps/branches on the raw code itself, so the queue must not
        // reject or rewrite a code — even the game-specific -8 the game synthesizes.
        let mut c = Canvas::new(240, 320);
        c.key_pressed(-8);
        c.key_released(53);
        assert_eq!(c.poll_event(), Some(CanvasEvent::KeyPressed(-8)));
        assert_eq!(c.poll_event(), Some(CanvasEvent::KeyReleased(53)));
        assert_eq!(c.poll_event(), None);
    }

    #[test]
    fn owed_paint_is_serviced_before_the_next_key() {
        // R9: a key delivered WHILE a repaint is owed is queued behind the paint.
        let mut c = Canvas::new(240, 320);
        c.request_repaint();
        c.key_pressed(KEY_NUM5); // arrives with a paint already owed

        // Perturbation control (R3): the key really is queued — if poll_event
        // drained input before the owed paint, this would be the key first.
        assert_eq!(c.pending_input_len(), 1);

        assert_eq!(c.poll_event(), Some(CanvasEvent::Paint));
        assert_eq!(c.poll_event(), Some(CanvasEvent::KeyPressed(KEY_NUM5)));
        assert_eq!(c.poll_event(), None);
    }

    #[test]
    fn multiple_repaints_coalesce_to_one_paint() {
        let mut c = Canvas::new(240, 320);
        c.request_repaint();
        c.request_repaint();
        c.request_repaint();
        assert_eq!(c.poll_event(), Some(CanvasEvent::Paint));
        assert_eq!(c.poll_event(), None); // only one paint owed
    }

    #[test]
    fn keys_dispatch_in_arrival_order_once_no_paint_is_owed() {
        let mut c = Canvas::new(240, 320);
        c.key_pressed(KEY_NUM4);
        c.key_released(KEY_NUM4);
        c.key_pressed(-5);
        assert_eq!(c.poll_event(), Some(CanvasEvent::KeyPressed(KEY_NUM4)));
        assert_eq!(c.poll_event(), Some(CanvasEvent::KeyReleased(KEY_NUM4)));
        assert_eq!(c.poll_event(), Some(CanvasEvent::KeyPressed(-5)));
        assert_eq!(c.poll_event(), None);
    }

    #[test]
    fn service_repaints_consumes_the_owed_paint_synchronously() {
        let mut c = Canvas::new(240, 320);
        c.request_repaint();
        assert!(c.service_repaints()); // a paint was due -> host paints now
        assert!(!c.repaint_owed());
        assert_eq!(c.poll_event(), None); // already serviced; no duplicate paint
        assert!(!c.service_repaints()); // nothing owed now
    }

    #[test]
    fn full_screen_mode_is_recorded() {
        let mut c = Canvas::new(240, 320);
        assert!(!c.is_full_screen());
        c.set_full_screen_mode(true);
        assert!(c.is_full_screen());
    }

    #[test]
    fn set_current_shows_and_schedules_the_first_paint() {
        let mut d = Display::default();
        let mut c = Canvas::new(176, 208);
        assert!(!c.is_shown());
        d.set_current(None, &mut c);
        assert!(d.has_current());
        assert!(c.is_shown());
        // The first serviced event after becoming current is the paint (R9),
        // even if a key was already waiting.
        c.key_pressed(KEY_NUM5);
        assert_eq!(c.poll_event(), Some(CanvasEvent::Paint));
        assert_eq!(c.poll_event(), Some(CanvasEvent::KeyPressed(KEY_NUM5)));
    }

    #[test]
    fn hide_notify_clears_shown() {
        let mut d = Display::default();
        let mut c = Canvas::new(240, 320);
        d.set_current(None, &mut c);
        assert!(c.is_shown());
        d.clear_current(&mut c);
        assert!(!c.is_shown());
    }
}
