//! `j2me-me` — the reusable Java ME / MIDP **2D** device runtime for strict J2ME
//! transliterations, as idiomatic Rust whose *observable behavior* matches the
//! Java ME contract. **2D only** (no M3G).
//!
//! The device-runtime surface a strict 2D port draws on:
//!
//! - [`graphics`] — `setColor`/clip/`translate`/`fillRect`/`drawRect`/`drawLine`/
//!   `drawImage`/`drawRegion` (with `GraphicsError`/`SpriteTransform`) plus
//!   `drawArc` / `fillArc` (the MIDP ellipse-sector rasteriser);
//! - [`canvas`] — the `Canvas`/`Display` serial paint-input queue, the
//!   [`Displayable`] surface, ordered subclass `showNotify`/`hideNotify`
//!   dispatch, host-visible `Display.vibrate` requests, and the
//!   `Canvas.getGameAction` resolver with its MIDP-default and Nokia device
//!   key-to-action tables;
//! - [`media`] — the MMAPI player model with `VolumeControl`
//!   (`getControl`/`setLevel`/`getLevel`), a `PlayerListener` registration, and
//!   the `getState()` MMAPI integers;
//! - [`rms`] — the monotonic-record-id `RecordStore` (`getNextRecordID`,
//!   `getRecordSize`, offset/length-checked `addRecord`);
//! - [`image`] — the `Image.createImage(byte[])` / `createImage(String)`
//!   PNG-decode factories.
//!
//! The ARGB pixel buffer itself lives in the neutral [`j2me_canvas`] crate;
//! `j2me-me` surfaces it as `javax.microedition.lcdui.Image`
//! (`Image::create_mutable` is the MIDP `createImage(int, int)` factory).

pub mod canvas;
pub mod graphics;
pub mod image;
pub mod media;
pub mod rms;

pub use canvas::{
    get_game_action, midp_default_game_action, nokia_game_action, Canvas, CanvasEvent,
    DeviceGameActionTable, Display, Displayable, HostDisplayOp,
};
pub use graphics::{Graphics, GraphicsError, SpriteTransform};
pub use image::{create_image_named, create_image_region, ImageResources};
pub use j2me_canvas::{source_over, Argb, Image, ImageError};
pub use media::{HostAudioOp, MediaRuntime, PlayerId, PlayerState};
pub use rms::{RecordStore, RmsRuntime};
