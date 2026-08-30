//! `javax.microedition.media` (MMAPI) — the `Player`/`Manager` state model a
//! game's sound layer drives, as idiomatic Rust whose *observable* behavior
//! matches the Java ME contract.
//!
//! **R9 — model the channel as STATE, emit host operations; never synthesize
//! audio.** A real MMAPI `Player` owns one audio stream; `getState()` walks the
//! lifecycle `UNREALIZED → REALIZED → PREFETCHED → STARTED` (and `CLOSED`), and
//! the transitions that actually reach the device — `start()`, `stop()`,
//! `setLoopCount`, the `VolumeControl.setLevel` the game applies — are recorded
//! as [`HostAudioOp`]s the host later renders. Decode/output is the host's job.
//!
//! The **device play-stops-active policy stays in the GAME**, not here: each
//! player's lifecycle is independent; the game's audio manager decides when to
//! stop one track before starting another.
//!
//! Object references become handles into an arena the runtime owns (R10):
//! [`MediaRuntime::create_player`] returns a [`PlayerId`]; the ordered op sink
//! has ONE owner (the runtime), so a caller can assert the exact sequence of
//! operations reaching the host (R4).
//!
//! The modeled surface includes realization/prefetch/deallocation, independent
//! player start/stop/looping, listener events, media time/duration, mute/volume,
//! close, and profile-controlled MIME/control availability. Every preferred
//! player source is a classpath resource plus content type; opaque integers are
//! retained only as a compatibility handle for strict existing ports.

use j2me_jvm::JavaError;

/// Host-visible identity of the bytes behind a player. Resource paths are the
/// preferred game-owned form; `Opaque` preserves strict ports whose Java audio
/// manager indexed already-open streams numerically.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MediaSource {
    Resource(String),
    Opaque(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerEvent {
    Started,
    Stopped,
    EndOfMedia,
    Closed,
}

/// The MMAPI `Player` lifecycle states. The integer each maps to (via
/// [`PlayerState::as_mmapi_int`]) is the value `Player.getState()` returns; the
/// game compares it against `400` (`isPlaying` = `getState() >= 400`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState {
    /// `Player.CLOSED` (0) — released; no further calls except `close()`.
    Closed,
    /// `Player.UNREALIZED` (100) — created, not yet realized.
    Unrealized,
    /// `Player.REALIZED` (200) — media located; not prefetched.
    Realized,
    /// `Player.PREFETCHED` (300) — buffered and ready to start.
    Prefetched,
    /// `Player.STARTED` (400) — actively playing.
    Started,
}

impl PlayerState {
    /// The integer `Player.getState()` returns for this state.
    pub fn as_mmapi_int(self) -> i32 {
        match self {
            PlayerState::Closed => 0,
            PlayerState::Unrealized => 100,
            PlayerState::Realized => 200,
            PlayerState::Prefetched => 300,
            PlayerState::Started => 400,
        }
    }
}

/// A handle into the [`MediaRuntime`] player arena — a `javax...media.Player`
/// reference (R10). `Copy`, like a Java reference held in an array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerId(pub usize);

/// One operation the state model asks the host to perform. R9: these are what
/// reaches the audio device; a test asserts this sequence instead of samples.
/// `source` is the resource or compatibility handle the player renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostAudioOp {
    /// `Manager.createPlayer(stream, mime)`.
    Create {
        player: PlayerId,
        source: MediaSource,
        mime: String,
    },
    /// `realize()` — media located (no sound).
    Realize {
        player: PlayerId,
    },
    /// `prefetch()` — buffered ready (no sound).
    Prefetch {
        player: PlayerId,
    },
    Deallocate {
        player: PlayerId,
    },
    /// `setLoopCount(count)`.
    SetLoopCount {
        player: PlayerId,
        count: i32,
    },
    /// `start()` — begin playing the player's source (the audible op).
    Start {
        player: PlayerId,
        source: MediaSource,
    },
    /// `stop()` — halt the player's source (the audible op).
    Stop {
        player: PlayerId,
        source: MediaSource,
    },
    /// `VolumeControl.setLevel(level)` — the level actually applied (0..=100).
    SetVolume {
        player: PlayerId,
        level: i32,
    },
    SetMute {
        player: PlayerId,
        muted: bool,
    },
    SetMediaTime {
        player: PlayerId,
        microseconds: i64,
    },
    /// `close()` — release the player entirely.
    Close {
        player: PlayerId,
    },
}

