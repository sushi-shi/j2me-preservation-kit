//! A deterministic, explicitly approximate General MIDI fallback synthesizer.
//!
//! Phones rendered MIDI through vendor ROM synthesizers that are not carried
//! by game archives. This renderer preserves note timing, keys, velocities,
//! programs and tempo, but its timbre is a host approximation rather than a
//! recovered phone implementation. It uses no sampled instrument or borrowed
//! sound font.
//!
//! # How a voice is built
//!
//! Three things separate a synthesizer from a buzzer, and this has all three.
//!
//! *Band-limited wavetables.* Each family is described by a harmonic recipe —
//! the relative strength of each partial — and [`WaveSet`] renders that recipe
//! into one table per octave, dropping every partial that would sit above
//! Nyquist for the notes that table serves. A voice then reads the table for
//! its own octave, so nothing can alias no matter how high the part is
//! written. Recipes are also what makes a flute a flute: the difference
//! between a flute, a koto and a string ensemble is mostly which partials they
//! carry, and a synthesizer built from a fixed sawtooth cannot express that
//! difference however it is enveloped.
//!
//! *A filter that moves.* Real instruments are brightest at the attack and
//! grow darker as the note decays; a static spectrum is the single most
//! recognisable "cheap synthesizer" trait. Every voice runs through a
//! resonant low-pass whose cutoff has its own envelope, so the koto's attack
//! is bright and its tail is round.
//!
//! *Motion.* Two detuned oscillators make an ensemble sound like more than one
//! player, and a delayed vibrato keeps sustained flute and string notes from
//! sitting perfectly still.
//!
//! The current family set covers koto, synth and string ensembles, synth bass,
//! flutes, muted and overdriven guitars, and percussion. Programs outside that
//! reviewed subset fall back to the soft lead.
//!
//! The whole render is deterministic: the tables are built from a fixed sine
//! table, the noise used by the drums is seeded from each hit's position in
//! the song, and nothing consults a clock or an allocator address.

use std::sync::OnceLock;

use crate::midi::MidiSong;

/// The fixed render rate. 44.1 kHz keeps the wavetables' interpolation residue
/// above the audible program content.
pub const SAMPLE_RATE: u32 = 44_100;

/// The loudness every rendered song is normalized to, as RMS over the whole
/// render.
///
/// Peak normalization — what this renderer used to do — says nothing about how
/// loud a track sounds, because one stray transient sets the peak for the
/// entire song. Average energy is what a listener hears, and it is what the
/// tests pin. The MMAPI host applies `VolumeControl` after synthesis.
const TARGET_RMS: f32 = 0.16;

/// The absolute ceiling after normalization, leaving headroom under full
/// scale so that no host's resampling can push a sample into clipping.
const PEAK_CEILING: f32 = 0.9;

/// Samples per wavetable. A power of two so the phase index wraps by mask.
const TABLE_LEN: usize = 2048;
const TABLE_MASK: usize = TABLE_LEN - 1;

/// One table per MIDI octave: twelve keys each, eleven bands covering the
/// whole 0..=127 key range.
const BANDS: usize = 11;

/// The most partials any recipe may ask for. Beyond this the detail is
/// inaudible against this score's own noise floor and only costs table-build
/// time.
const MAX_PARTIALS: usize = 48;

/// How often the filter recomputes its coefficient, in samples.
///
/// The cutoff envelope moves over tens of milliseconds, so updating it at
/// roughly 700 Hz is inaudible and avoids a transcendental call per sample
/// per voice.
const FILTER_INTERVAL: usize = 64;

/// A quarter-turn sine table, shared by every wavetable build.
///
/// Building the tables from one table of sines rather than from `f32::sin`
/// calls keeps the result identical everywhere: the only transcendental
/// evaluations in the whole synthesizer happen here, once.
fn sine_table() -> &'static [f32; TABLE_LEN] {
    static TABLE: OnceLock<[f32; TABLE_LEN]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0.0f32; TABLE_LEN];
        for (index, slot) in table.iter_mut().enumerate() {
            *slot = (std::f32::consts::TAU * index as f32 / TABLE_LEN as f32).sin();
        }
        table
    })
}

/// A harmonic recipe: the relative amplitude of partial 1, 2, 3, ...
///
/// These are the timbres themselves. A partial list that falls off slowly is
/// bright and reedy; one that falls off as `1/n` is a sawtooth and reads as a
/// bowed string; one with almost nothing above the fundamental is a flute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recipe {
    /// Plucked zither: strong low partials with a bright, quickly thinning
    /// upper set, which is what gives a koto its wooden attack.
    Koto,
    /// Nearly a sine with a trace of the second and third partial.
    Flute,
    /// Sawtooth-like, dense: many partials falling as `1/n`.
    Strings,
    /// Fundamental-heavy with a firm second and third, for synth bass.
    Bass,
    /// Muted guitar: a compact set of strong low partials and little above.
    MutedGuitar,
    /// Overdriven guitar: dense partials, then soft-clipped at render.
    OverdrivenGuitar,
    /// Hollow odd-harmonic tone for anything unmapped.
    Lead,
}

impl Recipe {
    const ALL: [Recipe; 7] = [
        Recipe::Koto,
        Recipe::Flute,
        Recipe::Strings,
        Recipe::Bass,
        Recipe::MutedGuitar,
        Recipe::OverdrivenGuitar,
        Recipe::Lead,
    ];

    fn index(self) -> usize {
        Recipe::ALL.iter().position(|&r| r == self).unwrap_or(0)
    }

