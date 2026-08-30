//! Band-limited sample-rate conversion for host output.
//!
//! Java ME tracks often arrive at rates a native sound card does not expose:
//! AMR-NB decodes at 8 kHz while synthesized MIDI commonly renders at
//! 44.1 kHz. A browser can hand both to WebAudio, which resamples them itself.
//! A native host has to do it, and doing it naively is audible: repeating
//! or dropping samples to stretch 8 kHz speech up to 48 kHz mirrors the
//! signal's whole spectrum around every multiple of 4 kHz.
//!
//! So this converts properly, with a windowed-sinc kernel: 32 taps, a
//! Blackman window, and a cutoff that follows the *lower* of the two rates
//! so downsampling cannot alias either. It runs once per track when the host
//! prepares it, never in the audio callback, so the cost is irrelevant and
//! the quality is not.
//!
//! This lives in the shared crate rather than in the desktop host because it
//! is signal processing, and the repository keeps signal processing in Rust
//! where every client can reach it.

/// Taps on each side of the interpolation point.
const HALF_TAPS: isize = 16;

/// Resamples mono samples from one rate to another.
///
/// Returns the input untouched when the rates already match, and an empty
/// vector for empty input or a zero rate, so a caller cannot divide by zero
/// or index an empty kernel.
pub fn to_rate(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    if from == 0 || to == 0 {
        return Vec::new();
    }
    let ratio = f64::from(from) / f64::from(to);
    // Downsampling must lower the cutoff to the new Nyquist; upsampling
    // keeps the source's own band and simply fills in between samples.
    let cutoff = if to < from {
        f64::from(to) / f64::from(from)
    } else {
        1.0
    };
    let output_len = ((input.len() as f64) / ratio).ceil() as usize;
    let mut output = Vec::with_capacity(output_len);
    for index in 0..output_len {
        let position = index as f64 * ratio;
        let centre = position.floor() as isize;
        let mut sum = 0.0f64;
        let mut weight = 0.0f64;
        for tap in -HALF_TAPS..=HALF_TAPS {
            let source = centre + tap;
            if source < 0 || source as usize >= input.len() {
                continue;
            }
            let distance = position - source as f64;
            let value = kernel(distance, cutoff);
            sum += f64::from(input[source as usize]) * value;
            weight += value;
        }
        // Normalising by the realised weight keeps the signal level flat at
        // the edges, where part of the kernel hangs off the end of the input.
        let sample = if weight.abs() > 1e-12 {
            sum / weight
        } else {
            0.0
        };
        output.push(sample as f32);
    }
    output
}

/// One windowed-sinc tap at `distance` input samples from the centre.
fn kernel(distance: f64, cutoff: f64) -> f64 {
    let span = HALF_TAPS as f64;
    if distance.abs() > span {
        return 0.0;
    }
    let sinc = {
        let x = std::f64::consts::PI * cutoff * distance;
        if x.abs() < 1e-9 {
            1.0
        } else {
            x.sin() / x
        }
    };
    // Blackman window over the full kernel width.
    let phase = std::f64::consts::PI * (distance + span) / span;
    let window = 0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos();
    sinc * window
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f32]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples
            .iter()
            .map(|&s| f64::from(s) * f64::from(s))
            .sum::<f64>()
            / samples.len() as f64)
            .sqrt()
    }

    #[test]
    fn matching_rates_and_degenerate_input_are_handled_without_panicking() {
        let input = vec![0.1, -0.2, 0.3];
        assert_eq!(to_rate(&input, 8000, 8000), input);
        assert!(to_rate(&[], 8000, 48000).is_empty());
        assert!(to_rate(&input, 0, 48000).is_empty());
        assert!(to_rate(&input, 8000, 0).is_empty());
        // A single sample must not index off either end of the kernel.
        assert_eq!(to_rate(&[0.5], 8000, 48000).len(), 6);
    }

    #[test]
    fn the_output_length_follows_the_rate_ratio() {
        let input = vec![0.0f32; 800]; // 0.1 s at 8 kHz
        assert_eq!(to_rate(&input, 8000, 48000).len(), 4800);
        assert_eq!(to_rate(&input, 8000, 44100).len(), 4410);
        let input = vec![0.0f32; 44100];
        assert_eq!(to_rate(&input, 44100, 48000).len(), 48000);
        assert_eq!(to_rate(&input, 44100, 22050).len(), 22050);
    }

    /// A tone well inside the passband survives with its level intact.
    ///
    /// This is the property a naive resampler fails: it would either change
    /// the level or fold energy in from the images it creates.
    #[test]
    fn a_passband_tone_keeps_its_amplitude_and_frequency() {
        let rate = 8000u32;
        let frequency = 500.0f64;
        let input: Vec<f32> = (0..8000)
            .map(|n| {
                (2.0 * std::f64::consts::PI * frequency * f64::from(n) / f64::from(rate)).sin()
                    as f32
            })
            .collect();
        let output = to_rate(&input, rate, 48000);
        // Ignore the kernel-width edges, where the window is still ramping.
        let interior = &output[600..output.len() - 600];
        let expected = 1.0 / 2.0f64.sqrt();
        assert!(
            (rms(interior) - expected).abs() < 0.01,
            "RMS {} drifted from {expected}",
            rms(interior)
        );
        // The tone crosses zero at 2 * frequency per second; count the
        // upward crossings to confirm the pitch did not move.
        let crossings = interior
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        let seconds = interior.len() as f64 / 48000.0;
        let measured = crossings as f64 / seconds;
        assert!(
            (measured - frequency).abs() < 5.0,
            "measured {measured} Hz, expected {frequency} Hz"
        );
    }

    /// Downsampling rejects content above the new Nyquist instead of folding
    /// it back down as a spurious low tone.
    #[test]
    fn downsampling_rejects_content_above_the_new_nyquist() {
        let rate = 48000u32;
        // 15 kHz is above the 4 kHz Nyquist of the 8 kHz target, so a
        // resampler without a lowpass would alias it into the audible band
        // at full strength.
        let input: Vec<f32> = (0..48000)
            .map(|n| {
                (2.0 * std::f64::consts::PI * 15000.0 * f64::from(n) / f64::from(rate)).sin() as f32
            })
            .collect();
        let output = to_rate(&input, rate, 8000);
        let interior = &output[200..output.len() - 200];
        assert!(
            rms(interior) < 0.05,
            "aliased energy {} survived the downsample",
            rms(interior)
        );
    }

    #[test]
    fn conversion_is_deterministic() {
        let input: Vec<f32> = (0..2000)
            .map(|n| ((n * 37) % 71) as f32 / 71.0 - 0.5)
            .collect();
        assert_eq!(to_rate(&input, 8000, 48000), to_rate(&input, 8000, 48000));
    }
}
