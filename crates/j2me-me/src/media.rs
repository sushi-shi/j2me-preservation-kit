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
//! The modeled `Player` surface: `realize`, `prefetch`, `start`, `stop`,
//! `close`, `getState`, `setLoopCount`, `addPlayerListener`, and
//! `getControl("VolumeControl").setLevel(level)`. Methods a strict port does not
//! call — `deallocate` / `setMediaTime` / `removePlayerListener` / `getDuration`
//! — are left unmodeled until a game needs them (no APIs the game never calls),
//! and every player is created from a resource stream with a content type
//! (`Manager.createPlayer(InputStream, mime)`), the shipped-asset path.

use j2me_jvm::JavaError;

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
/// `track` is the sound the player was created for (what the host plays).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostAudioOp {
    /// `Manager.createPlayer(stream, mime)` — a player for `track` with `mime`.
    Create {
        player: PlayerId,
        track: i32,
        mime: String,
    },
    /// `realize()` — media located (no sound).
    Realize { player: PlayerId },
    /// `prefetch()` — buffered ready (no sound).
    Prefetch { player: PlayerId },
    /// `setLoopCount(count)`.
    SetLoopCount { player: PlayerId, count: i32 },
    /// `start()` — begin playing `track` (the audible op).
    Start { player: PlayerId, track: i32 },
    /// `stop()` — halt `track` (the audible op).
    Stop { player: PlayerId, track: i32 },
    /// `VolumeControl.setLevel(level)` — the level actually applied (0..=100).
    SetVolume { player: PlayerId, level: i32 },
    /// `close()` — release the player entirely.
    Close { player: PlayerId },
}

/// One player in the arena.
#[derive(Debug, Clone)]
struct PlayerCell {
    state: PlayerState,
    /// The sound the player renders (what the host plays).
    track: i32,
    loop_count: i32,
    /// The `VolumeControl` level, 0..=100 (default full).
    volume_level: i32,
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
}

fn illegal_state(what: &'static str) -> JavaError {
    JavaError::Media(what.to_string())
}

impl MediaRuntime {
    /// A fresh runtime with no players and an empty op log.
    pub fn new() -> Self {
        Self::default()
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
        let id = PlayerId(self.players.len());
        self.players.push(PlayerCell {
            state: PlayerState::Unrealized,
            track,
            loop_count: 1,
            volume_level: 100,
            has_listener: false,
        });
        self.ops.push(HostAudioOp::Create {
            player: id,
            track,
            mime: mime.to_string(),
        });
        id
    }

    /// The sound (`track`) a player renders.
    pub fn track_of(&self, p: PlayerId) -> Result<i32, JavaError> {
        Ok(self.cell(p)?.track)
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
        let c = self.cell_mut(p)?;
        if matches!(c.state, PlayerState::Started | PlayerState::Closed) {
            return Err(illegal_state("setLoopCount when STARTED/CLOSED"));
        }
        c.loop_count = count;
        self.ops
            .push(HostAudioOp::SetLoopCount { player: p, count });
        Ok(())
    }

    /// `start()` — prefetches if needed, then `→ STARTED` and asks the host to
    /// play. A no-op (no op emitted) if already STARTED. `IllegalStateException`
    /// on a closed player.
    pub fn start(&mut self, p: PlayerId) -> Result<(), JavaError> {
        let (track, was_started) = {
            let c = self.cell_mut(p)?;
            if c.state == PlayerState::Closed {
                return Err(illegal_state("start on CLOSED"));
            }
            let was = c.state == PlayerState::Started;
            if !was {
                c.state = PlayerState::Started;
            }
            (c.track, was)
        };
        if !was_started {
            self.ops.push(HostAudioOp::Start { player: p, track });
        }
        Ok(())
    }

    /// `stop()` — `STARTED → PREFETCHED` and asks the host to halt. A no-op (no op
    /// emitted) if not started. `IllegalStateException` on a closed player.
    pub fn stop(&mut self, p: PlayerId) -> Result<(), JavaError> {
        let (track, was_started) = {
            let c = self.cell_mut(p)?;
            if c.state == PlayerState::Closed {
                return Err(illegal_state("stop on CLOSED"));
            }
            let was = c.state == PlayerState::Started;
            if was {
                c.state = PlayerState::Prefetched;
            }
            (c.track, was)
        };
        if was_started {
            self.ops.push(HostAudioOp::Stop { player: p, track });
        }
        Ok(())
    }

    /// `close()` — `→ CLOSED` (idempotent).
    pub fn close(&mut self, p: PlayerId) -> Result<(), JavaError> {
        self.cell_mut(p)?.state = PlayerState::Closed;
        self.ops.push(HostAudioOp::Close { player: p });
        Ok(())
    }

    // --- VolumeControl -----------------------------------------------------

    /// `getControl(controlType)` — presence of the named `Control`. The game asks
    /// for `"VolumeControl"` and null-checks the result; every player here exposes
    /// a `VolumeControl` (returns `true`), and any other control type is absent
    /// (`false`, i.e. Java `null`). A missing player is `NullPointerException`.
    pub fn get_control(&self, p: PlayerId, control_type: &str) -> Result<bool, JavaError> {
        self.cell(p)?; // validate the handle
        Ok(control_type == "VolumeControl")
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
            track: 7
        }));
        assert!(ops.contains(&HostAudioOp::Stop {
            player: p,
            track: 7
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
}