    /// The partial amplitudes, generated rather than tabulated so the shape of
    /// each timbre is legible as a rule instead of as a list of numbers.
    fn partials(self) -> Vec<f32> {
        let mut partials = Vec::with_capacity(MAX_PARTIALS);
        for harmonic in 1..=MAX_PARTIALS {
            let n = harmonic as f32;
            let amplitude = match self {
                // A struck string over a wooden box: the even partials are
                // held back, and a broad resonance around the fourth to
                // eighth partial stands in for the body. Without that
                // resonance the recipe is just another `1/n^a` curve and
                // comes out barely distinguishable from the synth bass,
                // which is what the waveform comparison in the tests caught.
                Recipe::Koto => {
                    let even = if harmonic % 2 == 0 { 0.55 } else { 1.0 };
                    let body = 1.0 + 1.6 * (-(((n - 5.5) / 2.4).powi(2))).exp();
                    even * body / n.powf(1.1)
                }
                // Three partials and nothing else; the breath noise carries
                // the rest of the character.
                Recipe::Flute => match harmonic {
                    1 => 1.0,
                    2 => 0.16,
                    3 => 0.07,
                    4 => 0.02,
                    _ => 0.0,
                },
                // A sawtooth. Two of these detuned against each other is the
                // classic string-ensemble sound, and it is what the score's
                // two ensemble programs want.
                Recipe::Strings => 1.0 / n,
                // A narrow pulse, which is what a synth bass of this era
                // usually is. The duty cycle puts nulls at every fourth
                // partial, and that comb is what makes it structurally
                // different from the ensemble's sawtooth rather than merely a
                // steeper version of it. Its low cutoff is what keeps it
                // under the mix.
                Recipe::Bass => {
                    let duty = 0.25;
                    (std::f32::consts::PI * n * duty).sin().abs() / n
                }
                // Palm-muted: a handful of partials, sharply truncated.
                Recipe::MutedGuitar => {
                    if harmonic <= 6 {
                        1.0 / n.powf(0.9)
                    } else {
                        0.0
                    }
                }
                // Dense and even; the soft clipping at render adds the rest.
                Recipe::OverdrivenGuitar => 1.0 / n.powf(0.75),
                // Odd partials only: a hollow, square-ish lead.
                Recipe::Lead => {
                    if harmonic % 2 == 1 {
                        1.0 / n
                    } else {
                        0.0
                    }
                }
            };
            partials.push(amplitude);
        }
        partials
    }
}

/// One recipe rendered into a band-limited table per octave.
struct WaveSet {
    bands: Vec<Vec<f32>>,
}

impl WaveSet {
    fn build(recipe: Recipe) -> WaveSet {
        let partials = recipe.partials();
        let sine = sine_table();
        let nyquist = SAMPLE_RATE as f32 / 2.0;
        let mut bands = Vec::with_capacity(BANDS);
        // Every band is scaled by the same factor so that a melody crossing an
        // octave boundary does not step in level; the higher bands come out
        // quieter on their own, which is what losing partials should sound
        // like.
        let mut reference_peak = 1.0f32;
        for band in 0..BANDS {
            // The highest note this band serves decides how many partials fit.
            let top_key = (band * 12 + 11).min(127) as u8;
            let top = key_frequency(top_key);
            let limit = ((nyquist / top).floor() as usize).clamp(1, partials.len());
            let mut table = vec![0.0f32; TABLE_LEN];
            for (index, slot) in table.iter_mut().enumerate() {
                let mut value = 0.0f32;
                for harmonic in 1..=limit {
                    let amplitude = partials[harmonic - 1];
                    if amplitude == 0.0 {
                        continue;
                    }
                    value += amplitude * sine[(harmonic * index) & TABLE_MASK];
                }
                *slot = value;
            }
            let peak = table.iter().fold(0.0f32, |peak, &s| peak.max(s.abs()));
            if band == 0 {
                reference_peak = peak.max(f32::EPSILON);
            }
            for slot in table.iter_mut() {
                *slot /= reference_peak;
            }
            bands.push(table);
        }
        WaveSet { bands }
    }

    /// One interpolated sample from the table serving `key`.
    fn sample(&self, key: u8, phase: f32) -> f32 {
        let band = ((key / 12) as usize).min(BANDS - 1);
        let table = &self.bands[band];
        let position = phase * TABLE_LEN as f32;
        let index = position as usize;
        let fraction = position - index as f32;
        let low = table[index & TABLE_MASK];
        let high = table[(index + 1) & TABLE_MASK];
        low + (high - low) * fraction
    }
}

fn wave_sets() -> &'static [WaveSet; 7] {
    static SETS: OnceLock<[WaveSet; 7]> = OnceLock::new();
    SETS.get_or_init(|| Recipe::ALL.map(WaveSet::build))
}

