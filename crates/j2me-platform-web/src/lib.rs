//! Browser-only host adapters. Web dependencies are introduced only by this
//! crate and never by the Java ME semantic runtime.

pub mod audio;
pub mod input;

pub use audio::{vibrate, BrowserAudio};