/// One player in the arena.
#[derive(Debug, Clone)]
struct PlayerCell {
    state: PlayerState,
    /// The sound the player renders (what the host plays).
    source: MediaSource,
    mime: String,
    loop_count: i32,
    /// The `VolumeControl` level, 0..=100 (default full).
    volume_level: i32,
    muted: bool,
    media_time_us: i64,
    duration_us: Option<i64>,
    /// Whether a `PlayerListener` is attached (a game may register itself and
    /// ignore the events; recorded for fidelity).
    has_listener: bool,
}

/// The `javax.microedition.media` runtime: the player arena plus the single
/// ordered host-operation sink (R4). Held by the game shell like any other
/// device-runtime surface.
#[derive(Debug, Default)]
pub struct MediaRuntime {
    players: Vec<PlayerCell>,
    ops: Vec<HostAudioOp>,
    events: Vec<(PlayerId, PlayerEvent)>,
    capabilities: Option<j2me_device::MediaFragment>,
}

fn illegal_state(what: &'static str) -> JavaError {
    JavaError::Media(what.to_string())
}

impl MediaRuntime {
    /// A fresh runtime with no players and an empty op log.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capabilities(capabilities: j2me_device::MediaFragment) -> Self {
        Self {
            capabilities: Some(capabilities),
            ..Self::default()
        }
    }

    // --- host-op sink (R4: one owner) --------------------------------------

    /// The ops emitted so far, in order.
    pub fn ops(&self) -> &[HostAudioOp] {
        &self.ops
    }

    /// Take (and clear) the emitted ops — e.g. between test phases.
    pub fn drain_ops(&mut self) -> Vec<HostAudioOp> {
        std::mem::take(&mut self.ops)
    }

    /// Clear the op log without reading it.
    pub fn clear_ops(&mut self) {
        self.ops.clear();
    }

    pub fn drain_events(&mut self) -> Vec<(PlayerId, PlayerEvent)> {
        std::mem::take(&mut self.events)
    }

    fn cell(&self, p: PlayerId) -> Result<&PlayerCell, JavaError> {
        self.players.get(p.0).ok_or(JavaError::NullPointer)
    }

    fn cell_mut(&mut self, p: PlayerId) -> Result<&mut PlayerCell, JavaError> {
        self.players.get_mut(p.0).ok_or(JavaError::NullPointer)
    }

    // --- Manager -----------------------------------------------------------

    /// `Manager.createPlayer(InputStream, mime)` — a new player for `track` (the
    /// sound's id) in the `UNREALIZED` state, which the game then `realize()`s and
    /// `prefetch()`es. Returns its handle.
    pub fn create_player(&mut self, track: i32, mime: &str) -> PlayerId {
        self.create(MediaSource::Opaque(track), mime)
    }

    /// Profile-checked `Manager.createPlayer` for a classpath resource.
    pub fn create_player_resource(
        &mut self,
        resource: &str,
        mime: &str,
    ) -> Result<PlayerId, JavaError> {
        if resource.is_empty() {
            return Err(JavaError::IllegalArgument("empty media resource"));
        }
        let canonical = self
            .capabilities
            .as_ref()
            .map_or(mime, |capabilities| capabilities.canonical_mime(mime))
            .to_owned();
        if self
            .capabilities
            .as_ref()
            .is_some_and(|capabilities| !capabilities.supports_mime(mime))
        {
            return Err(JavaError::Media(format!(
                "unsupported content type {mime:?} for selected device profile"
            )));
        }
        Ok(self.create(MediaSource::Resource(resource.to_owned()), &canonical))
    }

    fn create(&mut self, source: MediaSource, mime: &str) -> PlayerId {
        let id = PlayerId(self.players.len());
        self.players.push(PlayerCell {
            state: PlayerState::Unrealized,
            source: source.clone(),
            mime: mime.to_owned(),
            loop_count: 1,
            volume_level: 100,
            muted: false,
            media_time_us: 0,
            duration_us: None,
            has_listener: false,
        });
        self.ops.push(HostAudioOp::Create {
            player: id,
            source,
            mime: mime.to_string(),
        });
        id
    }

    /// The sound (`track`) a player renders.
    pub fn track_of(&self, p: PlayerId) -> Result<i32, JavaError> {
        match self.cell(p)?.source {
            MediaSource::Opaque(track) => Ok(track),
            MediaSource::Resource(_) => Err(JavaError::Media(
                "resource player has no numeric track id".to_owned(),
            )),
        }
    }

    pub fn source_of(&self, p: PlayerId) -> Result<&MediaSource, JavaError> {
        Ok(&self.cell(p)?.source)
    }

    pub fn content_type(&self, p: PlayerId) -> Result<&str, JavaError> {
        Ok(&self.cell(p)?.mime)
    }

    // --- Player lifecycle --------------------------------------------------

    /// `getState()`.
    pub fn get_state(&self, p: PlayerId) -> Result<i32, JavaError> {
        Ok(self.cell(p)?.state.as_mmapi_int())
    }

    /// `addPlayerListener(...)` — the game passes itself; we only record that a
    /// listener is attached.
    pub fn add_player_listener(&mut self, p: PlayerId) -> Result<(), JavaError> {
        self.cell_mut(p)?.has_listener = true;
        Ok(())
    }

    pub fn remove_player_listener(&mut self, p: PlayerId) -> Result<(), JavaError> {
        self.cell_mut(p)?.has_listener = false;
        Ok(())
    }

    /// Whether a listener is attached.
    pub fn has_listener(&self, p: PlayerId) -> Result<bool, JavaError> {
        Ok(self.cell(p)?.has_listener)
    }

    /// `realize()` — `UNREALIZED → REALIZED`; a no-op at or past REALIZED;
    /// `IllegalStateException` on a closed player.
    pub fn realize(&mut self, p: PlayerId) -> Result<(), JavaError> {
        let c = self.cell_mut(p)?;
        if c.state == PlayerState::Closed {
            return Err(illegal_state("realize on CLOSED"));
        }
        if c.state == PlayerState::Unrealized {
            c.state = PlayerState::Realized;
        }
        self.ops.push(HostAudioOp::Realize { player: p });
        Ok(())
    }

    /// `prefetch()` — realizes if needed, then `→ PREFETCHED`; a no-op when
    /// already prefetched or started; `IllegalStateException` on a closed player.
    pub fn prefetch(&mut self, p: PlayerId) -> Result<(), JavaError> {
        let c = self.cell_mut(p)?;
        match c.state {
            PlayerState::Closed => return Err(illegal_state("prefetch on CLOSED")),
            PlayerState::Unrealized | PlayerState::Realized => {
                c.state = PlayerState::Prefetched;
            }
            PlayerState::Prefetched | PlayerState::Started => {}
        }
        self.ops.push(HostAudioOp::Prefetch { player: p });
        Ok(())
    }

    /// `setLoopCount(n)` — legal only before `start()`; `IllegalStateException`
    /// when STARTED or CLOSED.
    pub fn set_loop_count(&mut self, p: PlayerId, count: i32) -> Result<(), JavaError> {
        if count == 0 || count < -1 {
            return Err(JavaError::IllegalArgument(
                "setLoopCount accepts -1 or a positive count",
            ));
        }
        let c = self.cell_mut(p)?;
        if matches!(c.state, PlayerState::Started | PlayerState::Closed) {
            return Err(illegal_state("setLoopCount when STARTED/CLOSED"));
        }
        c.loop_count = count;
        self.ops
            .push(HostAudioOp::SetLoopCount { player: p, count });
        Ok(())
    }

    /// `deallocate()` releases prefetched resources and returns the player to
    /// REALIZED. Hosts receive an explicit operation even when it was not
    /// started so their decoded buffer ownership stays visible.
    pub fn deallocate(&mut self, p: PlayerId) -> Result<(), JavaError> {
        let cell = self.cell_mut(p)?;
        if cell.state == PlayerState::Closed {
            return Err(illegal_state("deallocate on CLOSED"));
        }
        cell.state = PlayerState::Realized;
        self.ops.push(HostAudioOp::Deallocate { player: p });
        Ok(())
    }

    /// `start()` — prefetches if needed, then `→ STARTED` and asks the host to
    /// play. A no-op (no op emitted) if already STARTED. `IllegalStateException`
    /// on a closed player.
    pub fn start(&mut self, p: PlayerId) -> Result<(), JavaError> {
        let (source, was_started, has_listener) = {
            let c = self.cell_mut(p)?;
            if c.state == PlayerState::Closed {
                return Err(illegal_state("start on CLOSED"));
            }
            let was = c.state == PlayerState::Started;
            if !was {
                c.state = PlayerState::Started;
            }
            (c.source.clone(), was, c.has_listener)
        };
        if !was_started {
            self.ops.push(HostAudioOp::Start { player: p, source });
            if has_listener {
                self.events.push((p, PlayerEvent::Started));
            }
        }
        Ok(())
    }

    /// `stop()` — `STARTED → PREFETCHED` and asks the host to halt. A no-op (no op
    /// emitted) if not started. `IllegalStateException` on a closed player.
    pub fn stop(&mut self, p: PlayerId) -> Result<(), JavaError> {
        let (source, was_started, has_listener) = {
            let c = self.cell_mut(p)?;
            if c.state == PlayerState::Closed {
                return Err(illegal_state("stop on CLOSED"));
            }
            let was = c.state == PlayerState::Started;
            if was {
                c.state = PlayerState::Prefetched;
            }
            (c.source.clone(), was, c.has_listener)
        };
        if was_started {
            self.ops.push(HostAudioOp::Stop { player: p, source });
            if has_listener {
                self.events.push((p, PlayerEvent::Stopped));
            }
        }
        Ok(())
    }

    /// `close()` — `→ CLOSED` (idempotent).
    pub fn close(&mut self, p: PlayerId) -> Result<(), JavaError> {
        let listener = self.cell_mut(p)?;
        listener.state = PlayerState::Closed;
        let has_listener = listener.has_listener;
        self.ops.push(HostAudioOp::Close { player: p });
        if has_listener {
            self.events.push((p, PlayerEvent::Closed));
        }
        Ok(())
    }

    // --- VolumeControl -----------------------------------------------------

    /// `getControl(controlType)` — presence of the named `Control`. The game asks
    /// for `"VolumeControl"` and null-checks the result; every player here exposes
    /// a `VolumeControl` (returns `true`), and any other control type is absent
    /// (`false`, i.e. Java `null`). A missing player is `NullPointerException`.
    pub fn get_control(&self, p: PlayerId, control_type: &str) -> Result<bool, JavaError> {
        self.cell(p)?; // validate the handle
        let profile_has_control = match &self.capabilities {
            Some(capabilities) => capabilities.has_control("VolumeControl"),
            None => true,
        };
        Ok(control_type == "VolumeControl" && profile_has_control)
    }

    /// `VolumeControl.setLevel(level)` — set the absolute volume (clamped to
    /// 0..=100 as MMAPI does) and return the level actually applied; emits a host
    /// op. The game gates this behind a non-null `getControl("VolumeControl")`.
    pub fn set_level(&mut self, p: PlayerId, level: i32) -> Result<i32, JavaError> {
        let applied = level.clamp(0, 100);
        self.cell_mut(p)?.volume_level = applied;
        self.ops.push(HostAudioOp::SetVolume {
            player: p,
            level: applied,
        });
        Ok(applied)
    }

    /// `VolumeControl.getLevel()`.
    pub fn get_level(&self, p: PlayerId) -> Result<i32, JavaError> {
        Ok(self.cell(p)?.volume_level)
    }

    pub fn set_mute(&mut self, p: PlayerId, muted: bool) -> Result<(), JavaError> {
        self.cell_mut(p)?.muted = muted;
        self.ops.push(HostAudioOp::SetMute { player: p, muted });
        Ok(())
    }

    pub fn is_muted(&self, p: PlayerId) -> Result<bool, JavaError> {
        Ok(self.cell(p)?.muted)
    }

    pub fn set_media_time(&mut self, p: PlayerId, microseconds: i64) -> Result<i64, JavaError> {
        if microseconds < 0 {
            return Err(JavaError::Media(
                "setMediaTime requires a non-negative value".to_owned(),
            ));
        }
        let cell = self.cell_mut(p)?;
        if cell.state == PlayerState::Closed {
            return Err(illegal_state("setMediaTime on CLOSED"));
        }
        cell.media_time_us = microseconds;
        self.ops.push(HostAudioOp::SetMediaTime {
            player: p,
            microseconds,
        });
        Ok(microseconds)
    }

    pub fn media_time(&self, p: PlayerId) -> Result<i64, JavaError> {
        Ok(self.cell(p)?.media_time_us)
    }

    /// Host-supplied decoded duration. `getDuration()` returns MMAPI
    /// `TIME_UNKNOWN` (-1) until the adapter has prepared the media.
    pub fn set_duration(&mut self, p: PlayerId, microseconds: i64) -> Result<(), JavaError> {
        if microseconds < 0 {
            return Err(JavaError::IllegalArgument("negative media duration"));
        }
        self.cell_mut(p)?.duration_us = Some(microseconds);
        Ok(())
    }

    pub fn duration(&self, p: PlayerId) -> Result<i64, JavaError> {
        Ok(self.cell(p)?.duration_us.unwrap_or(-1))
    }

    /// Host notification after finite playback reaches the end.
    pub fn notify_end_of_media(&mut self, p: PlayerId) -> Result<(), JavaError> {
        let cell = self.cell_mut(p)?;
        if cell.state != PlayerState::Started {
            return Err(illegal_state("endOfMedia when not STARTED"));
        }
        cell.state = PlayerState::Prefetched;
        if cell.has_listener {
            self.events.push((p, PlayerEvent::EndOfMedia));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmapi_state_integers_match_the_contract() {
        assert_eq!(PlayerState::Closed.as_mmapi_int(), 0);
        assert_eq!(PlayerState::Unrealized.as_mmapi_int(), 100);
        assert_eq!(PlayerState::Realized.as_mmapi_int(), 200);
        assert_eq!(PlayerState::Prefetched.as_mmapi_int(), 300);
        assert_eq!(PlayerState::Started.as_mmapi_int(), 400);
    }

    #[test]
    fn lifecycle_walks_the_states_and_emits_ops() {
        // A typical start() path: realize -> prefetch -> start, then stop.
        let mut m = MediaRuntime::new();
        let p = m.create_player(7, "audio/midi");
        assert_eq!(m.get_state(p).unwrap(), 100);
        m.realize(p).unwrap();
        assert_eq!(m.get_state(p).unwrap(), 200);
        m.prefetch(p).unwrap();
        assert_eq!(m.get_state(p).unwrap(), 300);
        m.set_loop_count(p, 1).unwrap();
        m.start(p).unwrap();
        assert_eq!(m.get_state(p).unwrap(), 400);
        assert!(m.get_state(p).unwrap() >= 400, "isPlaying is state >= 400");
        m.stop(p).unwrap();
        assert_eq!(m.get_state(p).unwrap(), 300);

        let ops = m.ops();
        assert!(ops.contains(&HostAudioOp::Start {
            player: p,
            source: MediaSource::Opaque(7)
        }));
        assert!(ops.contains(&HostAudioOp::Stop {
            player: p,
            source: MediaSource::Opaque(7)
        }));
    }

    #[test]
    fn start_on_started_is_a_silent_noop() {
        let mut m = MediaRuntime::new();
        let p = m.create_player(1, "audio/midi");
        m.prefetch(p).unwrap();
        m.start(p).unwrap();
        m.clear_ops();
        m.start(p).unwrap(); // already STARTED
        assert!(m.drain_ops().is_empty());
    }

    #[test]
    fn set_loop_count_rejected_while_started() {
        let mut m = MediaRuntime::new();
        let p = m.create_player(1, "audio/midi");
        m.prefetch(p).unwrap();
        m.start(p).unwrap();
        assert!(m.set_loop_count(p, 1).is_err());
    }

    #[test]
    fn closed_player_rejects_transitions() {
        let mut m = MediaRuntime::new();
        let p = m.create_player(1, "audio/midi");
        m.close(p).unwrap();
        assert_eq!(m.get_state(p).unwrap(), 0);
        assert!(m.realize(p).is_err());
        assert!(m.prefetch(p).is_err());
        assert!(m.start(p).is_err());
    }

    #[test]
    fn volume_control_is_present_and_sets_a_clamped_level() {
        // The setVolume path: getControl("VolumeControl") then setLevel(level).
        let mut m = MediaRuntime::new();
        let p = m.create_player(2, "audio/x-wav");
        assert!(m.get_control(p, "VolumeControl").unwrap());
        assert!(!m.get_control(p, "ToneControl").unwrap()); // absent (null)
        assert_eq!(m.set_level(p, 70).unwrap(), 70);
        assert_eq!(m.get_level(p).unwrap(), 70);
        // Clamp out-of-range levels rather than storing them raw (R3 control).
        assert_eq!(m.set_level(p, 150).unwrap(), 100);
        assert_eq!(m.set_level(p, -5).unwrap(), 0);
        assert!(m.ops().contains(&HostAudioOp::SetVolume {
            player: p,
            level: 100
        }));
    }

    #[test]
    fn add_player_listener_is_recorded() {
        let mut m = MediaRuntime::new();
        let p = m.create_player(3, "audio/midi");
        assert!(!m.has_listener(p).unwrap());
        m.add_player_listener(p).unwrap();
        assert!(m.has_listener(p).unwrap());
    }

    #[test]
    fn a_bad_handle_is_a_null_pointer_not_a_panic() {
        let m = MediaRuntime::new();
        assert!(matches!(
            m.get_state(PlayerId(9)),
            Err(JavaError::NullPointer)
        ));
    }

    #[test]
    fn capabilities_control_mime_aliases_and_controls() {
        use std::collections::{BTreeMap, BTreeSet};
        let capabilities = j2me_device::MediaFragment {
            content_types: BTreeSet::from(["audio/midi".to_owned()]),
            mime_aliases: BTreeMap::from([("audio/x-midi".to_owned(), "audio/midi".to_owned())]),
            controls: BTreeSet::new(),
            midi_renderer: "approximate".to_owned(),
        };
        let mut runtime = MediaRuntime::with_capabilities(capabilities);
        let player = runtime
            .create_player_resource("/audio/theme.mid", "audio/x-midi")
            .unwrap();
        assert_eq!(runtime.content_type(player).unwrap(), "audio/midi");
        assert!(!runtime.get_control(player, "VolumeControl").unwrap());
        assert!(runtime
            .create_player_resource("/audio/voice.amr", "audio/amr")
            .is_err());
    }

    #[test]
    fn mute_seek_and_completion_are_stateful_and_host_visible() {
        let mut runtime = MediaRuntime::new();
        let player = runtime
            .create_player_resource("/tone.wav", "audio/x-wav")
            .unwrap();
        runtime.add_player_listener(player).unwrap();
        runtime.set_mute(player, true).unwrap();
        runtime.set_media_time(player, 42_000).unwrap();
        assert_eq!(runtime.duration(player).unwrap(), -1);
        runtime.set_duration(player, 125_000).unwrap();
        runtime.start(player).unwrap();
        runtime.notify_end_of_media(player).unwrap();
        assert!(runtime.is_muted(player).unwrap());
        assert_eq!(runtime.media_time(player).unwrap(), 42_000);
        assert_eq!(runtime.duration(player).unwrap(), 125_000);
        assert_eq!(runtime.get_state(player).unwrap(), 300);
        assert!(runtime
            .drain_events()
            .contains(&(player, PlayerEvent::EndOfMedia)));
    }
}
