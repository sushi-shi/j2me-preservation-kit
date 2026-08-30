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

/// A device's `Canvas.getGameAction` key-to-action table: it answers the game
/// action for a raw key code, or `0` for a key with no game action, exactly as
/// MIDP defines the mapping to be device-supplied. This portable crate ships no
/// implicit handset table; a host supplies a closure or reviewed profile.
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

/// `Canvas.getGameAction` through a reviewed game-owned handset fragment.
pub fn get_game_action_profile(code: i32, input: &j2me_device::InputFragment) -> i32 {
    input.game_action(code)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CanvasEvent {
    Paint,
    KeyPressed(i32),
    KeyReleased(i32),
    KeyRepeated(i32),
    PointerPressed { x: i32, y: i32 },
    PointerDragged { x: i32, y: i32 },
    PointerReleased { x: i32, y: i32 },
    SizeChanged { width: i32, height: i32 },
    CommandAction(CommandId),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct CommandId(pub u32);

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Command {
    pub id: CommandId,
    pub label: String,
    pub command_type: i32,
    pub priority: i32,
}

impl Command {
    pub const SCREEN: i32 = 1;
    pub const BACK: i32 = 2;
    pub const CANCEL: i32 = 3;
    pub const OK: i32 = 4;
    pub const HELP: i32 = 5;
    pub const STOP: i32 = 6;
    pub const EXIT: i32 = 7;
    pub const ITEM: i32 = 8;

    pub fn new(id: CommandId, label: impl Into<String>, command_type: i32, priority: i32) -> Self {
        Self {
            id,
            label: label.into(),
            command_type,
            priority,
        }
    }
}

#[derive(Debug)]
pub struct Canvas {
    width: i32,
    height: i32,
    shown: bool,
    full_screen: bool,
    repaint_owed: bool,
    input: VecDeque<CanvasEvent>,
    commands: Vec<Command>,
    command_listener: bool,
    pointer_events: bool,
    pointer_motion_events: bool,
    repeat_events: bool,
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
            commands: Vec::new(),
            command_listener: false,
            pointer_events: false,
            pointer_motion_events: false,
            repeat_events: false,
        }
    }

    pub fn for_device(display: &j2me_device::DisplayFragment, fullscreen: bool) -> Self {
        let (width, height) = display.canvas_size(fullscreen);
        let mut canvas = Self::new(
            i32::try_from(width).expect("device-profile width fits Java int"),
            i32::try_from(height).expect("device-profile height fits Java int"),
        );
        canvas.pointer_events = false;
        canvas.pointer_motion_events = false;
        canvas.repeat_events = false;
        canvas
    }

    /// Construct a canvas with the selected device's display and callback
    /// capabilities. Keeping these flags in the game-owned profile prevents a
    /// desktop host from silently pretending every handset was touch-capable.
    pub fn for_profile(profile: &j2me_device::DeviceProfile, fullscreen: bool) -> Self {
        let mut canvas = Self::for_device(&profile.display, fullscreen);
        canvas.pointer_events = profile.input.pointer_events;
        canvas.pointer_motion_events = profile.input.pointer_motion_events;
        canvas.repeat_events = profile.input.repeat_events;
        canvas
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

    pub const fn has_pointer_events(&self) -> bool {
        self.pointer_events
    }

    pub const fn has_pointer_motion_events(&self) -> bool {
        self.pointer_motion_events
    }

    pub const fn has_repeat_events(&self) -> bool {
        self.repeat_events
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

    pub fn pointer_pressed(&mut self, x: i32, y: i32) {
        self.input.push_back(CanvasEvent::PointerPressed { x, y });
    }

    pub fn pointer_dragged(&mut self, x: i32, y: i32) {
        self.input.push_back(CanvasEvent::PointerDragged { x, y });
    }

    pub fn pointer_released(&mut self, x: i32, y: i32) {
        self.input.push_back(CanvasEvent::PointerReleased { x, y });
    }

    /// Apply a host surface resize before queueing the MIDP `sizeChanged`
    /// callback. Duplicate dimensions do not invent duplicate callbacks.
    pub fn resize(&mut self, width: i32, height: i32) -> Result<(), JavaError> {
        if width <= 0 || height <= 0 {
            return Err(JavaError::IllegalArgument(
                "Canvas dimensions must be positive",
            ));
        }
        if self.width != width || self.height != height {
            self.width = width;
            self.height = height;
            self.input
                .push_back(CanvasEvent::SizeChanged { width, height });
            self.request_repaint();
        }
        Ok(())
    }

    pub fn add_command(&mut self, command: Command) {
        if !self
            .commands
            .iter()
            .any(|existing| existing.id == command.id)
        {
            self.commands.push(command);
        }
    }

    pub fn remove_command(&mut self, id: CommandId) {
        self.commands.retain(|command| command.id != id);
    }

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    pub fn set_command_listener(&mut self, present: bool) {
        self.command_listener = present;
    }

    /// Queue a soft-command callback only when the same command object is
    /// registered and a listener is installed, matching `Displayable` identity
    /// rather than guessing from its label or priority.
    pub fn command_action(&mut self, id: CommandId) -> bool {
        if self.command_listener && self.commands.iter().any(|command| command.id == id) {
            self.input.push_back(CanvasEvent::CommandAction(id));
            true
        } else {
            false
        }
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

    pub fn for_device(haptics: &j2me_device::HapticsFragment) -> Self {
        Self::with_vibration_support(haptics.vibration)
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
    fn game_action_has_no_implicit_phone_table() {
        let custom = |code| if code == 710 { GAME_A } else { 0 };
        assert_eq!(get_game_action(710, &custom), GAME_A);
        assert_eq!(get_game_action(KEY_NUM2, &custom), 0);

        let input = j2me_device::InputFragment {
            up: 700,
            down: 701,
            left: 702,
            right: 703,
            fire: 704,
            soft_left: 705,
            soft_right: 706,
            star: 707,
            pound: 708,
            digits: [720, 721, 722, 723, 724, 725, 726, 727, 728, 729],
            game_action_up: vec![700, 722],
            game_action_down: vec![701, 728],
            game_action_left: vec![702, 724],
            game_action_right: vec![703, 726],
            game_action_fire: vec![704, 725],
            pointer_events: true,
            pointer_motion_events: true,
            repeat_events: true,
        };
        assert_eq!(get_game_action_profile(722, &input), UP);
        assert_eq!(get_game_action_profile(705, &input), 0);
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

    #[test]
    fn pointer_resize_and_command_callbacks_share_the_serial_queue() {
        let mut c = Canvas::new(128, 160);
        c.pointer_pressed(7, 11);
        c.resize(176, 208).unwrap();
        let left = Command::new(CommandId(1), " ", Command::OK, 1);
        c.add_command(left);
        c.set_command_listener(true);
        assert!(c.command_action(CommandId(1)));

        // resize owes a repaint, so the serialized MIDP paint remains first.
        assert_eq!(c.poll_event(), Some(CanvasEvent::Paint));
        assert_eq!(
            c.poll_event(),
            Some(CanvasEvent::PointerPressed { x: 7, y: 11 })
        );
        assert_eq!(
            c.poll_event(),
            Some(CanvasEvent::SizeChanged {
                width: 176,
                height: 208
            })
        );
        assert_eq!(
            c.poll_event(),
            Some(CanvasEvent::CommandAction(CommandId(1)))
        );
    }
}
