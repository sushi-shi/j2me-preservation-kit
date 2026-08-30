//! Safe per-game host data paths.

use std::path::{Path, PathBuf};

use crate::PlatformError;

fn validate_slug(slug: &str) -> Result<(), PlatformError> {
    if slug.is_empty()
        || slug == "."
        || slug == ".."
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(PlatformError::Config(format!(
            "unsafe slug {slug:?}; expected ASCII letters, digits, '-' or '_'"
        )));
    }
    Ok(())
}

/// Read the top-level `slug` from a `game.toml` document.
pub fn slug_from_game_toml(document: &str) -> Result<String, PlatformError> {
    let value: toml::Value = document
        .parse()
        .map_err(|error| PlatformError::Config(format!("invalid game.toml: {error}")))?;
    let slug = value
        .get("slug")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| PlatformError::Config("game.toml has no string slug".to_owned()))?;
    validate_slug(slug)?;
    Ok(slug.to_owned())
}

/// Join the kit namespace and a validated game slug below a known data root.
/// This deterministic form is also the test seam for platform-specific roots.
pub fn application_data_dir_in(base: &Path, slug: &str) -> Result<PathBuf, PlatformError> {
    validate_slug(slug)?;
    Ok(base.join("j2me").join(slug))
}

/// Resolve the operating system's user-data root and append `j2me/<slug>`.
pub fn application_data_dir(slug: &str) -> Result<PathBuf, PlatformError> {
    validate_slug(slug)?;

    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());

    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|home| home.join("Library").join("Application Support"));

    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local").join("share"))
        });

    #[cfg(not(any(unix, target_os = "windows")))]
    let base: Option<PathBuf> = None;

    let base = base.ok_or_else(|| {
        PlatformError::Config("the operating system has no usable user-data directory".to_owned())
    })?;
    application_data_dir_in(&base, slug)
}

/// Parse `game.toml` and resolve its slug beneath the operating-system data root.
pub fn application_data_dir_from_game_toml(document: &str) -> Result<PathBuf, PlatformError> {
    application_data_dir(&slug_from_game_toml(document)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_toml_slug_owns_the_per_game_directory() {
        let slug = slug_from_game_toml("slug = \"ink-world\"\ntitle = \"Ink World\"\n").unwrap();
        assert_eq!(slug, "ink-world");
        assert_eq!(
            application_data_dir_in(Path::new("/data"), &slug).unwrap(),
            Path::new("/data/j2me/ink-world")
        );
    }

    #[test]
    fn path_separators_and_parent_components_are_rejected() {
        for slug in ["", ".", "..", "../other", "game/name", "game\\name"] {
            assert!(application_data_dir_in(Path::new("/data"), slug).is_err());
        }
    }
}
