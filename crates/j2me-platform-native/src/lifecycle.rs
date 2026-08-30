pub fn focus_from_window_event(
    lifecycle: &mut j2me_platform::FocusLifecycle,
    event: &winit::event::WindowEvent,
) -> Option<Vec<j2me_platform::LifecycleAction>> {
    match event {
        winit::event::WindowEvent::Focused(focused) => Some(lifecycle.changed(*focused)),
        _ => None,
    }
}
