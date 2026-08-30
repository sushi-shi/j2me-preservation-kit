//! `javax.microedition.midlet.MIDlet` lifecycle state.
//!
//! This models the framework boundary only. Whether a desktop window losing
//! focus should ask the application manager to pause is host policy and is not
//! encoded here.

use j2me_jvm::JavaError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidletState {
    Paused,
    Active,
    Destroyed,
}

/// Callback the application manager must deliver to the game. The caller
/// commits it only after the callback succeeds, preserving the fact that
/// `startApp` and conditional `destroyApp` may throw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidletCallback {
    StartApp,
    PauseApp,
    DestroyApp { unconditional: bool },
}

/// Operation requested by the MIDlet itself through a final framework method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostMidletOp {
    NotifyPaused,
    ResumeRequest,
    NotifyDestroyed,
    PlatformRequest { url: String },
}

#[derive(Debug, Clone)]
pub struct MidletLifecycle {
    state: MidletState,
    host_ops: Vec<HostMidletOp>,
    platform_request_result: Option<bool>,
}

impl Default for MidletLifecycle {
    fn default() -> Self {
        Self {
            // A constructed MIDlet begins paused until the application manager
            // invokes startApp for the first time.
            state: MidletState::Paused,
            host_ops: Vec::new(),
            platform_request_result: None,
        }
    }
}

impl MidletLifecycle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure support for `platformRequest`; the boolean is the MIDP return
    /// value (whether the MIDlet must exit before the request can proceed).
    pub fn with_platform_request_result(result: Option<bool>) -> Self {
        Self {
            platform_request_result: result,
            ..Self::default()
        }
    }

    pub const fn state(&self) -> MidletState {
        self.state
    }

    pub fn request_start(&self) -> Result<MidletCallback, JavaError> {
        match self.state {
            MidletState::Paused => Ok(MidletCallback::StartApp),
            MidletState::Active => Err(JavaError::IllegalState("MIDlet is already active")),
            MidletState::Destroyed => Err(JavaError::IllegalState("MIDlet is destroyed")),
        }
    }

    pub fn request_pause(&self) -> Result<MidletCallback, JavaError> {
        match self.state {
            MidletState::Active => Ok(MidletCallback::PauseApp),
            MidletState::Paused => Err(JavaError::IllegalState("MIDlet is already paused")),
            MidletState::Destroyed => Err(JavaError::IllegalState("MIDlet is destroyed")),
        }
    }

    pub fn request_destroy(&self, unconditional: bool) -> Result<MidletCallback, JavaError> {
        if self.state == MidletState::Destroyed {
            Err(JavaError::IllegalState("MIDlet is destroyed"))
        } else {
            Ok(MidletCallback::DestroyApp { unconditional })
        }
    }

    /// Commit a callback only after the game implementation returned normally.
    pub fn commit(&mut self, callback: MidletCallback) -> Result<(), JavaError> {
        match callback {
            MidletCallback::StartApp if self.state == MidletState::Paused => {
                self.state = MidletState::Active;
            }
            MidletCallback::PauseApp if self.state == MidletState::Active => {
                self.state = MidletState::Paused;
            }
            MidletCallback::DestroyApp { .. } if self.state != MidletState::Destroyed => {
                self.state = MidletState::Destroyed;
            }
            _ => return Err(JavaError::IllegalState("stale MIDlet callback")),
        }
        Ok(())
    }

    /// `MIDlet.notifyPaused()`: no `pauseApp` callback is generated.
    pub fn notify_paused(&mut self) -> Result<(), JavaError> {
        if self.state == MidletState::Destroyed {
            return Err(JavaError::IllegalState("MIDlet is destroyed"));
        }
        self.state = MidletState::Paused;
        self.host_ops.push(HostMidletOp::NotifyPaused);
        Ok(())
    }

    /// `MIDlet.resumeRequest()`: the application manager may later call
    /// `startApp`; the request itself does not activate the MIDlet.
    pub fn resume_request(&mut self) -> Result<(), JavaError> {
        if self.state == MidletState::Destroyed {
            return Err(JavaError::IllegalState("MIDlet is destroyed"));
        }
        self.host_ops.push(HostMidletOp::ResumeRequest);
        Ok(())
    }

    /// `MIDlet.notifyDestroyed()`: enter the terminal state without a callback.
    pub fn notify_destroyed(&mut self) {
        self.state = MidletState::Destroyed;
        self.host_ops.push(HostMidletOp::NotifyDestroyed);
    }

    /// `MIDlet.platformRequest(url)`. Actual URL handling remains a host
    /// operation; unsupported profiles throw `ConnectionNotFoundException`.
    pub fn platform_request(&mut self, url: &str) -> Result<bool, JavaError> {
        if self.state == MidletState::Destroyed {
            return Err(JavaError::IllegalState("MIDlet is destroyed"));
        }
        if url.is_empty() {
            return Err(JavaError::ConnectionNotFound("empty URL".to_owned()));
        }
        let result = self
            .platform_request_result
            .ok_or_else(|| JavaError::ConnectionNotFound(format!("unsupported URL: {url}")))?;
        self.host_ops.push(HostMidletOp::PlatformRequest {
            url: url.to_owned(),
        });
        Ok(result)
    }

    pub fn drain_host_ops(&mut self) -> Vec<HostMidletOp> {
        std::mem::take(&mut self.host_ops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callbacks_are_committed_only_after_success() {
        let mut lifecycle = MidletLifecycle::new();
        let start = lifecycle.request_start().unwrap();
        assert_eq!(lifecycle.state(), MidletState::Paused);
        lifecycle.commit(start).unwrap();
        assert_eq!(lifecycle.state(), MidletState::Active);

        let pause = lifecycle.request_pause().unwrap();
        lifecycle.commit(pause).unwrap();
        assert_eq!(lifecycle.state(), MidletState::Paused);
    }

    #[test]
    fn midlet_requests_do_not_invent_window_focus_policy() {
        let mut lifecycle = MidletLifecycle::new();
        lifecycle.resume_request().unwrap();
        assert_eq!(lifecycle.state(), MidletState::Paused);
        assert_eq!(
            lifecycle.drain_host_ops(),
            vec![HostMidletOp::ResumeRequest]
        );

        lifecycle.notify_destroyed();
        assert_eq!(lifecycle.state(), MidletState::Destroyed);
        assert!(lifecycle.request_start().is_err());
    }

    #[test]
    fn platform_request_is_profile_controlled_and_host_visible() {
        let mut supported = MidletLifecycle::with_platform_request_result(Some(true));
        assert!(supported
            .platform_request("https://example.invalid")
            .unwrap());
        assert_eq!(
            supported.drain_host_ops(),
            vec![HostMidletOp::PlatformRequest {
                url: "https://example.invalid".to_owned()
            }]
        );
        assert!(MidletLifecycle::new()
            .platform_request("https://example.invalid")
            .is_err());
    }
}
