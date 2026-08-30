//! Native-only host adapters. Implementations are kept out of `j2me-me` and
//! `j2me-platform` so platform dependencies remain visible in Cargo metadata.

pub mod audio;
pub mod evdev;
pub mod haptics;
pub mod input;
pub mod lifecycle;

pub use audio::{NativeAudioOutput, SoftwareMixer};
pub use evdev::EvdevVibrator;
pub use input::NativeInput;