/// A zero-delay-feedback state-variable filter.
///
/// The topology matters: the naive Chamberlin form goes unstable above about a
/// sixth of the sample rate, which is inside the range these voices open their
/// cutoff to on an attack. This one is stable at any cutoff below Nyquist.
#[derive(Default, Clone, Copy)]
struct Filter {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Filter {
    /// Recomputes the coefficients for a cutoff in Hz and a resonance `q`.
    fn tune(&mut self, cutoff: f32, q: f32, rate: f32) {
        let cutoff = cutoff.clamp(20.0, rate * 0.45);
        let g = (std::f32::consts::PI * cutoff / rate).tan();
        let k = 1.0 / q.max(0.5);
        self.a1 = 1.0 / (1.0 + g * (g + k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
        self.k = k;
    }

    fn low_pass(&mut self, input: f32) -> f32 {
        let v3 = input - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v2
    }

    fn high_pass(&mut self, input: f32) -> f32 {
        let v3 = input - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        input - self.k * v1 - v2
    }
}

/// One voice: a recipe, an amplitude envelope, a filter envelope, and motion.
#[derive(Debug, Clone, Copy)]
struct Voice {
    recipe: Recipe,
    /// A second oscillator detuned by this many cents, mixed in at `blend`.
    detune_cents: f32,
    blend: f32,
    /// Amplitude envelope, in seconds and a 0..=1 sustain level. A zero
    /// sustain means a plucked or struck note that dies on its own however
    /// long the score holds it.
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    /// Filter cutoff at the attack and at the sustain, as multiples of the
    /// note's own fundamental. Expressing them relative to the note keeps a
    /// bass and a lead equally bright in their own registers.
    cutoff_peak: f32,
    cutoff_sustain: f32,
    /// Seconds for the cutoff to fall from peak to sustain.
    cutoff_decay: f32,
    /// Filter resonance. Above 1 the cutoff region is emphasised, which gives
    /// synthetic families their character.
    resonance: f32,
    /// Soft-clipping drive. 1.0 passes through untouched.
    drive: f32,
    /// Vibrato depth in cents, its rate in Hz, and the delay before it starts.
    vibrato_cents: f32,
    vibrato_hz: f32,
    vibrato_delay: f32,
    /// Breath noise mixed in under the envelope.
    breath: f32,
    /// Per-family output level, balancing families that carry many notes
    /// against families that carry few.
    level: f32,
}

/// The base voice every family starts from, so each definition below shows
/// only what makes that family different.
const BASE: Voice = Voice {
    recipe: Recipe::Lead,
    detune_cents: 0.0,
    blend: 0.0,
    attack: 0.005,
    decay: 0.3,
    sustain: 0.5,
    release: 0.12,
    cutoff_peak: 12.0,
    cutoff_sustain: 5.0,
    cutoff_decay: 0.25,
    resonance: 0.9,
    drive: 1.0,
    vibrato_cents: 0.0,
    vibrato_hz: 5.0,
    vibrato_delay: 0.25,
    breath: 0.0,
    level: 0.85,
};

const LEAD: Voice = Voice {
    recipe: Recipe::Lead,
    detune_cents: 7.0,
    blend: 0.3,
    resonance: 1.4,
    ..BASE
};

/// The koto lead: instant attack, no sustain, and a cutoff that closes fast,
/// which is what turns a bright pluck into a wooden one.
const KOTO: Voice = Voice {
    recipe: Recipe::Koto,
    detune_cents: 4.0,
    blend: 0.18,
    attack: 0.001,
    decay: 0.85,
    sustain: 0.0,
    release: 0.16,
    cutoff_peak: 22.0,
    cutoff_sustain: 3.0,
    cutoff_decay: 0.22,
    resonance: 1.1,
    level: 1.0,
    ..BASE
};

/// Muted guitar: a short pluck with very little sustain and a dark filter.
const MUTED_GUITAR: Voice = Voice {
    recipe: Recipe::MutedGuitar,
    detune_cents: -6.0,
    blend: 0.25,
    attack: 0.002,
    decay: 0.35,
    sustain: 0.05,
    release: 0.10,
    cutoff_peak: 10.0,
    cutoff_sustain: 2.5,
    cutoff_decay: 0.12,
    resonance: 0.9,
    level: 0.95,
    ..BASE
};

/// Overdriven guitar: dense partials, driven into soft clipping, sustaining
/// the way a distorted string does.
///
/// The filter stays open. An overdrive that is darker than the string
/// ensemble it plays against is the wrong instrument however it is
/// enveloped, and measurement caught exactly that in the first draft of these
/// numbers.
const OVERDRIVEN_GUITAR: Voice = Voice {
    recipe: Recipe::OverdrivenGuitar,
    detune_cents: 9.0,
    blend: 0.4,
    attack: 0.004,
    decay: 0.5,
    sustain: 0.4,
    release: 0.18,
    cutoff_peak: 16.0,
    cutoff_sustain: 9.0,
    cutoff_decay: 0.35,
    resonance: 1.2,
    drive: 4.0,
    level: 0.75,
    ..BASE
};

/// Synth bass: fundamental-heavy, with the filter kept low so it stays under
/// the arrangement.
const BASS: Voice = Voice {
    recipe: Recipe::Bass,
    detune_cents: 0.0,
    blend: 0.0,
    attack: 0.006,
    decay: 0.4,
    sustain: 0.7,
    release: 0.10,
    cutoff_peak: 7.0,
    cutoff_sustain: 3.0,
    cutoff_decay: 0.18,
    resonance: 1.3,
    level: 1.2,
    ..BASE
};

/// String and synth ensembles: slow in, full sustain, slow out, two saws
/// detuned against each other, and a vibrato that arrives after the note has
/// settled.
const STRINGS: Voice = Voice {
    recipe: Recipe::Strings,
    detune_cents: 11.0,
    blend: 0.5,
    attack: 0.11,
    decay: 0.25,
    sustain: 0.85,
    release: 0.28,
    cutoff_peak: 8.0,
    cutoff_sustain: 6.0,
    cutoff_decay: 0.5,
    resonance: 0.8,
    vibrato_cents: 5.0,
    vibrato_hz: 4.6,
    vibrato_delay: 0.35,
    level: 0.72,
    ..BASE
};

/// Flutes: almost a pure tone, with breath noise and a slow vibrato doing the
/// work that harmonics do elsewhere.
const FLUTE: Voice = Voice {
    recipe: Recipe::Flute,
    detune_cents: 0.0,
    blend: 0.0,
    attack: 0.06,
    decay: 0.2,
    sustain: 0.9,
    release: 0.14,
    cutoff_peak: 6.0,
    cutoff_sustain: 4.0,
    cutoff_decay: 0.3,
    resonance: 0.7,
    vibrato_cents: 9.0,
    vibrato_hz: 5.2,
    vibrato_delay: 0.22,
    breath: 0.05,
    level: 0.85,
    ..BASE
};

fn voice_for(program: u8) -> Voice {
    match program {
        // GM 28 is the muted guitar; 29..=31 are the overdriven and distorted
        // ones, and the score uses both sides of that line.
        24..=28 => MUTED_GUITAR,
        29..=31 => OVERDRIVEN_GUITAR,
        32..=39 => BASS,
        40..=55 => STRINGS,
        72..=79 => FLUTE,
        // GM's ethnic block, which includes the koto program,
        // plus the chromatic percussion block above the pianos.
        8..=15 | 104..=111 => KOTO,
        _ => LEAD,
    }
}

/// The percussion voices, chosen by GM drum key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drum {
    Kick,
    Snare,
    ClosedHat,
    OpenHat,
    Cymbal,
    Tom,
}

/// The GM drum map, to the extent this renderer distinguishes it.
fn drum_for(key: u8) -> Drum {
    match key {
        35 | 36 => Drum::Kick,
        37..=40 => Drum::Snare,
        42 | 44 => Drum::ClosedHat,
        46 => Drum::OpenHat,
        49 | 51 | 52 | 55 | 57 | 59 => Drum::Cymbal,
        41 | 43 | 45 | 47 | 48 | 50 => Drum::Tom,
        _ => Drum::ClosedHat,
    }
}

fn key_frequency(key: u8) -> f32 {
    440.0 * ((f32::from(key) - 69.0) / 12.0).exp2()
}

fn detune(frequency: f32, cents: f32) -> f32 {
    frequency * (cents / 1200.0).exp2()
}

/// Soft clipping. `drive` of 1 is a no-op; above that the curve compresses
/// peaks the way an overdriven amplifier does, adding partials rather than
/// slicing the top off a wave.
fn saturate(sample: f32, drive: f32) -> f32 {
    if drive <= 1.0 {
        sample
    } else {
        (sample * drive).tanh() / drive.tanh()
    }
}

/// The amplitude envelope at `index` samples into a note held for `held`.
fn envelope(voice: &Voice, rate: f32, index: usize, held: usize) -> f32 {
    let seconds = index as f32 / rate;
    let held_seconds = held as f32 / rate;
    let attack_decay = |seconds: f32| -> f32 {
        if seconds < voice.attack {
            seconds / voice.attack.max(f32::EPSILON)
        } else {
            let into_decay = seconds - voice.attack;
            // Exponential rather than linear: a decaying string loses most of
            // its energy early, which is what makes a pluck read as a pluck.
            let decayed = (-3.0 * into_decay / voice.decay.max(f32::EPSILON)).exp();
            voice.sustain + (1.0 - voice.sustain) * decayed
        }
    };
    if seconds < held_seconds {
        attack_decay(seconds)
    } else {
        let into_release = seconds - held_seconds;
        let fade = (-4.0 * into_release / voice.release.max(f32::EPSILON)).exp();
        attack_decay(held_seconds) * fade
    }
}

/// Renders one parsed song to mono f32 PCM at [`SAMPLE_RATE`], normalized to
/// [`TARGET_RMS`] and bounded by [`PEAK_CEILING`].
pub fn render(song: &MidiSong) -> Vec<f32> {
    let rate = SAMPLE_RATE as f32;
    let length_samples = ((song.duration_us as f64 / 1_000_000.0) * f64::from(SAMPLE_RATE)).ceil()
        as usize
        + SAMPLE_RATE as usize / 2;
    let mut mix = vec![0.0f32; length_samples];
    for note in &song.notes {
        let start = ((note.start_us as f64 / 1_000_000.0) * f64::from(SAMPLE_RATE)) as usize;
        let held = ((note.duration_us as f64 / 1_000_000.0) * f64::from(SAMPLE_RATE)) as usize;
        if note.channel == 9 {
            let velocity = f32::from(note.velocity) / 127.0;
            render_drum(&mut mix, start, drum_for(note.key), velocity, rate);
            continue;
        }
        render_pitched(&mut mix, start, held, &voice_for(note.program), note, rate);
    }
    normalize(&mut mix);
    mix
}

fn render_pitched(
    mix: &mut [f32],
    start: usize,
    held: usize,
    voice: &Voice,
    note: &crate::midi::MidiNote,
    rate: f32,
) {
    // A note stops when its envelope is inaudible, not when the score says so:
    // a plucked note with no sustain has already died long before a whole-bar
    // duration elapses, and a sustained one still needs its release.
    let tail = (rate * (voice.release * 2.0 + 0.05)) as usize;
    let natural = if voice.sustain <= f32::EPSILON {
        (rate * voice.decay * 2.5) as usize
    } else {
        usize::MAX - held.max(1)
    };
    let total = held.min(natural) + tail;
    if total == 0 {
        return;
    }
    let amplitude = 0.09 * voice.level * (0.25 + 0.75 * (f32::from(note.velocity) / 127.0));
    let fundamental = key_frequency(note.key);
    let waves = &wave_sets()[voice.recipe.index()];

    let mut phase = 0.0f32;
    let mut second_phase = 0.0f32;
    let mut filter = Filter::default();
    filter.tune(fundamental * voice.cutoff_peak, voice.resonance, rate);
    // A per-note noise stream for breath, seeded from the note's position so
    // two identical notes at different times still differ, and every render
    // of the same song is identical.
    let mut noise_state: u32 =
        0x8b2c_9f31 ^ (start as u32).wrapping_mul(2_246_822_519) ^ u32::from(note.key);

    for index in 0..total {
        let Some(slot) = mix.get_mut(start + index) else {
            break;
        };
        let level = envelope(voice, rate, index, held);
        if level <= 0.0001 && index > held {
            break;
        }
        let seconds = index as f32 / rate;

        if index % FILTER_INTERVAL == 0 {
            // The cutoff falls from its attack value toward its sustain value
            // on its own curve, independent of the amplitude envelope.
            let progress = (-seconds / voice.cutoff_decay.max(f32::EPSILON)).exp();
            let multiple =
                voice.cutoff_sustain + (voice.cutoff_peak - voice.cutoff_sustain) * progress;
            filter.tune(fundamental * multiple, voice.resonance, rate);
        }

        // Vibrato, delayed so that short notes stay steady.
        let frequency = if voice.vibrato_cents > 0.0 && seconds > voice.vibrato_delay {
            let into = seconds - voice.vibrato_delay;
            // Fade the depth in over a quarter second rather than switching it
            // on, which would be audible as a click in the pitch.
            let depth = voice.vibrato_cents * (into * 4.0).min(1.0);
            let sweep = (std::f32::consts::TAU * voice.vibrato_hz * into).sin();
            detune(fundamental, depth * sweep)
        } else {
            fundamental
        };
        let step = frequency / rate;

        let mut sample = waves.sample(note.key, phase);
        if voice.blend > 0.0 {
            let blended = waves.sample(note.key, second_phase);
            sample = sample * (1.0 - voice.blend) + blended * voice.blend;
            second_phase += detune(frequency, voice.detune_cents) / rate;
            if second_phase >= 1.0 {
                second_phase -= 1.0;
            }
        }
        if voice.breath > 0.0 {
            noise_state = noise_state
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let white = (noise_state >> 8) as f32 / 8_388_608.0 - 1.0;
            sample += white * voice.breath;
        }
        // Distortion before the filter, as a real signal chain has it: an
        // overdriven amplifier generates the extra partials and the cabinet
        // shapes them afterwards. Filtering first and clipping second would
        // put the harmonics the filter just removed straight back in.
        sample = saturate(sample, voice.drive);
        sample = filter.low_pass(sample);

        *slot += sample * amplitude * level;
        phase += step;
        if phase >= 1.0 {
            phase -= 1.0;
        }
    }
}

/// The drum voices: a pitch-dropping sine for the kick, filtered noise plus a
/// body tone for the snare and toms, and high-passed noise for the metal.
fn render_drum(mix: &mut [f32], start: usize, drum: Drum, velocity: f32, rate: f32) {
    let (seconds, amplitude) = match drum {
        Drum::Kick => (0.24, 0.5),
        Drum::Snare => (0.20, 0.34),
        Drum::ClosedHat => (0.05, 0.13),
        Drum::OpenHat => (0.26, 0.13),
        Drum::Cymbal => (0.60, 0.15),
        Drum::Tom => (0.28, 0.3),
    };
    let total = (rate * seconds) as usize;
    // A deterministic noise stream per hit: the position in the song seeds it,
    // so the render stays reproducible sample for sample.
    let mut noise_state: u32 = 0x9e37_79b9 ^ (start as u32).wrapping_mul(2_654_435_761);
    let mut phase = 0.0f32;
    let mut body_phase = 0.0f32;
    let mut shaper = Filter::default();
    // Each metal voice is noise through a fixed filter rather than a
    // one-sample difference, which is a far steeper and thinner shape than any
    // cymbal has.
    match drum {
        Drum::ClosedHat => shaper.tune(7_500.0, 0.8, rate),
        Drum::OpenHat => shaper.tune(6_500.0, 0.8, rate),
        Drum::Cymbal => shaper.tune(4_800.0, 0.7, rate),
        Drum::Snare => shaper.tune(1_600.0, 0.9, rate),
        _ => shaper.tune(400.0, 0.7, rate),
    }
    for index in 0..total {
        let Some(slot) = mix.get_mut(start + index) else {
            break;
        };
        let progress = index as f32 / total as f32;
        noise_state = noise_state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let white = (noise_state >> 8) as f32 / 8_388_608.0 - 1.0;
        let sample = match drum {
            Drum::Kick => {
                // 110 Hz falling to 48 Hz over the first third of the hit.
                let frequency = 48.0 + 62.0 * (-6.0 * progress).exp();
                phase += frequency / rate;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
                (phase * std::f32::consts::TAU).sin() + white * 0.15 * (-30.0 * progress).exp()
            }
            Drum::Snare => {
                // Two body tones a fifth apart under a band of noise: one tone
                // alone reads as a tom, not a snare.
                phase += 185.0 / rate;
                body_phase += 278.0 / rate;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
                if body_phase >= 1.0 {
                    body_phase -= 1.0;
                }
                let body = 0.6 * (phase * std::f32::consts::TAU).sin()
                    + 0.4 * (body_phase * std::f32::consts::TAU).sin();
                body * 0.35 + shaper.high_pass(white) * 0.65
            }
            Drum::Tom => {
                // A tom's pitch drops too, less steeply than a kick's.
                let frequency = 95.0 + 45.0 * (-8.0 * progress).exp();
                phase += frequency / rate;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
                (phase * std::f32::consts::TAU).sin() * 0.75 + white * 0.25
            }
            Drum::ClosedHat | Drum::OpenHat | Drum::Cymbal => shaper.high_pass(white) * 0.6,
        };
        let decay = match drum {
            Drum::Kick => (-5.0 * progress).exp(),
            Drum::Snare | Drum::Tom => (-7.0 * progress).exp(),
            Drum::ClosedHat => (-14.0 * progress).exp(),
            Drum::OpenHat | Drum::Cymbal => (-4.0 * progress).exp(),
        };
        *slot += sample * amplitude * decay * (0.35 + 0.65 * velocity);
    }
}

/// Where the soft knee begins. Below this, samples pass through untouched.
const KNEE: f32 = 0.6;

/// Normalizes to [`TARGET_RMS`], then rounds off whatever still pokes above
/// the knee.
///
/// Dense full-ensemble passages can produce a single high transient where
/// eighteen notes land together — six ensemble strings, three muted guitars,
/// two flutes, bass, two overdriven guitars and four drums. Scaling the whole
/// song down to fit that one 23-millisecond transient under the ceiling costs
/// six decibels everywhere else, which is how a track ends up both quiet and
/// lifeless. Instead the transient itself is rounded off: everything below the
/// knee is untouched, and above it a `tanh` curve approaches the ceiling
/// without ever reaching it. The curve is continuous and has unit slope at the
/// knee, so nothing steps.
fn normalize(mix: &mut [f32]) {
    let sum_squares: f64 = mix
        .iter()
        .map(|&sample| f64::from(sample) * f64::from(sample))
        .sum();
    if sum_squares <= 0.0 {
        return;
    }
    let rms = (sum_squares / mix.len() as f64).sqrt() as f32;
    let scale = TARGET_RMS / rms;
    let range = PEAK_CEILING - KNEE;
    for sample in mix.iter_mut() {
        let scaled = *sample * scale;
        let magnitude = scaled.abs();
        *sample = if magnitude <= KNEE {
            scaled
        } else {
            scaled.signum() * (KNEE + range * ((magnitude - KNEE) / range).tanh())
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::MidiNote;

    fn one_note_song(channel: u8, program: u8) -> MidiSong {
        MidiSong {
            format: 0,
            track_count: 1,
            ticks_per_quarter: 96,
            notes: vec![MidiNote {
                start_us: 0,
                duration_us: 250_000,
                channel,
                key: 69,
                velocity: 100,
                program,
            }],
            duration_us: 250_000,
        }
    }

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples
            .iter()
            .map(|&sample| f64::from(sample) * f64::from(sample))
            .sum::<f64>()
            / samples.len() as f64)
            .sqrt() as f32
    }

    /// Energy at one frequency, by correlation against a Hann-windowed probe.
    ///
    /// The window is not decoration: a rectangular correlation leaks a strong
    /// fundamental across the whole spectrum as `1/df`, which for these
    /// signals is larger than the harmonics being measured. Hann drops that
    /// to `1/df^3` and makes the numbers mean what they claim to.
    fn energy_at(signal: &[f32], probe: f32, rate: f32) -> f32 {
        let (mut real, mut imaginary) = (0.0f32, 0.0f32);
        let length = signal.len() as f32;
        for (index, &sample) in signal.iter().enumerate() {
            let window = 0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / length).cos();
            let angle = std::f32::consts::TAU * probe * index as f32 / rate;
            real += sample * window * angle.cos();
            imaginary += sample * window * angle.sin();
        }
        (real * real + imaginary * imaginary).sqrt() / length
    }

