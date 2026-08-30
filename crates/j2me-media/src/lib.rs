//! Portable audio formats and DSP for Java ME hosts.

mod error;
mod writer;

pub mod amr;
pub mod midi;
pub mod resample;
pub mod smaf;
pub mod synth;
pub mod wav;

pub use error::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    /// Present when the rendered waveform is a documented host approximation.
    pub approximation: Option<&'static str>,
}

/// Decode/render the standard Java ME formats supported by the shared host.
/// This has no output-device dependency and is usable by native and browser
/// adapters alike.
pub fn decode_audio(bytes: &[u8], mime: &str) -> std::result::Result<DecodedAudio, String> {
    match mime.trim().to_ascii_lowercase().as_str() {
        "audio/amr" | "audio/amr-nb" => {
            let track = amr::parse(bytes).map_err(|error| error.to_string())?;
            Ok(DecodedAudio {
                samples: track.decode_f32().map_err(|error| error.to_string())?,
                sample_rate: amr::SAMPLE_RATE,
                approximation: None,
            })
        }
        "audio/midi" | "audio/mid" | "audio/x-midi" | "audio/sp-midi" => {
            let song = midi::parse(bytes).map_err(|error| error.to_string())?;
            Ok(DecodedAudio {
                samples: synth::render(&song),
                sample_rate: synth::SAMPLE_RATE,
                approximation: Some(
                    "Approximate MIDI synthesis: phone vendor instruments are not emulated.",
                ),
            })
        }
        "application/vnd.smaf" | "audio/mmf" | "audio/x-smaf" => {
            let file = smaf::parse(bytes).map_err(|error| error.to_string())?;
            let smf = file
                .to_approximate_smf()
                .map_err(|error| error.to_string())?;
            let song = midi::parse(&smf).map_err(|error| error.to_string())?;
            Ok(DecodedAudio {
                samples: synth::render(&song),
                sample_rate: synth::SAMPLE_RATE,
                approximation: Some(smaf::APPROXIMATION_NOTICE),
            })
        }
        "audio/wav" | "audio/x-wav" | "audio/wave" => {
            let (samples, sample_rate) = wav::decode_pcm16_mono(bytes, "media asset")?;
            Ok(DecodedAudio {
                samples,
                sample_rate,
                approximation: None,
            })
        }
        _ => Err(format!("unsupported Java ME media content type {mime:?}")),
    }
}
