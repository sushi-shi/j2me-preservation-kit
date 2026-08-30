//! A tiny, dependency-free reader for the optional `[keymap]` override table.
//!
//! The format is a flat subset of TOML — exactly what a `[keymap]` table in
//! `game.toml`, or a standalone `keymap.toml`, needs — so no `serde`/`toml`
//! dependency is pulled into every native host just to remap a key:
//!
//! ```toml
//! [keymap]
//! preset = "mobile"       # "mobile" (default) or "standard"
//!
//! # key-name = binding, where a binding is an action name, a raw Nokia code,
//! # or "none"/"unbind" to clear a preset binding. Values containing `#` or `*`
//! # must be quoted (an unquoted `#` starts a comment).
//! KeyH   = "SoftLeft"     # bind H to the left soft key by action name
//! KeyJ   = -5             # ...or by raw Nokia code (Fire)
//! KeyQ   = "none"         # unbind Q (it is Fire's neighbour in Mobile)
//! Comma  = "*"            # a symbol action, quoted
//! ```
//!
//! When the text contains no `[section]` header at all, the whole document is
//! treated as the keymap table (the standalone `keymap.toml` case). Otherwise
//! only lines inside `[keymap]` are read and every other section is ignored, so
//! the entire `game.toml` can be handed in as-is.

use crate::keymap::KeyBinding;
use crate::nokia::Action;
use crate::preset::{key_from_name, Preset};
use core::fmt;
use winit::keyboard::KeyCode;

/// A parsed `[keymap]` table: an optional preset selection plus the explicit
/// per-key overrides, in file order. A binding of `None` is an explicit unbind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeymapConfig {
    /// The `preset = "..."` selection, if the table set one.
    pub preset: Option<Preset>,
    /// `(physical key, binding)` overrides; `None` clears the preset binding.
    pub bindings: Vec<(KeyCode, Option<KeyBinding>)>,
}

/// A problem found while parsing a keymap config, with the 1-based line number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    /// 1-based line number the problem was found on.
    pub line: usize,
    /// Human-readable description of the problem.
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "keymap config, line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Parse a `[keymap]` table (or a header-less keymap document) into a
/// [`KeymapConfig`]. See the module docs for the accepted format.
pub fn parse(config: &str) -> Result<KeymapConfig, ConfigError> {
    let has_headers = config
        .lines()
        .any(|l| section_header(strip_comment(l).trim()).is_some());
    let mut in_keymap = !has_headers;

    let mut out = KeymapConfig::default();
    for (idx, raw) in config.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(section) = section_header(line) {
            in_keymap = section.eq_ignore_ascii_case("keymap");
            continue;
        }
        if !in_keymap {
            continue;
        }

        let (lhs, rhs) = split_kv(line).ok_or_else(|| ConfigError {
            line: line_no,
            message: format!("expected `key = value`, found `{line}`"),
        })?;

        if lhs.eq_ignore_ascii_case("preset") {
            let name = unquote(rhs);
            let preset = Preset::from_name(name).ok_or_else(|| ConfigError {
                line: line_no,
                message: format!("unknown preset `{name}` (expected `mobile` or `standard`)"),
            })?;
            out.preset = Some(preset);
            continue;
        }

        let key = key_from_name(lhs).ok_or_else(|| ConfigError {
            line: line_no,
            message: format!("unknown key name `{lhs}`"),
        })?;
        let binding = parse_binding(rhs, line_no)?;
        out.bindings.push((key, binding));
    }
    Ok(out)
}