    #[test]
    fn rendering_is_deterministic_and_bounded() {
        let song = one_note_song(0, 0);
        let first = render(&song);
        let second = render(&song);
        assert_eq!(first, second);
        assert!(first.iter().any(|&sample| sample != 0.0));
        assert!(first
            .iter()
            .all(|&sample| sample.abs() <= PEAK_CEILING + f32::EPSILON));
        // 250 ms of note plus the fixed 500 ms tail the buffer always carries.
        let rate = SAMPLE_RATE as usize;
        assert!((rate * 70 / 100..=rate * 80 / 100).contains(&first.len()));
    }

    #[test]
    fn every_voice_family_produces_signal() {
        for program in [0u8, 26, 30, 35, 48, 73, 107] {
            let rendered = render(&one_note_song(0, program));
            assert!(
                rendered.iter().any(|&sample| sample.abs() > 0.01),
                "voice for program {program} is silent"
            );
        }
    }

    #[test]
    fn every_drum_voice_produces_signal() {
        for key in [36u8, 38, 42, 46, 49, 45] {
            let mut song = one_note_song(9, 0);
            song.notes[0].key = key;
            let rendered = render(&song);
            assert!(
                rendered.iter().any(|&sample| sample.abs() > 0.01),
                "drum key {key} is silent"
            );
        }
    }

