//! JAR classpath resources and MIDlet application properties.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::Path;

use j2me_me::ImageResources;

use crate::PlatformError;

pub const MAX_JAR_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_JAR_ENTRIES: usize = 16_384;
pub const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_EXPANDED_BYTES: u64 = 256 * 1024 * 1024;

fn validate_entry_name(name: &str) -> Result<(), PlatformError> {
    if name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name.contains('\0')
        || name
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(PlatformError::Resource(format!(
            "unsafe or non-canonical entry name {name:?}"
        )));
    }
    Ok(())
}

/// Parse manifest/JAD `Name: value` attributes, including continuation lines.
/// Later occurrences replace earlier ones, which makes applying a JAD override
/// a simple map extension.
pub fn parse_application_properties(bytes: &[u8]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut properties = BTreeMap::new();
    let mut current_name: Option<String> = None;
    let mut current_value = String::new();

    let flush = |name: &mut Option<String>, value: &mut String, out: &mut BTreeMap<_, _>| {
        if let Some(name) = name.take() {
            out.insert(name, std::mem::take(value));
        }
    };
    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        // A JAR manifest may contain named-entry sections after the blank line;
        // MIDlet application properties come only from the main section.
        if line.is_empty() {
            flush(&mut current_name, &mut current_value, &mut properties);
            break;
        }
        if let Some(continuation) = line.strip_prefix(' ') {
            if current_name.is_some() {
                current_value.push_str(continuation);
            }
            continue;
        }
        flush(&mut current_name, &mut current_value, &mut properties);
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        current_name = Some(name.trim().to_owned());
        current_value.push_str(value.trim());
    }
    flush(&mut current_name, &mut current_value, &mut properties);
    properties
}

fn normalized(path: &str) -> &str {
    path.trim_start_matches('/')
}

/// Resolve the name passed to `Class.getResourceAsStream` to a root-relative
/// classpath entry.
///
/// Java removes exactly one leading slash. Otherwise it resolves the name
/// relative to the package of the receiver class. `class_binary_name` uses the
/// source/binary spelling (`"java.lang.String"`, not `"java/lang/String"`).
/// The returned name never has the leading slash removed a second time: for
/// example, `"//asset.bin"` resolves to `"/asset.bin"`, which cannot alias the
/// ordinary root entry `"asset.bin"`.
pub fn resolve_class_resource_name(class_binary_name: &str, name: &str) -> String {
    if let Some(root_relative) = name.strip_prefix('/') {
        return root_relative.to_owned();
    }

    let Some((package, _class_name)) = class_binary_name.rsplit_once('.') else {
        return name.to_owned();
    };
    let mut resolved = package.replace('.', "/");
    resolved.push('/');
    resolved.push_str(name);
    resolved
}

/// UTF-16-facing form of [`resolve_class_resource_name`] for strict Java
/// transliterations.
///
/// Java `String` values can contain unpaired surrogates, while Rust `str`
/// cannot. Valid surrogate pairs are decoded to their exact Unicode scalar;
/// malformed UTF-16 returns `None` because it cannot name a UTF-8 ZIP entry.
/// No replacement character is introduced, so an ill-formed Java name cannot
/// accidentally alias a real resource containing U+FFFD.
pub fn resolve_class_resource_name_utf16(class_binary_name: &str, name: &[u16]) -> Option<String> {
    let name = String::from_utf16(name).ok()?;
    Some(resolve_class_resource_name(class_binary_name, &name))
}

/// The classpath surface used by `Class.getResourceAsStream`, named-image
/// loading, and `MIDlet.getAppProperty`.
pub trait ResourceSource {
    /// Resolve a root-relative JAR entry. A leading slash is optional.
    fn bytes(&self, path: &str) -> Option<&[u8]>;

    /// The resource lookup portion of `Class.getResourceAsStream(name)`.
    ///
    /// This returns borrowed classpath bytes rather than pretending that the
    /// host archive owns a Java `InputStream`. The JVM runtime/caller creates
    /// the stream object, preserving its cursor and failure behavior. A
    /// missing entry is `None`, matching the Java API's null result.
    fn class_resource_bytes(&self, class_binary_name: &str, name: &str) -> Option<&[u8]> {
        let resolved = resolve_class_resource_name(class_binary_name, name);
        // `bytes` deliberately accepts one or more leading slashes for direct
        // host convenience. Class lookup strips exactly one, so do not let a
        // double-leading-slash Java name alias an ordinary root entry.
        if resolved.starts_with('/') {
            return None;
        }
        self.bytes(&resolved)
    }

    /// UTF-16-facing `Class.getResourceAsStream(name)` lookup for strict Java
    /// transliterations. Ill-formed Java strings cannot name a UTF-8 JAR entry
    /// and therefore produce the API's null result (`None`).
    fn class_resource_bytes_utf16(&self, class_binary_name: &str, name: &[u16]) -> Option<&[u8]> {
        let resolved = resolve_class_resource_name_utf16(class_binary_name, name)?;
        if resolved.starts_with('/') {
            return None;
        }
        self.bytes(&resolved)
    }

