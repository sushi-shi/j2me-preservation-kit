//! MIDP Canvas/Display state and the serialized paint/input queue, plus the
//! [`Displayable`] surface trait and ordered subclass visibility callbacks.

use std::collections::VecDeque;

use j2me_jvm::JavaError;

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

// Negative navigation and soft-key device codes. MIDP does not standardize the
// concrete integers a handset reports for its navigation cluster or soft keys —
// they are device policy — but this negative-code assignment is the one Nokia's
// Series 60 uses and the one every port in this collection has assumed, so it is
// the shared default the device tables below are written against.
pub const NAV_UP: i32 = -1;
pub const NAV_DOWN: i32 = -2;
pub const NAV_LEFT: i32 = -3;
pub const NAV_RIGHT: i32 = -4;
pub const NAV_FIRE: i32 = -5;
pub const SOFT_LEFT: i32 = -6;
pub const SOFT_RIGHT: i32 = -7;

/// A device's `Canvas.getGameAction` key-to-action table: it answers the game
/// action for a raw key code, or `0` for a key with no game action, exactly as
/// MIDP defines the mapping to be device-supplied. [`midp_default_game_action`]
/// and [`nokia_game_action`] are the two tables this kit ships; a host may pass
/// any closure to model another handset.
pub type DeviceGameActionTable<'a> = &'a dyn Fn(i32) -> i32;

/// `Canvas.getGameAction(int)`.
///
/// MIDP defines this mapping entirely by the device — the standard only says a
/// key *may* correspond to a game action — so the resolver carries no table of
/// its own and consults the one the host supplies. Keys the table does not map
/// (typically the soft keys, `*`, `#`) return `0`, the MIDP "no game action"
/// value. A game whose own `Canvas` subclass overrides `getGameAction` (e.g. to
/// give the soft keys a private action) layers that override on top of this in
/// the game body; it is not part of the device-neutral mapping.
pub fn get_game_action(key_code: i32, device_table: DeviceGameActionTable<'_>) -> i32 {
    device_table(key_code)
}

/// The MIDP navigation-cluster default: the abstract UP/DOWN/LEFT/RIGHT/FIRE
/// game actions on the shared negative navigation codes, and nothing else. A
/// handset that maps only its d-pad (not the keypad) uses this table.
pub const fn midp_default_game_action(code: i32) -> i32 {
    match code {
        NAV_UP => UP,
        NAV_DOWN => DOWN,
        NAV_LEFT => LEFT,
        NAV_RIGHT => RIGHT,
        NAV_FIRE => FIRE,
        _ => 0,
    }
}

/// Nokia Series 60 (e.g. the N70): the navigation cluster **plus** the ITU
/// keypad, whose physical layout doubles as a d-pad — 2 up, 8 down, 4 left,
/// 6 right, 5 fire.
pub const fn nokia_game_action(code: i32) -> i32 {
    match code {
        NAV_UP | KEY_NUM2 => UP,
        NAV_DOWN | KEY_NUM8 => DOWN,
        NAV_LEFT | KEY_NUM4 => LEFT,
        NAV_RIGHT | KEY_NUM6 => RIGHT,
        NAV_FIRE | KEY_NUM5 => FIRE,
        _ => 0,
    }
}

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

    /// Common keypad and d-pad mapping. A convenience alias for the
    /// [`nokia_game_action`] device table (the single owner of the mapping);
    /// per-device overrides belong in the game's host adapter and must be
    /// oracle-backed.
    pub const fn common_game_action(code: i32) -> i32 {
        nokia_game_action(code)
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

/// A device-visible operation requested through `Display`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostDisplayOp {
    /// `Display.vibrate(duration)` on a vibration-capable device.
    Vibrate { duration_ms: i32 },
}

#[derive(Debug, Default)]
pub struct Display {
    has_current: bool,
    vibration_supported: bool,
    host_ops: Vec<HostDisplayOp>,
}

impl Display {
    pub fn with_vibration_support(vibration_supported: bool) -> Self {
        Self {
            vibration_supported,
            ..Self::default()
        }
    }

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