    /// The kit's voices occupy the registers they should.
    ///
    /// The metal voices are noise through a fixed high-pass now rather than
    /// the difference of successive noise samples the renderer used before,
    /// which is a far steeper and thinner shape than any cymbal has. That
    /// change is only worth making if the result still lands where a hat
    /// lands, so this measures it: the kick and the toms carry their energy
    /// low, the hats and the cymbal carry theirs high, and the snare sits
    /// between them.
    #[test]
    fn the_drum_voices_sit_in_their_own_registers() {
        let rate = SAMPLE_RATE as f32;
        let brightness = |key: u8| -> f32 {
            let mut song = one_note_song(9, 0);
            song.notes[0].key = key;
            let rendered = render(&song);
            let window = &rendered[..(rate * 0.04) as usize];
            let low: f32 = [80.0f32, 160.0, 240.0]
                .iter()
                .map(|&probe| energy_at(window, probe, rate))
                .sum();
            let high: f32 = [6_000.0f32, 9_000.0, 12_000.0]
                .iter()
                .map(|&probe| energy_at(window, probe, rate))
                .sum();
            high / low.max(f32::EPSILON)
        };
        let kick = brightness(36);
        let tom = brightness(45);
        let snare = brightness(38);
        let closed_hat = brightness(42);
        let cymbal = brightness(49);
        assert!(
            kick < snare,
            "kick {kick:.4} is not darker than snare {snare:.4}"
        );
        assert!(
            tom < snare,
            "tom {tom:.4} is not darker than snare {snare:.4}"
        );
        assert!(
            snare < closed_hat,
            "snare {snare:.4} is not darker than the hat {closed_hat:.4}"
        );
        assert!(
            snare < cymbal,
            "snare {snare:.4} is not darker than the cymbal {cymbal:.4}"
        );
        // The kick is genuinely low, not merely relatively so.
        assert!(kick < 0.05, "the kick's high-band ratio is {kick:.4}");
    }

