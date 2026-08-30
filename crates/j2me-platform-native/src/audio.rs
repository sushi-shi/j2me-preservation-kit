//! Native multi-player software mixer and CPAL output endpoint.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use j2me_me::{HostAudioOp, MediaSource, PlayerId};

#[derive(Debug, Clone)]
struct Clip {
    samples: Arc<[f32]>,
}

#[derive(Debug, Clone)]
struct Playback {
    source: MediaSource,
    cursor: usize,
    loops: i32,
    loops_left: i32,
    level: i32,
    muted: bool,
    playing: bool,
}

#[derive(Debug)]
pub struct SoftwareMixer {
    sample_rate: u32,
    clips: HashMap<MediaSource, Clip>,
    players: HashMap<PlayerId, Playback>,
    completed: Vec<PlayerId>,
}

impl SoftwareMixer {
    pub fn new(sample_rate: u32) -> Result<Self, String> {
        if sample_rate == 0 {
            return Err("audio output rate is zero".to_owned());
        }
        Ok(Self {
            sample_rate,
            clips: HashMap::new(),
            players: HashMap::new(),
            completed: Vec::new(),
        })
    }

    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn prepare(
        &mut self,
        source: MediaSource,
        bytes: &[u8],
        mime: &str,
    ) -> Result<Option<&'static str>, String> {
        let decoded = j2me_media::decode_audio(bytes, mime)?;
        let samples = if decoded.sample_rate == self.sample_rate {
            decoded.samples
        } else {
            j2me_media::resample::to_rate(&decoded.samples, decoded.sample_rate, self.sample_rate)
        };
        if samples.is_empty() {
            return Err(format!("decoded {source:?} has no samples"));
        }
        self.clips.insert(
            source,
            Clip {
                samples: samples.into(),
            },
        );
        Ok(decoded.approximation)
    }

    pub fn prepare_resource(
        &mut self,
        resources: &dyn j2me_platform::ResourceSource,
        path: &str,
        mime: &str,
    ) -> Result<Option<&'static str>, String> {
        let bytes = resources
            .bytes(path)
            .ok_or_else(|| format!("missing media resource {path:?}"))?;
        self.prepare(MediaSource::Resource(path.to_owned()), bytes, mime)
    }

    pub fn apply(&mut self, operation: &HostAudioOp) -> Result<(), String> {
        match operation {
            HostAudioOp::Create { player, source, .. } => {
                self.players.insert(
                    *player,
                    Playback {
                        source: source.clone(),
                        cursor: 0,
                        loops: 1,
                        loops_left: 1,
                        level: 100,
                        muted: false,
                        playing: false,
                    },
                );
            }
            HostAudioOp::SetLoopCount { player, count } => {
                let playback = self.player_mut(*player)?;
                playback.loops = *count;
                playback.loops_left = *count;
            }
            HostAudioOp::Start { player, source } => {
                if !self.clips.contains_key(source) {
                    return Err(format!("media source {source:?} was not prepared"));
                }
                let playback = self.player_mut(*player)?;
                playback.source = source.clone();
                playback.loops_left = playback.loops;
                playback.playing = true;
            }
            HostAudioOp::Stop { player, .. } => self.player_mut(*player)?.playing = false,
            HostAudioOp::SetVolume { player, level } => self.player_mut(*player)?.level = *level,
            HostAudioOp::SetMute { player, muted } => self.player_mut(*player)?.muted = *muted,
            HostAudioOp::SetMediaTime {
                player,
                microseconds,
            } => {
                let sample_rate = self.sample_rate;
                let playback = self.player_mut(*player)?;
                playback.cursor = ((*microseconds as u128) * u128::from(sample_rate) / 1_000_000)
                    .min(usize::MAX as u128) as usize;
            }
            HostAudioOp::Close { player } => {
                self.players.remove(player);
            }
            HostAudioOp::Deallocate { player } => self.player_mut(*player)?.playing = false,
            HostAudioOp::Realize { .. } | HostAudioOp::Prefetch { .. } => {}
        }
        Ok(())
    }

    fn player_mut(&mut self, player: PlayerId) -> Result<&mut Playback, String> {
        self.players
            .get_mut(&player)
            .ok_or_else(|| format!("unknown MMAPI player {player:?}"))
    }

    /// Fill interleaved host samples. Every MMAPI player mixes independently;
    /// completion ids are returned for `MediaRuntime::notify_end_of_media`.
    pub fn mix(&mut self, output: &mut [f32], channels: usize) -> Vec<PlayerId> {
        output.fill(0.0);
        if channels == 0 {
            return vec![];
        }
        let mut completed = Vec::new();
        let (clips, players) = (&self.clips, &mut self.players);
        for (id, playback) in players.iter_mut() {
            if !playback.playing {
                continue;
            }
            let Some(clip) = clips.get(&playback.source) else {
                continue;
            };
            let gain = if playback.muted {
                0.0
            } else {
                playback.level.clamp(0, 100) as f32 / 100.0
            };
            for frame in output.chunks_mut(channels) {
                if playback.cursor >= clip.samples.len() {
                    if playback.loops == -1 || playback.loops_left > 1 {
                        if playback.loops_left > 1 {
                            playback.loops_left -= 1;
                        }
                        playback.cursor = 0;
                    } else {
                        playback.playing = false;
                        completed.push(*id);
                        break;
                    }
                }
                let sample = clip.samples[playback.cursor] * gain;
                playback.cursor += 1;
                for channel in frame {
                    *channel += sample;
                }
            }
        }
        for sample in output {
            *sample = sample.clamp(-1.0, 1.0);
        }
        self.completed.extend(completed.iter().copied());
        completed
    }

    /// Completion notifications accumulated by the real-time callback. The
    /// event-loop thread forwards these to `MediaRuntime::notify_end_of_media`.
    pub fn drain_completed(&mut self) -> Vec<PlayerId> {
        std::mem::take(&mut self.completed)
    }
}

