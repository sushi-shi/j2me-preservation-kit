//! Host adapters shared by resource-free Java ME ports.
//!
//! Device semantics live in `j2me-me`. This crate supplies the deliberately
//! boring host edges around them: JAR/JAD resources, filesystem RMS snapshots,
//! per-game application-data paths, pixel projection, and policy-driven focus
//! action ordering. It has no output API or game-specific constants.

mod lifecycle;
mod paths;
mod presentation;
mod resources;
mod rms;
mod services;
mod system;

pub use paths::{
    application_data_dir, application_data_dir_from_game_toml, application_data_dir_in,
    slug_from_game_toml,
};
pub use presentation::{argb_to_rgba_cropped, CanvasPlacement, PresentationError, Rect};
pub use resources::{
    parse_application_properties, resolve_class_resource_name, resolve_class_resource_name_utf16,
    JarResources, MemoryResources, ResourceSource,
};
pub use rms::{decode_rms_snapshot, encode_rms_snapshot, PersistentRms};

/// Failure at a host boundary. Device-facing Java exceptions remain owned by
/// `j2me-jvm`/`j2me-me`; these errors describe archive, configuration, or host
/// filesystem failures before a Java callback is entered.
#[derive(Debug)]
pub enum PlatformError {
    Io(std::io::Error),
    Archive(zip::result::ZipError),
    Resource(String),
    Config(String),
    CorruptRms(String),
    Service(String),
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "host I/O error: {error}"),
            Self::Archive(error) => write!(formatter, "JAR error: {error}"),
            Self::Resource(error) => write!(formatter, "JAR resource error: {error}"),
            Self::Config(error) => write!(formatter, "game configuration error: {error}"),
            Self::CorruptRms(error) => write!(formatter, "corrupt RMS snapshot: {error}"),
            Self::Service(error) => write!(formatter, "external service error: {error}"),
        }
    }
}

impl std::error::Error for PlatformError {}

impl From<std::io::Error> for PlatformError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<zip::result::ZipError> for PlatformError {
    fn from(error: zip::result::ZipError) -> Self {
        Self::Archive(error)
    }
}
pub use lifecycle::{focus_actions, FocusLifecycle, LifecycleAction};
pub use services::{
    DisabledServices, HttpRequest, HttpResponse, ServiceBackend, ServiceRuntime, SmsRequest,
};
pub use system::{SystemEnvironment, SystemOverrides};