    /// Sorted entry names without a leading slash. This is for tools/tests;
    /// game code should not invent directory enumeration the handset lacked.
    fn entries(&self) -> Vec<String>;

    /// `MIDlet.getAppProperty`: JAD attributes override JAR manifest values.
    fn application_property(&self, key: &str) -> Option<&str>;
}

/// An original JAR held in memory, with an optional sidecar JAD.
#[derive(Debug, Clone)]
pub struct JarResources {
    entries: BTreeMap<String, Vec<u8>>,
    properties: BTreeMap<String, String>,
}

impl JarResources {
    /// Read a JAR and, when present, its same-stem `.jad` sidecar.
    pub fn open(path: &Path) -> Result<Self, PlatformError> {
        let jar = std::fs::read(path)?;
        let jad_path = path.with_extension("jad");
        let jad = match std::fs::read(&jad_path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        Self::from_bytes(&jar, jad.as_deref())
    }

    /// Construct from authored/test bytes without touching the filesystem.
    pub fn from_bytes(jar: &[u8], jad: Option<&[u8]>) -> Result<Self, PlatformError> {
        if jar.len() > MAX_JAR_BYTES {
            return Err(PlatformError::Resource(format!(
                "JAR is {} bytes; limit is {MAX_JAR_BYTES}",
                jar.len()
            )));
        }
        let mut archive = zip::ZipArchive::new(Cursor::new(jar))?;
        if archive.len() > MAX_JAR_ENTRIES {
            return Err(PlatformError::Resource(format!(
                "JAR has {} entries; limit is {MAX_JAR_ENTRIES}",
                archive.len()
            )));
        }
        let mut entries = BTreeMap::new();
        let mut expanded = 0_u64;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().to_owned();
            validate_entry_name(&name)?;
            if entry.size() > MAX_ENTRY_BYTES {
                return Err(PlatformError::Resource(format!(
                    "entry {name:?} expands to {} bytes; per-entry limit is {MAX_ENTRY_BYTES}",
                    entry.size()
                )));
            }
            expanded = expanded
                .checked_add(entry.size())
                .ok_or_else(|| PlatformError::Resource("expanded JAR size overflow".to_owned()))?;
            if expanded > MAX_EXPANDED_BYTES {
                return Err(PlatformError::Resource(format!(
                    "expanded JAR exceeds {MAX_EXPANDED_BYTES} bytes"
                )));
            }
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry
                .by_ref()
                .take(MAX_ENTRY_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_ENTRY_BYTES {
                return Err(PlatformError::Resource(format!(
                    "entry {name:?} exceeded the per-entry limit while decoding"
                )));
            }
            if entries.insert(name.clone(), bytes).is_some() {
                return Err(PlatformError::Resource(format!(
                    "duplicate JAR entry {name:?}"
                )));
            }
        }

        let mut properties = entries
            .get("META-INF/MANIFEST.MF")
            .map(|bytes| parse_application_properties(bytes))
            .unwrap_or_default();
        if let Some(jad) = jad {
            properties.extend(parse_application_properties(jad));
        }
        Ok(Self {
            entries,
            properties,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl ResourceSource for JarResources {
    fn bytes(&self, path: &str) -> Option<&[u8]> {
        self.entries.get(normalized(path)).map(Vec::as_slice)
    }

    fn entries(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    fn application_property(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(String::as_str)
    }
}

impl ImageResources for JarResources {
    fn load(&self, name: &str) -> Option<Vec<i8>> {
        self.bytes(name)
            .map(|bytes| bytes.iter().map(|byte| *byte as i8).collect())
    }
}

/// Explicit resource bytes and properties for unit tests and process oracles.
#[derive(Debug, Clone, Default)]
pub struct MemoryResources {
    entries: BTreeMap<String, Vec<u8>>,
    properties: BTreeMap<String, String>,
}

impl MemoryResources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) -> &mut Self {
        self.entries.insert(normalized(path).to_owned(), bytes);
        self
    }

    pub fn property(&mut self, key: &str, value: &str) -> &mut Self {
        self.properties.insert(key.to_owned(), value.to_owned());
        self
    }
}

impl ResourceSource for MemoryResources {
    fn bytes(&self, path: &str) -> Option<&[u8]> {
        self.entries.get(normalized(path)).map(Vec::as_slice)
    }

    fn entries(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    fn application_property(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(String::as_str)
    }
}

impl ImageResources for MemoryResources {
    fn load(&self, name: &str) -> Option<Vec<i8>> {
        self.bytes(name)
            .map(|bytes| bytes.iter().map(|byte| *byte as i8).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn synthetic_jar() -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut output);
            let options = zip::write::SimpleFileOptions::default();
            archive.start_file("META-INF/MANIFEST.MF", options).unwrap();
            archive
                .write_all(b"MIDlet-Name: Manifest Name\r\nLong: one\r\n two\r\n")
                .unwrap();
            archive.start_file("images/icon.bin", options).unwrap();
            archive.write_all(&[0, 127, 128, 255]).unwrap();
            archive.start_file("emoji/👻.bin", options).unwrap();
            archive.write_all(&[9]).unwrap();
            archive.finish().unwrap();
        }
        output.into_inner()
    }

    #[test]
    fn jar_lookup_normalizes_a_leading_slash() {
        let resources = JarResources::from_bytes(&synthetic_jar(), None).unwrap();
        assert_eq!(
            resources.bytes("/images/icon.bin"),
            Some(&[0, 127, 128, 255][..])
        );
        assert_eq!(
            resources.bytes("images/icon.bin"),
            resources.bytes("/images/icon.bin")
        );
        assert_eq!(resources.entries().len(), 3);
    }

    #[test]
    fn class_resource_lookup_is_absolute_or_package_relative() {
        let mut resources = MemoryResources::new();
        resources
            .insert("asset.bin", vec![1])
            .insert("fixtures/local.bin", vec![2]);

        assert_eq!(
            resolve_class_resource_name("java.lang.String", "/asset.bin"),
            "asset.bin"
        );
        assert_eq!(
            resolve_class_resource_name("fixtures.Loader", "local.bin"),
            "fixtures/local.bin"
        );
        assert_eq!(
            resolve_class_resource_name("Loader", "asset.bin"),
            "asset.bin"
        );
        assert_eq!(
            resources.class_resource_bytes("java.lang.String", "/asset.bin"),
            Some(&[1][..])
        );
        assert_eq!(
            resources.class_resource_bytes("fixtures.Loader", "local.bin"),
            Some(&[2][..])
        );
    }

    #[test]
    fn class_resource_lookup_strips_exactly_one_leading_slash() {
        let mut resources = MemoryResources::new();
        resources.insert("asset.bin", vec![1]);

        assert_eq!(
            resolve_class_resource_name("java.lang.String", "//asset.bin"),
            "/asset.bin"
        );
        assert_eq!(
            resources.class_resource_bytes("java.lang.String", "//asset.bin"),
            None
        );
    }

    #[test]
    fn class_resource_utf16_accepts_pairs_and_rejects_lone_surrogates() {
        let resources = JarResources::from_bytes(&synthetic_jar(), None).unwrap();
        let non_bmp_name: Vec<u16> = "/emoji/👻.bin".encode_utf16().collect();

        assert_eq!(
            resolve_class_resource_name_utf16("java.lang.String", &non_bmp_name),
            Some("emoji/👻.bin".to_owned())
        );
        assert_eq!(
            resources.class_resource_bytes_utf16("java.lang.String", &non_bmp_name),
            Some(&[9][..])
        );
        assert_eq!(
            resolve_class_resource_name_utf16("java.lang.String", &[b'/' as u16, 0xd800]),
            None
        );
        assert_eq!(
            resources.class_resource_bytes_utf16("java.lang.String", &[b'/' as u16, 0xdc00]),
            None
        );
    }

    #[test]
    fn jad_overrides_manifest_and_continuations_are_joined() {
        let resources = JarResources::from_bytes(
            &synthetic_jar(),
            Some(b"MIDlet-Name: Installed Name\r\nJad-Only: yes\r\n"),
        )
        .unwrap();
        assert_eq!(
            resources.application_property("MIDlet-Name"),
            Some("Installed Name")
        );
        assert_eq!(resources.application_property("Long"), Some("onetwo"));
        assert_eq!(resources.application_property("Jad-Only"), Some("yes"));
    }

    #[test]
    fn named_manifest_sections_do_not_override_midlet_properties() {
        let properties = parse_application_properties(
            b"MIDlet-Name: Main\r\n\r\nName: icon.png\r\nMIDlet-Name: Section\r\n",
        );
        assert_eq!(
            properties.get("MIDlet-Name").map(String::as_str),
            Some("Main")
        );
        assert!(!properties.contains_key("Name"));
    }

    #[test]
    fn memory_source_is_deterministic_and_implements_image_resources() {
        let mut resources = MemoryResources::new();
        resources
            .insert("/b", vec![255])
            .insert("/a", vec![128, 1])
            .property("MIDlet-Name", "Fixture");
        assert_eq!(resources.entries(), vec!["a", "b"]);
        assert_eq!(resources.load("/a"), Some(vec![-128, 1]));
        assert_eq!(
            resources.application_property("MIDlet-Name"),
            Some("Fixture")
        );
    }

    #[test]
    fn unsafe_and_duplicate_archive_entries_are_rejected() {
        assert!(validate_entry_name("../outside").is_err());
        assert!(validate_entry_name("dir\\file").is_err());
        assert!(validate_entry_name("/absolute").is_err());
        assert!(validate_entry_name("normal/asset.bin").is_ok());
    }
}