    #[test]
    fn an_empty_song_renders_leading_silence_only() {
        let song = MidiSong {
            format: 0,
            track_count: 0,
            ticks_per_quarter: 96,
            notes: Vec::new(),
            duration_us: 0,
        };
        let rendered = render(&song);
        assert!(rendered.iter().all(|&sample| sample == 0.0));
    }

    /// The spectral centroid of a sustained note: the mean harmonic number,
    /// weighted by energy. 1.0 is a pure sine; larger is brighter.
    fn centroid(program: u8) -> f32 {
        let rate = SAMPLE_RATE as f32;
        let fundamental = key_frequency(69); // A440
        let mut song = one_note_song(0, program);
        song.notes[0].duration_us = 1_000_000;
        song.duration_us = 1_000_000;
        let rendered = render(&song);
        // A window well inside the sustain, past every attack and filter
        // sweep.
        let window = &rendered[(rate * 0.3) as usize..(rate * 0.6) as usize];
        let mut weighted = 0.0f32;
        let mut total = 0.0f32;
        for harmonic in 1..=14u32 {
            let energy = energy_at(window, fundamental * harmonic as f32, rate);
            weighted += energy * harmonic as f32;
            total += energy;
        }
        weighted / total.max(f32::EPSILON)
    }

    /// The families are genuinely different instruments, not one timbre with
    /// different envelopes.
    ///
    /// This is the property the previous renderer could not have passed: it
    /// built every pitched voice from the same sawtooth-and-square pair, so a
    /// flute and a string ensemble carried the same harmonic content and
    /// differed only in how loudly and how long they held it. Here each family
    /// has its own harmonic recipe and its own filter, and the spectral
    /// centroid separates all seven.
    ///
    /// The ordering is also a claim about the arrangement, not just about
    /// distinctness: the flute has to be the purest voice in the score and the
    /// overdriven guitar the dirtiest, and an earlier draft of these constants
    /// had the overdrive *darker* than the strings it plays against. That was
    /// wrong, and this is what caught it.
    #[test]
    fn the_families_have_genuinely_different_spectra() {
        // (program, family) across every voice the renderer can select.
        let families = [
            (73u8, "flute"),
            (35, "bass"),
            (26, "muted guitar"),
            (107, "koto"),
            (0, "lead"),
            (48, "strings"),
            (30, "overdriven guitar"),
        ];
        let measured: Vec<(f32, &str)> = families
            .iter()
            .map(|&(program, name)| (centroid(program), name))
            .collect();

        // The flute is nearly a sine; the overdrive is the brightest thing in
        // the arrangement.
        let flute = measured[0].0;
        let overdriven = measured[6].0;
        assert!(flute < 1.5, "the flute's centroid is {flute:.2}");
        assert!(
            overdriven > 4.0,
            "the overdriven guitar's centroid is {overdriven:.2}"
        );
        assert!(
            measured.iter().all(|&(value, _)| value <= overdriven),
            "something is brighter than the overdriven guitar"
        );
        assert!(
            measured.iter().all(|&(value, _)| value >= flute),
            "something is purer than the flute"
        );

        // And every family is separated from every other one.
        let mut sorted = measured.clone();
        sorted.sort_by(|left, right| left.0.total_cmp(&right.0));
        for pair in sorted.windows(2) {
            let gap = pair[1].0 - pair[0].0;
            assert!(
                gap > 0.10,
                "{} ({:.2}) and {} ({:.2}) are the same instrument",
                pair[0].1,
                pair[0].0,
                pair[1].1,
                pair[1].0
            );
        }

        // The rendered voice really is reading its own family's table. The
        // lead's recipe has no even partials at all, so if `render_pitched`
        // selected any other table its second harmonic would appear
        // immediately. Without this the two checks either side of it —
        // rendered centroids and raw tables — could both pass while the
        // render path ignored the recipe entirely.
        let rate = SAMPLE_RATE as f32;
        let fundamental = key_frequency(69);
        let mut lead = one_note_song(0, 0);
        lead.notes[0].duration_us = 1_000_000;
        lead.duration_us = 1_000_000;
        let rendered = render(&lead);
        let window = &rendered[(rate * 0.3) as usize..(rate * 0.6) as usize];
        let even = energy_at(window, fundamental * 2.0, rate);
        let odd = energy_at(window, fundamental, rate);
        assert!(
            even < odd * 0.02,
            "the lead's second harmonic is {:.4} of its first; it is not \
             reading the odd-harmonic table",
            even / odd
        );

        // The separation above could in principle come from the filters and
        // envelopes alone, leaving every family sharing one waveform — which
        // is exactly what the previous renderer did. So compare the waveforms
        // themselves: each recipe's table, at unit RMS, must differ from
        // every other recipe's.
        let unit_rms = |recipe: Recipe| -> Vec<f32> {
            let table = &wave_sets()[recipe.index()].bands[0];
            let energy = (table.iter().map(|&s| s * s).sum::<f32>() / table.len() as f32).sqrt();
            table
                .iter()
                .map(|&s| s / energy.max(f32::EPSILON))
                .collect()
        };
        let tables: Vec<(Recipe, Vec<f32>)> =
            Recipe::ALL.iter().map(|&r| (r, unit_rms(r))).collect();
        let mut closest = (f32::MAX, String::new());
        for (index, (left_recipe, left)) in tables.iter().enumerate() {
            for (right_recipe, right) in tables.iter().skip(index + 1) {
                let difference = (left
                    .iter()
                    .zip(right.iter())
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum::<f32>()
                    / left.len() as f32)
                    .sqrt();
                if difference < closest.0 {
                    closest = (difference, format!("{left_recipe:?}/{right_recipe:?}"));
                }
                assert!(
                    difference > 0.15,
                    "{left_recipe:?} and {right_recipe:?} are the same waveform \
                     (difference {difference:.4})"
                );
            }
        }
        // The closest legitimate pair is the string ensemble against the
        // overdriven guitar, at 0.235 — both are dense sawtooth-family
        // timbres and the overdrive's character comes mostly from its drive
        // stage. The threshold above sits well below that and far above the
        // zero a shared waveform would produce.
        assert!(
            closest.0 > 0.2,
            "the closest pair {} fell to {:.4}",
            closest.1,
            closest.0
        );
    }