/// Resolve the right-hand side of a `key = value` line to a binding:
/// an action name or a Nokia code -> `Some(code)`; `none`/`unbind`/`off` ->
/// `None` (explicit unbind).
fn parse_binding(rhs: &str, line_no: usize) -> Result<Option<KeyBinding>, ConfigError> {
    let value = unquote(rhs);
    let folded = value.trim().to_ascii_lowercase();
    if matches!(folded.as_str(), "none" | "unbind" | "off" | "unmapped") {
        return Ok(None);
    }

    // Raw device codes are an explicit escape hatch for a reviewed phone
    // profile. Named bindings remain portable across profiles.
    if let Ok(code) = value.trim().parse::<i32>() {
        return Ok(Some(KeyBinding::Raw(code)));
    }

    // Otherwise a named action.
    match Action::from_name(value) {
        Some(action) => Ok(Some(KeyBinding::Handset(action.handset_key()))),
        None => Err(ConfigError {
            line: line_no,
            message: format!("`{value}` is not an action name, a Nokia code, or `none`"),
        }),
    }
}

/// `[name]` -> `Some("name")`; anything else -> `None`.
fn section_header(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    // Tolerate `[[array]]` headers by peeling the extra bracket; the inner name
    // (e.g. `array`) simply won't match `keymap`.
    Some(inner.trim().trim_matches(|c| c == '[' || c == ']').trim())
}

/// Return the slice of `line` before the first unquoted `#` (a TOML comment).
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '#' => return &line[..i],
                _ => {}
            },
        }
    }
    line
}

/// Split on the first `=`, trimming both sides; `None` if there is no `=` or the
/// key is empty.
fn split_kv(line: &str) -> Option<(&str, &str)> {
    let (lhs, rhs) = line.split_once('=')?;
    let lhs = lhs.trim();
    if lhs.is_empty() {
        return None;
    }
    Some((lhs, rhs.trim()))
}

/// Strip one layer of matching `"` or `'` quotes, if present.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'') && first == last {
            return &s[1..s.len() - 1];
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode as K;

    #[test]
    fn empty_config_is_empty() {
        let cfg = parse("").unwrap();
        assert_eq!(cfg, KeymapConfig::default());
    }

    #[test]
    fn headerless_document_is_all_keymap() {
        let cfg = parse("preset = \"standard\"\nKeyH = SoftLeft\n").unwrap();
        assert_eq!(cfg.preset, Some(Preset::Standard));
        assert_eq!(
            cfg.bindings,
            vec![(
                K::KeyH,
                Some(KeyBinding::Handset(j2me_device::HandsetKey::SoftLeft))
            )]
        );
    }

    #[test]
    fn only_the_keymap_section_of_a_game_toml_is_read() {
        let text = "\
slug = \"demo\"

[oracle]
canvas_w = 240

[keymap]
preset = \"mobile\"
KeyJ = -5        # Fire by raw code

[other]
KeyZ = 999       # ignored: not in [keymap]
";
        let cfg = parse(text).unwrap();
        assert_eq!(cfg.preset, Some(Preset::Mobile));
        assert_eq!(cfg.bindings, vec![(K::KeyJ, Some(KeyBinding::Raw(-5)))]);
    }

    #[test]
    fn none_unbinds_and_quoted_symbol_survives_the_comment_scanner() {
        let cfg = parse("[keymap]\nKeyQ = \"none\"\nComma = \"#\"\n").unwrap();
        assert_eq!(
            cfg.bindings,
            vec![
                (K::KeyQ, None),
                (
                    K::Comma,
                    Some(KeyBinding::Handset(j2me_device::HandsetKey::Pound))
                )
            ]
        );
    }

    #[test]
    fn raw_device_code_is_preserved_for_reviewed_profiles() {
        let cfg = parse("[keymap]\nKeyH = 999\n").unwrap();
        assert_eq!(cfg.bindings, vec![(K::KeyH, Some(KeyBinding::Raw(999)))]);
    }

    #[test]
    fn unknown_key_name_and_action_are_config_errors() {
        assert!(parse("[keymap]\nWiggle = Fire\n")
            .unwrap_err()
            .message
            .contains("unknown key name"));
        assert!(parse("[keymap]\nKeyH = Teleport\n")
            .unwrap_err()
            .message
            .contains("not an action name"));
    }

    #[test]
    fn malformed_line_reports_its_number() {
        let err = parse("[keymap]\n\nKeyH\n").unwrap_err();
        assert_eq!(err.line, 3);
    }
}