    /// `Display.setCurrent` with the protected Canvas subclass notifications
    /// exposed to a strict transliteration.
    ///
    /// MIDP changes the display state around `hideNotify` / `showNotify`; the
    /// callbacks themselves belong to the game subclass. The old Canvas is
    /// marked hidden before `hide_notify` runs. Only after that callback
    /// succeeds is the new Canvas marked shown and `show_notify` invoked. This
    /// fixes both the successful order and observable failure cut points
    /// without embedding any game policy in the runtime.
    ///
    /// A failing hide leaves the old Canvas hidden, `has_current == false`, and
    /// the new Canvas untouched. A failing show leaves the new Canvas shown and
    /// current, because the generic visibility transition already occurred.
    pub fn set_current_notifying<HideNotify, ShowNotify, E>(
        &mut self,
        previous: Option<&mut Canvas>,
        next: &mut Canvas,
        hide_notify: HideNotify,
        show_notify: ShowNotify,
    ) -> Result<(), E>
    where
        HideNotify: FnOnce(&mut Canvas) -> Result<(), E>,
        ShowNotify: FnOnce(&mut Canvas) -> Result<(), E>,
    {
        if let Some(previous) = previous {
            previous.hide_notify();
            self.has_current = false;
            hide_notify(previous)?;
        }
        next.show_notify();
        self.has_current = true;
        show_notify(next)
    }

    pub fn clear_current(&mut self, current: &mut Canvas) {
        current.hide_notify();
        self.has_current = false;
    }

    /// Remove the current Canvas, then dispatch its protected subclass
    /// `hideNotify` callback.
    ///
    /// The visibility mutation is intentionally retained if the callback
    /// fails: by callback time the Canvas has already ceased to be shown.
    pub fn clear_current_notifying<HideNotify, E>(
        &mut self,
        current: &mut Canvas,
        hide_notify: HideNotify,
    ) -> Result<(), E>
    where
        HideNotify: FnOnce(&mut Canvas) -> Result<(), E>,
    {
        current.hide_notify();
        self.has_current = false;
        hide_notify(current)
    }