    /// The filter envelope moves: a note is brighter at its attack than in its
    /// tail.
    ///
    /// Without this the whole point of the filter is unverified, and a static
    /// cutoff would pass every other test in this file. The koto is the right
    /// voice to check, since its cutoff closes fastest and a plucked string
    /// that stays equally bright for a whole second is the most recognisable
    /// synthetic artefact there is.
    #[test]
    fn notes_grow_darker_as_they_decay() {
        let rate = SAMPLE_RATE as f32;
        let fundamental = key_frequency(57); // A3, so its partials fit easily
        let mut song = one_note_song(0, 107);
        song.notes[0].key = 57;
        song.notes[0].duration_us = 1_000_000;
        song.duration_us = 1_000_000;
        let rendered = render(&song);
        let brightness = |window: &[f32]| -> f32 {
            let first = energy_at(window, fundamental, rate);
            let upper: f32 = [6.0f32, 8.0, 10.0]
                .iter()
                .map(|&n| energy_at(window, fundamental * n, rate))
                .sum();
            upper / first.max(f32::EPSILON)
        };
        let attack = brightness(&rendered[(rate * 0.005) as usize..(rate * 0.045) as usize]);
        let tail = brightness(&rendered[(rate * 0.70) as usize..(rate * 0.78) as usize]);
        assert!(
            tail < attack * 0.6,
            "attack brightness {attack:.4} barely fell to {tail:.4}"
        );
    }