pub struct NativeAudioOutput {
    mixer: Arc<Mutex<SoftwareMixer>>,
    _stream: cpal::Stream,
}

impl NativeAudioOutput {
    pub fn open_default() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| "no default audio output device".to_owned())?;
        let supported = device
            .default_output_config()
            .map_err(|error| error.to_string())?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = usize::from(config.channels);
        let mixer = Arc::new(Mutex::new(SoftwareMixer::new(config.sample_rate)?));
        let stream = match sample_format {
            cpal::SampleFormat::F32 => build_f32(&device, config, Arc::clone(&mixer), channels),
            cpal::SampleFormat::I16 => build_i16(&device, config, Arc::clone(&mixer), channels),
            cpal::SampleFormat::U16 => build_u16(&device, config, Arc::clone(&mixer), channels),
            format => return Err(format!("unsupported native sample format {format:?}")),
        }?;
        stream.play().map_err(|error| error.to_string())?;
        Ok(Self {
            mixer,
            _stream: stream,
        })
    }

    pub fn mixer(&self) -> Arc<Mutex<SoftwareMixer>> {
        Arc::clone(&self.mixer)
    }
}

fn build_f32(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mixer: Arc<Mutex<SoftwareMixer>>,
    channels: usize,
) -> Result<cpal::Stream, String> {
    device
        .build_output_stream(
            config,
            move |data: &mut [f32], _| {
                if let Ok(mut mixer) = mixer.lock() {
                    mixer.mix(data, channels);
                } else {
                    data.fill(0.0);
                }
            },
            |error| eprintln!("native audio stream error: {error}"),
            None,
        )
        .map_err(|error| error.to_string())
}

fn build_i16(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mixer: Arc<Mutex<SoftwareMixer>>,
    channels: usize,
) -> Result<cpal::Stream, String> {
    let mut float = Vec::new();
    device
        .build_output_stream(
            config,
            move |data: &mut [i16], _| {
                float.resize(data.len(), 0.0);
                if let Ok(mut mixer) = mixer.lock() {
                    mixer.mix(&mut float, channels);
                }
                for (out, sample) in data.iter_mut().zip(float.iter().copied()) {
                    *out = (sample * i16::MAX as f32) as i16;
                }
            },
            |error| eprintln!("native audio stream error: {error}"),
            None,
        )
        .map_err(|error| error.to_string())
}

fn build_u16(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mixer: Arc<Mutex<SoftwareMixer>>,
    channels: usize,
) -> Result<cpal::Stream, String> {
    let mut float = Vec::new();
    device
        .build_output_stream(
            config,
            move |data: &mut [u16], _| {
                float.resize(data.len(), 0.0);
                if let Ok(mut mixer) = mixer.lock() {
                    mixer.mix(&mut float, channels);
                }
                for (out, sample) in data.iter_mut().zip(float.iter().copied()) {
                    *out = ((sample * 0.5 + 0.5) * u16::MAX as f32) as u16;
                }
            },
            |error| eprintln!("native audio stream error: {error}"),
            None,
        )
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_mix_independently_with_volume_mute_and_loops() {
        let mut mixer = SoftwareMixer::new(8_000).unwrap();
        let source = MediaSource::Opaque(1);
        mixer.clips.insert(
            source.clone(),
            Clip {
                samples: vec![0.5, -0.5].into(),
            },
        );
        let player = PlayerId(0);
        mixer
            .apply(&HostAudioOp::Create {
                player,
                source: source.clone(),
                mime: "test".into(),
            })
            .unwrap();
        mixer
            .apply(&HostAudioOp::SetLoopCount { player, count: 2 })
            .unwrap();
        mixer
            .apply(&HostAudioOp::SetVolume { player, level: 50 })
            .unwrap();
        mixer.apply(&HostAudioOp::Start { player, source }).unwrap();
        let mut output = [0.0; 5];
        assert_eq!(mixer.mix(&mut output, 1), vec![player]);
        assert_eq!(output, [0.25, -0.25, 0.25, -0.25, 0.0]);
        assert_eq!(mixer.drain_completed(), vec![player]);
    }
}