    /// `Display.vibrate(duration)`. Unsupported devices return `false` without
    /// emitting a host operation; negative durations throw
    /// `IllegalArgumentException`. Duration zero is retained as the MIDP
    /// request to stop an active vibration.
    pub fn vibrate(&mut self, duration_ms: i32) -> Result<bool, JavaError> {
        if duration_ms < 0 {
            return Err(JavaError::IllegalArgument("negative vibration duration"));
        }
        if self.vibration_supported {
            self.host_ops.push(HostDisplayOp::Vibrate { duration_ms });
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn host_ops(&self) -> &[HostDisplayOp] {
        &self.host_ops
    }

    pub fn drain_host_ops(&mut self) -> Vec<HostDisplayOp> {
        std::mem::take(&mut self.host_ops)
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

    #[test]
    fn notifying_transition_hides_before_showing_and_exposes_updated_state() {
        let mut display = Display::default();
        let mut previous = Canvas::new(176, 220);
        let mut next = Canvas::new(240, 320);
        display.set_current(None, &mut previous);
        let calls = std::cell::RefCell::new(Vec::new());

        display
            .set_current_notifying(
                Some(&mut previous),
                &mut next,
                |canvas| {
                    assert!(!canvas.is_shown());
                    calls.borrow_mut().push("hide");
                    Ok::<(), &'static str>(())
                },
                |canvas| {
                    assert!(canvas.is_shown());
                    calls.borrow_mut().push("show");
                    Ok::<(), &'static str>(())
                },
            )
            .unwrap();

        assert_eq!(calls.into_inner(), vec!["hide", "show"]);
        assert!(!previous.is_shown());
        assert!(next.is_shown());
        assert!(display.has_current());
        assert_eq!(next.poll_event(), Some(CanvasEvent::Paint));
    }

    #[test]
    fn notifying_transition_preserves_hide_and_show_failure_cut_points() {
        let mut display = Display::default();
        let mut previous = Canvas::new(176, 220);
        let mut next = Canvas::new(240, 320);
        display.set_current(None, &mut previous);

        let hide_failure = display.set_current_notifying(
            Some(&mut previous),
            &mut next,
            |canvas| {
                assert!(!canvas.is_shown());
                Err("hide")
            },
            |_| panic!("showNotify must not run after hideNotify fails"),
        );
        assert_eq!(hide_failure, Err("hide"));
        assert!(!previous.is_shown());
        assert!(!next.is_shown());
        assert!(!display.has_current());

        let show_failure = display.set_current_notifying(
            None,
            &mut next,
            |_| panic!("there is no previous Canvas to hide"),
            |canvas| {
                assert!(canvas.is_shown());
                Err("show")
            },
        );
        assert_eq!(show_failure, Err("show"));
        assert!(next.is_shown());
        assert!(display.has_current());

        let hide_failure = display.clear_current_notifying(&mut next, |canvas| {
            assert!(!canvas.is_shown());
            Err("clear-hide")
        });
        assert_eq!(hide_failure, Err("clear-hide"));
        assert!(!next.is_shown());
        assert!(!display.has_current());
    }

    #[test]
    fn vibration_reports_capability_and_emits_only_supported_requests() {
        let mut unsupported = Display::default();
        assert_eq!(unsupported.vibrate(500), Ok(false));
        assert!(unsupported.host_ops().is_empty());

        let mut supported = Display::with_vibration_support(true);
        assert_eq!(supported.vibrate(500), Ok(true));
        assert_eq!(
            supported.drain_host_ops(),
            vec![HostDisplayOp::Vibrate { duration_ms: 500 }]
        );
        assert!(matches!(
            supported.vibrate(-1),
            Err(JavaError::IllegalArgument(_))
        ));
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
    fn get_game_action_resolves_through_the_supplied_device_table() {
        // The ITU keypad doubles as a d-pad under the Nokia table: 2 is up,
        // 8 down, 4 left, 6 right, 5 fire.
        assert_eq!(get_game_action(KEY_NUM2, &nokia_game_action), UP);
        assert_eq!(get_game_action(KEY_NUM8, &nokia_game_action), DOWN);
        assert_eq!(get_game_action(KEY_NUM4, &nokia_game_action), LEFT);
        assert_eq!(get_game_action(KEY_NUM6, &nokia_game_action), RIGHT);
        assert_eq!(get_game_action(KEY_NUM5, &nokia_game_action), FIRE);
        // The negative navigation cluster maps the same under both tables.
        assert_eq!(get_game_action(NAV_UP, &nokia_game_action), UP);
        assert_eq!(get_game_action(NAV_UP, &midp_default_game_action), UP);
        assert_eq!(get_game_action(NAV_FIRE, &midp_default_game_action), FIRE);
    }

    #[test]
    fn the_midp_default_table_maps_only_the_navigation_cluster() {
        // Unlike the Nokia table, the bare MIDP default does not treat the
        // numeric keypad as a d-pad.
        assert_eq!(get_game_action(KEY_NUM2, &midp_default_game_action), 0);
        assert_eq!(get_game_action(KEY_NUM5, &midp_default_game_action), 0);
        // Soft keys, '*' and '#' have no game action under either table.
        for table in [
            &nokia_game_action as &dyn Fn(i32) -> i32,
            &midp_default_game_action,
        ] {
            assert_eq!(get_game_action(SOFT_LEFT, table), 0);
            assert_eq!(get_game_action(SOFT_RIGHT, table), 0);
            assert_eq!(get_game_action(KEY_STAR, table), 0);
            assert_eq!(get_game_action(KEY_POUND, table), 0);
        }
    }

    #[test]
    fn common_game_action_stays_an_alias_of_the_nokia_table() {
        for code in [
            KEY_NUM2, KEY_NUM8, KEY_NUM4, KEY_NUM6, KEY_NUM5, NAV_UP, NAV_FIRE, SOFT_LEFT, 0,
        ] {
            assert_eq!(Canvas::common_game_action(code), nokia_game_action(code));
        }
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
