#[cfg(target_arch = "wasm32")]
mod implementation {
    use std::collections::HashMap;

    use j2me_me::{HostAudioOp, MediaSource, PlayerId};
    use wasm_bindgen::{JsCast, JsValue};
    use web_sys::{AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState, GainNode};

    struct Player {
        source: MediaSource,
        loops: i32,
        level: i32,
        muted: bool,
        offset_seconds: f64,
        active: Option<AudioBufferSourceNode>,
        gain: Option<GainNode>,
    }

    pub struct BrowserAudio {
        context: AudioContext,
        buffers: HashMap<MediaSource, AudioBuffer>,
        players: HashMap<PlayerId, Player>,
    }

    impl BrowserAudio {
        pub fn new() -> Result<Self, JsValue> {
            Ok(Self {
                context: AudioContext::new()?,
                buffers: HashMap::new(),
                players: HashMap::new(),
            })
        }

        pub fn prepare(
            &mut self,
            source: MediaSource,
            bytes: &[u8],
            mime: &str,
        ) -> Result<Option<&'static str>, JsValue> {
            let decoded =
                j2me_media::decode_audio(bytes, mime).map_err(|error| JsValue::from_str(&error))?;
            let buffer = self.context.create_buffer(
                1,
                decoded.samples.len() as u32,
                decoded.sample_rate as f32,
            )?;
            buffer.copy_to_channel(&decoded.samples, 0)?;
            self.buffers.insert(source, buffer);
            Ok(decoded.approximation)
        }

        pub fn resume_from_gesture(&self) {
            if self.context.state() == AudioContextState::Suspended {
                let _ = self.context.resume();
            }
        }

        pub fn apply(&mut self, operation: &HostAudioOp) -> Result<(), JsValue> {
            match operation {
                HostAudioOp::Create { player, source, .. } => {
                    self.players.insert(
                        *player,
                        Player {
                            source: source.clone(),
                            loops: 1,
                            level: 100,
                            muted: false,
                            offset_seconds: 0.0,
                            active: None,
                            gain: None,
                        },
                    );
                }
                HostAudioOp::SetLoopCount { player, count } => {
                    self.player_mut(*player)?.loops = *count
                }
                HostAudioOp::SetVolume { player, level } => {
                    let player = self.player_mut(*player)?;
                    player.level = *level;
                    update_gain(player);
                }
                HostAudioOp::SetMute { player, muted } => {
                    let player = self.player_mut(*player)?;
                    player.muted = *muted;
                    update_gain(player);
                }
                HostAudioOp::SetMediaTime {
                    player,
                    microseconds,
                } => {
                    self.player_mut(*player)?.offset_seconds = *microseconds as f64 / 1_000_000.0;
                }
                HostAudioOp::Start { player, source } => self.start(*player, source)?,
                HostAudioOp::Stop { player, .. } => self.stop(*player)?,
                HostAudioOp::Close { player } => {
                    self.stop(*player)?;
                    self.players.remove(player);
                }
                HostAudioOp::Deallocate { player } => self.stop(*player)?,
                HostAudioOp::Realize { .. } | HostAudioOp::Prefetch { .. } => {}
            }
            Ok(())
        }

        fn start(&mut self, id: PlayerId, source_id: &MediaSource) -> Result<(), JsValue> {
            let buffer = self.buffers.get(source_id).cloned().ok_or_else(|| {
                JsValue::from_str(&format!("unprepared media source {source_id:?}"))
            })?;
            let source = self.context.create_buffer_source()?;
            source.set_buffer(Some(&buffer));
            let gain = self.context.create_gain()?;
            source.connect_with_audio_node(&gain)?;
            gain.connect_with_audio_node(&self.context.destination())?;
            let current_time = self.context.current_time();
            let player = self.player_mut(id)?;
            player.source = source_id.clone();
            source.set_loop(player.loops == -1 || player.loops > 1);
            if player.loops > 1 {
                let scheduled: &web_sys::AudioScheduledSourceNode = source.unchecked_ref();
                scheduled
                    .stop_with_when(current_time + buffer.duration() * f64::from(player.loops))?;
            }
            player.active = Some(source.clone());
            player.gain = Some(gain);
            update_gain(player);
            source.start_with_when_and_grain_offset(0.0, player.offset_seconds)?;
            Ok(())
        }

        fn stop(&mut self, id: PlayerId) -> Result<(), JsValue> {
            let player = self.player_mut(id)?;
            if let Some(source) = player.active.take() {
                let scheduled: &web_sys::AudioScheduledSourceNode = source.unchecked_ref();
                let _ = scheduled.stop();
            }
            player.gain = None;
            Ok(())
        }

        fn player_mut(&mut self, id: PlayerId) -> Result<&mut Player, JsValue> {
            self.players
                .get_mut(&id)
                .ok_or_else(|| JsValue::from_str(&format!("unknown MMAPI player {id:?}")))
        }
    }

    fn update_gain(player: &Player) {
        if let Some(gain) = &player.gain {
            gain.gain().set_value(if player.muted {
                0.0
            } else {
                player.level.clamp(0, 100) as f32 / 100.0
            });
        }
    }

    pub fn vibrate(duration_ms: u32) -> bool {
        let Some(window) = web_sys::window() else {
            return false;
        };
        let navigator = window.navigator();
        let Ok(property) = js_sys::Reflect::get(&navigator, &JsValue::from_str("vibrate")) else {
            return false;
        };
        let Ok(function) = property.dyn_into::<js_sys::Function>() else {
            return false;
        };
        function
            .call1(&navigator, &JsValue::from_f64(f64::from(duration_ms)))
            .ok()
            .and_then(|answer| answer.as_bool())
            .unwrap_or(false)
    }
}

#[cfg(target_arch = "wasm32")]
pub use implementation::{vibrate, BrowserAudio};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct BrowserAudio;

#[cfg(not(target_arch = "wasm32"))]
impl BrowserAudio {
    pub fn new() -> Result<Self, String> {
        Err("BrowserAudio is available only on wasm32".to_owned())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn vibrate(_: u32) -> bool {
    false
}
