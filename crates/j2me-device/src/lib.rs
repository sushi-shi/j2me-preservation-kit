//! Composable, game-owned handset capability profiles.
//!
//! This crate describes the emulated phone. It contains no desktop keyboard,
//! window, sound-card, browser, or game behavior. A host first maps physical
//! input to [`HandsetKey`], then the selected [`DeviceProfile`] maps that key
//! to the raw integer delivered to `Canvas.keyPressed`.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandsetKey {
    Up,
    Down,
    Left,
    Right,
    Fire,
    SoftLeft,
    SoftRight,
    Digit(u8),
    Star,
    Pound,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayFragment {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub fullscreen_width: Option<u32>,
    #[serde(default)]
    pub fullscreen_height: Option<u32>,
}

impl DisplayFragment {
    pub fn canvas_size(&self, fullscreen: bool) -> (u32, u32) {
        if fullscreen {
            (
                self.fullscreen_width.unwrap_or(self.width),
                self.fullscreen_height.unwrap_or(self.height),
            )
        } else {
            (self.width, self.height)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputFragment {
    pub up: i32,
    pub down: i32,
    pub left: i32,
    pub right: i32,
    pub fire: i32,
    pub soft_left: i32,
    pub soft_right: i32,
    pub star: i32,
    pub pound: i32,
    pub digits: [i32; 10],
    #[serde(default)]
    pub game_action_up: Vec<i32>,
    #[serde(default)]
    pub game_action_down: Vec<i32>,
    #[serde(default)]
    pub game_action_left: Vec<i32>,
    #[serde(default)]
    pub game_action_right: Vec<i32>,
    #[serde(default)]
    pub game_action_fire: Vec<i32>,
    /// Whether the handset delivers `Canvas.pointerPressed` / `pointerReleased`.
    #[serde(default)]
    pub pointer_events: bool,
    /// Whether the handset additionally delivers `Canvas.pointerDragged`.
    #[serde(default)]
    pub pointer_motion_events: bool,
    /// Whether the handset delivers `Canvas.keyRepeated` callbacks.
    #[serde(default)]
    pub repeat_events: bool,
}

impl InputFragment {
    pub fn key_code(&self, key: HandsetKey) -> Option<i32> {
        Some(match key {
            HandsetKey::Up => self.up,
            HandsetKey::Down => self.down,
            HandsetKey::Left => self.left,
            HandsetKey::Right => self.right,
            HandsetKey::Fire => self.fire,
            HandsetKey::SoftLeft => self.soft_left,
            HandsetKey::SoftRight => self.soft_right,
            HandsetKey::Digit(digit) => *self.digits.get(usize::from(digit))?,
            HandsetKey::Star => self.star,
            HandsetKey::Pound => self.pound,
        })
    }

    /// MIDP `Canvas.getGameAction`, using the reviewed table for this handset.
    pub fn game_action(&self, raw_code: i32) -> i32 {
        if self.game_action_up.contains(&raw_code) {
            1
        } else if self.game_action_left.contains(&raw_code) {
            2
        } else if self.game_action_right.contains(&raw_code) {
            5
        } else if self.game_action_down.contains(&raw_code) {
            6
        } else if self.game_action_fire.contains(&raw_code) {
            8
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaFragment {
    #[serde(default)]
    pub content_types: BTreeSet<String>,
    #[serde(default)]
    pub mime_aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub controls: BTreeSet<String>,
    /// `device`, `approximate`, or `none`; a descriptive capability, not a
    /// request for the host to pretend an approximation is device-faithful.
    pub midi_renderer: String,
}

impl MediaFragment {
    pub fn canonical_mime<'a>(&'a self, mime: &'a str) -> &'a str {
        self.mime_aliases.get(mime).map_or(mime, String::as_str)
    }

    pub fn supports_mime(&self, mime: &str) -> bool {
        self.content_types.contains(self.canonical_mime(mime))
    }

    pub fn has_control(&self, control: &str) -> bool {
        self.controls.contains(control)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HapticsFragment {
    pub vibration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RmsFragment {
    #[serde(default)]
    pub capacity_bytes: Option<u64>,
}

/// Protocol families the handset exposes through the Generic Connection
/// Framework. This records device capability only; a desktop/browser host must
/// still provide an explicit, security-reviewed backend before any external
/// request can leave the process.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorFragment {
    #[serde(default)]
    pub schemes: BTreeSet<String>,
}

impl ConnectorFragment {
    pub fn supports(&self, scheme: &str) -> bool {
        self.schemes.contains(scheme)
    }
}

/// Handset/runtime defaults exposed through `java.lang.System` and APIs that
/// use the platform default character encoding.
///
/// These are device facts, not host configuration. A host may layer explicit
/// session or operator overrides on top, but must not rewrite the reviewed
/// profile in order to do so.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemFragment {
    /// Java charset name used by APIs such as `String.getBytes()` when no
    /// encoding argument is supplied. The profile records the name only; the
    /// JVM string codec remains responsible for implementing that encoding.
    pub default_charset: String,
    /// Defaults returned by `System.getProperty`. Missing keys remain absent
    /// rather than being synthesized from the desktop/browser host.
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

impl SystemFragment {
    pub fn property(&self, name: &str) -> Option<&str> {
        self.properties.get(name).map(String::as_str)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontFragment {
    /// Identifier of game-owned metrics/raster evidence. The portable runtime
    /// never silently substitutes one vendor's system font for another.
    pub provider: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FocusLossPolicy {
    None,
    Hide,
    PauseThenHide,
    HideThenPause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FocusGainPolicy {
    None,
    Show,
    StartThenShow,
    ShowThenStart,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleFragment {
    pub focus_loss: FocusLossPolicy,
    pub focus_gain: FocusGainPolicy,
    #[serde(default)]
    pub platform_request: bool,
    #[serde(default)]
    pub platform_request_requires_exit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileRefs {
    id: String,
    display: String,
    input: String,
    media: String,
    haptics: String,
    rms: String,
    connector: String,
    system: String,
    font: String,
    lifecycle: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceProfile {
    pub id: String,
    pub display: DisplayFragment,
    pub input: InputFragment,
    pub media: MediaFragment,
    pub haptics: HapticsFragment,
    pub rms: RmsFragment,
    pub connector: ConnectorFragment,
    pub system: SystemFragment,
    pub font: FontFragment,
    pub lifecycle: LifecycleFragment,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceCatalog {
    pub schema_version: u32,
    #[serde(default)]
    display: BTreeMap<String, DisplayFragment>,
    #[serde(default)]
    input: BTreeMap<String, InputFragment>,
    #[serde(default)]
    media: BTreeMap<String, MediaFragment>,
    #[serde(default)]
    haptics: BTreeMap<String, HapticsFragment>,
    #[serde(default)]
    rms: BTreeMap<String, RmsFragment>,
    #[serde(default)]
    connector: BTreeMap<String, ConnectorFragment>,
    #[serde(default)]
    system: BTreeMap<String, SystemFragment>,
    #[serde(default)]
    font: BTreeMap<String, FontFragment>,
    #[serde(default)]
    lifecycle: BTreeMap<String, LifecycleFragment>,
    #[serde(default, rename = "profile")]
    profiles: Vec<ProfileRefs>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileError(pub String);

impl std::fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ProfileError {}

impl DeviceCatalog {
    pub fn parse(source: &str) -> Result<Self, ProfileError> {
        let catalog: Self = toml::from_str(source)
            .map_err(|error| ProfileError(format!("invalid device profile catalog: {error}")))?;
        catalog.validate()?;
        Ok(catalog)
    }

    fn validate(&self) -> Result<(), ProfileError> {
        if self.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(ProfileError(format!(
                "unsupported device profile schema {}, expected {PROFILE_SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        let mut ids = BTreeSet::new();
        for profile in &self.profiles {
            if profile.id.trim().is_empty() || !ids.insert(profile.id.as_str()) {
                return Err(ProfileError(format!(
                    "device profile id {:?} is empty or duplicated",
                    profile.id
                )));
            }
            self.resolve_refs(profile)?;
        }
        for (id, display) in &self.display {
            if display.width == 0
                || display.height == 0
                || display.width > i32::MAX as u32
                || display.height > i32::MAX as u32
                || display
                    .fullscreen_width
                    .is_some_and(|width| width == 0 || width > i32::MAX as u32)
                || display
                    .fullscreen_height
                    .is_some_and(|height| height == 0 || height > i32::MAX as u32)
            {
                return Err(ProfileError(format!(
                    "display fragment {id:?} has an invalid Java canvas dimension"
                )));
            }
        }
        for (id, media) in &self.media {
            if !matches!(
                media.midi_renderer.as_str(),
                "device" | "approximate" | "none"
            ) {
                return Err(ProfileError(format!(
                    "media fragment {id:?} has unknown midi_renderer {:?}",
                    media.midi_renderer
                )));
            }
        }
        for (id, connector) in &self.connector {
            if let Some(scheme) = connector.schemes.iter().find(|scheme| {
                scheme.is_empty()
                    || !scheme.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || byte == b'+'
                            || byte == b'-'
                            || byte == b'.'
                    })
            }) {
                return Err(ProfileError(format!(
                    "connector fragment {id:?} has invalid lowercase URI scheme {scheme:?}"
                )));
            }
        }
        for (id, system) in &self.system {
            if system.default_charset.trim().is_empty() {
                return Err(ProfileError(format!(
                    "system fragment {id:?} has an empty default_charset"
                )));
            }
            if system
                .properties
                .keys()
                .any(|property| property.trim().is_empty())
            {
                return Err(ProfileError(format!(
                    "system fragment {id:?} has an empty property name"
                )));
            }
        }
        Ok(())
    }

    pub fn profile_ids(&self) -> impl Iterator<Item = &str> {
        self.profiles.iter().map(|profile| profile.id.as_str())
    }

    pub fn resolve(&self, id: &str) -> Result<DeviceProfile, ProfileError> {
        let refs = self
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .ok_or_else(|| ProfileError(format!("unknown device profile {id:?}")))?;
        self.resolve_refs(refs)
    }

    fn resolve_refs(&self, refs: &ProfileRefs) -> Result<DeviceProfile, ProfileError> {
        macro_rules! fragment {
            ($map:ident, $field:ident) => {
                self.$map.get(&refs.$field).cloned().ok_or_else(|| {
                    ProfileError(format!(
                        "device profile {:?} references missing {} fragment {:?}",
                        refs.id,
                        stringify!($field),
                        refs.$field
                    ))
                })?
            };
        }
        Ok(DeviceProfile {
            id: refs.id.clone(),
            display: fragment!(display, display),
            input: fragment!(input, input),
            media: fragment!(media, media),
            haptics: fragment!(haptics, haptics),
            rms: fragment!(rms, rms),
            connector: fragment!(connector, connector),
            system: fragment!(system, system),
            font: fragment!(font, font),
            lifecycle: fragment!(lifecycle, lifecycle),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = r#"
schema_version = 1

[display.small]
width = 128
height = 160

[display.large]
width = 240
height = 320

[input.keys]
up = -1
down = -2
left = -3
right = -4
fire = -5
soft_left = -6
soft_right = -7
star = 42
pound = 35
digits = [48,49,50,51,52,53,54,55,56,57]
game_action_up = [-1, 50]
game_action_down = [-2, 56]
game_action_left = [-3, 52]
game_action_right = [-4, 54]
game_action_fire = [-5, 53]

[media.midi]
content_types = ["audio/midi"]
controls = ["VolumeControl"]
midi_renderer = "device"

[haptics.yes]
vibration = true

[rms.small]
capacity_bytes = 32768

[connector.online]
schemes = ["http", "https", "sms"]

[system.fixture]
default_charset = "ISO-8859-1"
properties = { "microedition.platform" = "FixturePhone", "wireless.messaging.sms.smsc" = "+123" }

[font.oracle]
provider = "fixture-font-v1"

[lifecycle.standard]
focus_loss = "pause-then-hide"
focus_gain = "show-then-start"

[[profile]]
id = "small-midi"
display = "small"
input = "keys"
media = "midi"
haptics = "yes"
rms = "small"
connector = "online"
system = "fixture"
font = "oracle"
lifecycle = "standard"

[[profile]]
id = "large-midi"
display = "large"
input = "keys"
media = "midi"
haptics = "yes"
rms = "small"
connector = "online"
system = "fixture"
font = "oracle"
lifecycle = "standard"
"#;

    #[test]
    fn profiles_compose_independent_fragments() {
        let catalog = DeviceCatalog::parse(CATALOG).unwrap();
        let small = catalog.resolve("small-midi").unwrap();
        let large = catalog.resolve("large-midi").unwrap();
        assert_eq!(small.display.canvas_size(false), (128, 160));
        assert_eq!(large.display.canvas_size(false), (240, 320));
        assert_eq!(small.input.key_code(HandsetKey::Digit(5)), Some(53));
        assert_eq!(small.input.game_action(50), 1);
        assert!(small.media.supports_mime("audio/midi"));
        assert!(small.media.has_control("VolumeControl"));
        assert!(small.connector.supports("http"));
        assert!(small.connector.supports("sms"));
        assert_eq!(small.system.default_charset, "ISO-8859-1");
        assert_eq!(
            small.system.property("wireless.messaging.sms.smsc"),
            Some("+123")
        );
        assert_eq!(small.media, large.media);
    }

    #[test]
    fn missing_fragments_and_unknown_schema_fail() {
        assert!(DeviceCatalog::parse("schema_version = 2").is_err());
        assert!(DeviceCatalog::parse(
            "schema_version = 1\n[[profile]]\nid='x'\ndisplay='missing'\ninput='x'\nmedia='x'\nhaptics='x'\nrms='x'\nconnector='x'\nsystem='x'\nfont='x'\nlifecycle='x'\n"
        )
        .is_err());
    }

    #[test]
    fn invalid_system_defaults_fail() {
        let empty_charset = CATALOG.replace(
            "default_charset = \"ISO-8859-1\"",
            "default_charset = \"  \"",
        );
        assert!(DeviceCatalog::parse(&empty_charset).is_err());

        let empty_property = CATALOG.replace(
            "properties = { \"microedition.platform\"",
            "properties = { \"\" = \"bad\", \"microedition.platform\"",
        );
        assert!(DeviceCatalog::parse(&empty_property).is_err());
    }
}