    /// The magnitude of harmonic `n` in a single-cycle table.
    fn table_harmonic(table: &[f32], harmonic: usize) -> f32 {
        let (mut real, mut imaginary) = (0.0f32, 0.0f32);
        let length = table.len() as f32;
        for (index, &sample) in table.iter().enumerate() {
            let angle = std::f32::consts::TAU * harmonic as f32 * index as f32 / length;
            real += sample * angle.cos();
            imaginary += sample * angle.sin();
        }
        2.0 * (real * real + imaginary * imaginary).sqrt() / length
    }

    /// Where a frequency above Nyquist lands once sampling folds it back.
    fn fold(frequency: f32, rate: f32) -> f32 {
        let wrapped = frequency % rate;
        if wrapped > rate / 2.0 {
            rate - wrapped
        } else {
            wrapped
        }
    }

    /// No table holds a partial that would sit above Nyquist for the notes it
    /// serves.
    ///
    /// This is the property at its source, checked directly on every table of
    /// every recipe rather than inferred from how one of them sounds. The
    /// previous renderer could not have this property: PolyBLEP corrects a
    /// sawtooth's own discontinuity but cannot band-limit an arbitrary
    /// harmonic recipe, so a rich timbre played high still aliased.
    #[test]
    fn no_wavetable_carries_a_partial_above_nyquist() {
        let nyquist = SAMPLE_RATE as f32 / 2.0;
        for recipe in Recipe::ALL {
            let set = &wave_sets()[recipe.index()];
            for band in 0..BANDS {
                let top_key = (band * 12 + 11).min(127) as u8;
                let limit =
                    ((nyquist / key_frequency(top_key)).floor() as usize).clamp(1, MAX_PARTIALS);
                let table = &set.bands[band];
                for harmonic in (limit + 1)..=MAX_PARTIALS {
                    let magnitude = table_harmonic(table, harmonic);
                    assert!(
                        magnitude < 1e-4,
                        "{recipe:?} band {band} carries partial {harmonic} at \
                         {magnitude:.6}, above the {limit}-partial limit"
                    );
                }
            }
        }
    }

    /// Played back, that leaves nothing at the frequencies aliases would
    /// occupy.
    ///
    /// The probes are computed rather than chosen: for a high note, every
    /// partial the table would have carried above Nyquist is folded back to
    /// where sampling puts it, and those are the frequencies checked. Picking
    /// round numbers instead would miss them — the images of a 2793 Hz
    /// fundamental land at 588, 1176 and 1764 Hz, none of which is an obvious
    /// place to look.
    #[test]
    fn playback_puts_no_energy_where_aliases_would_land() {
        let rate = SAMPLE_RATE as f32;
        let waves = &wave_sets()[Recipe::Strings.index()];
        // MIDI key 102 is 2793 Hz; only seven partials fit under Nyquist.
        let key = 102u8;
        let frequency = key_frequency(key);
        let step = frequency / rate;
        let samples = 16_384;
        let mut table = Vec::with_capacity(samples);
        let mut naive = Vec::with_capacity(samples);
        let mut phase = 0.0f32;
        for _ in 0..samples {
            table.push(waves.sample(key, phase));
            naive.push(2.0 * phase - 1.0);
            phase += step;
            if phase >= 1.0 {
                phase -= 1.0;
            }
        }
        let limit = (rate / 2.0 / frequency).floor() as usize;
        let probes: Vec<f32> = ((limit + 1)..=MAX_PARTIALS)
            .map(|harmonic| fold(frequency * harmonic as f32, rate))
            // Skip anything that folds onto a real harmonic, where legitimate
            // energy would be mistaken for an alias.
            .filter(|&probe| (probe / frequency).fract() > 0.05)
            .collect();
        assert!(probes.len() > 10, "only {} alias probes", probes.len());
        let alias_energy = |signal: &[f32]| -> f32 {
            probes
                .iter()
                .map(|&probe| energy_at(signal, probe, rate))
                .sum()
        };
        let table_alias = alias_energy(&table);
        let naive_alias = alias_energy(&naive);
        assert!(
            table_alias < naive_alias * 0.1,
            "the table aliased {table_alias:.6} against the naive {naive_alias:.6}"
        );
    }

    /// Every render lands on the same loudness, whatever the song's own
    /// levels: the museum plays 21 different MIDI payloads and they should not
    /// each need their own volume knob.
    #[test]
    fn renders_are_normalized_to_the_target_loudness() {
        let mut quiet = one_note_song(0, 48);
        quiet.notes[0].velocity = 20;
        let mut loud = one_note_song(0, 48);
        loud.notes[0].velocity = 127;
        let quiet_rms = rms(&render(&quiet));
        let loud_rms = rms(&render(&loud));
        assert!((quiet_rms - TARGET_RMS).abs() < 0.001, "quiet: {quiet_rms}");
        assert!((loud_rms - TARGET_RMS).abs() < 0.001, "loud: {loud_rms}");
    }
}
