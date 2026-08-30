//! `j2me-me` — the reusable Java ME / MIDP **2D** device runtime for strict J2ME
//! transliterations, as idiomatic Rust whose *observable behavior* matches the
//! Java ME contract. **2D only** (no M3G).
//!
//! The device-runtime surface a strict 2D port draws on:
//!
//! - [`graphics`] — `setColor`/clip/`translate`/rectangles/lines/triangles/
//!   `drawImage`/`drawRegion` (with `GraphicsError`/`SpriteTransform`) plus
//!   `drawArc` / `fillArc` (the MIDP ellipse-sector rasteriser);
//! - [`canvas`] — the `Canvas`/`Display` serial paint-input queue, the
//!   [`Displayable`] surface, ordered subclass `showNotify`/`hideNotify`
//!   dispatch, host-visible `Display.vibrate` requests, and the
//!   `Canvas.getGameAction` resolver through a supplied device-profile table;
//! - [`media`] — profile-gated MMAPI players, `VolumeControl`, listener events,
//!   mute, seeking, duration, looping, and host operations;
//! - [`midlet`] — the application-manager lifecycle (`startApp`/`pauseApp`/
//!   `destroyApp`, `notifyPaused`/`resumeRequest`/`notifyDestroyed`);
//! - [`rms`] — quota-aware monotonic-id `RecordStore` including add/set/delete,
//!   listing, open handles, snapshots, and Java-checked byte ranges;
//! - [`font`] — a profile-selected reviewed metrics/glyph provider seam;
//! - [`image`] — the byte-array/stream/named-resource PNG factories and
//!   `Image.createRGBImage`.
//!
//! The ARGB pixel buffer itself lives in the neutral [`j2me_canvas`] crate;
//! `j2me-me` surfaces it as `javax.microedition.lcdui.Image`
//! (`Image::create_mutable` is the MIDP `createImage(int, int)` factory).

pub mod canvas;
pub mod font;
pub mod graphics;
pub mod image;
pub mod media;
pub mod midlet;
pub mod rms;

pub use canvas::{
    get_game_action, get_game_action_profile, Canvas, CanvasEvent, Command, CommandId,
    DeviceGameActionTable, Display, Displayable, HostDisplayOp,
};
pub use font::{FontMetrics, FontProvider, FontRuntime, FontSpec, GlyphBitmap};
pub use graphics::{Graphics, GraphicsError, SpriteTransform};
pub use image::{
    create_image_named, create_image_region, create_image_stream, create_rgb_image, ImageResources,
};
pub use j2me_canvas::{source_over, Argb, Image, ImageError};
pub use media::{HostAudioOp, MediaRuntime, MediaSource, PlayerEvent, PlayerId, PlayerState};
pub use midlet::{HostMidletOp, MidletCallback, MidletLifecycle, MidletState};
pub use rms::{RecordStore, RecordStoreSnapshot, RmsRuntime, RmsSnapshot};
