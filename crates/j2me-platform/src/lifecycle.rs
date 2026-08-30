//! Shared conversion from host focus events to explicit MIDP/display actions.

use j2me_device::{FocusGainPolicy, FocusLossPolicy, LifecycleFragment};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleAction {
    Start,
    Pause,
    Show,
    Hide,
}

pub fn focus_actions(policy: &LifecycleFragment, focused: bool) -> Vec<LifecycleAction> {
    if focused {
        match policy.focus_gain {
            FocusGainPolicy::None => vec![],
            FocusGainPolicy::Show => vec![LifecycleAction::Show],
            FocusGainPolicy::StartThenShow => vec![LifecycleAction::Start, LifecycleAction::Show],
            FocusGainPolicy::ShowThenStart => vec![LifecycleAction::Show, LifecycleAction::Start],
        }
    } else {
        match policy.focus_loss {
            FocusLossPolicy::None => vec![],
            FocusLossPolicy::Hide => vec![LifecycleAction::Hide],
            FocusLossPolicy::PauseThenHide => vec![LifecycleAction::Pause, LifecycleAction::Hide],
            FocusLossPolicy::HideThenPause => vec![LifecycleAction::Hide, LifecycleAction::Pause],
        }
    }
}

#[derive(Debug, Clone)]
pub struct FocusLifecycle {
    focused: Option<bool>,
    policy: LifecycleFragment,
}

impl FocusLifecycle {
    pub fn new(policy: LifecycleFragment) -> Self {
        Self {
            focused: None,
            policy,
        }
    }

    /// Suppresses duplicate host focus notifications while preserving the
    /// device-profile ordering of callbacks and display notifications.
    pub fn changed(&mut self, focused: bool) -> Vec<LifecycleAction> {
        if self.focused == Some(focused) {
            return vec![];
        }
        self.focused = Some(focused);
        focus_actions(&self.policy, focused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_order_is_profile_data_and_duplicates_are_suppressed() {
        let policy = LifecycleFragment {
            focus_loss: FocusLossPolicy::PauseThenHide,
            focus_gain: FocusGainPolicy::ShowThenStart,
            platform_request: false,
            platform_request_requires_exit: false,
        };
        let mut lifecycle = FocusLifecycle::new(policy);
        assert_eq!(
            lifecycle.changed(false),
            vec![LifecycleAction::Pause, LifecycleAction::Hide]
        );
        assert!(lifecycle.changed(false).is_empty());
        assert_eq!(
            lifecycle.changed(true),
            vec![LifecycleAction::Show, LifecycleAction::Start]
        );
    }
}
