# j2me-platform

Host-side adapters shared by resource-free Java ME ports:

- JAR classpath resources with exact absolute/package-relative
  `Class.getResourceAsStream` name resolution and manifest/JAD application
  properties;
- deterministic in-memory resources for tests and process oracles;
- a versioned filesystem snapshot for `j2me-me::RmsRuntime`;
- a safe per-game application-data directory derived from `game.toml`'s slug;
- integer-scaled centered 2D placement, viewport cropping, and ARGB→RGBA; and
- profile-ordered focus/pause/resume/show/hide actions; and
- reviewed handset system-property/default-charset facts with a separate layer
  for dynamic host, session, or operator overrides; and
- profile-gated HTTP/SMS host seams whose safe default performs no external
  action.

This crate contains no windowing, output-device/browser API, game policy, or
private game resources. CPAL/winit code is in `j2me-platform-native`; Web APIs
are in `j2me-platform-web`; codecs/DSP are in `j2me-media`.

`ResourceSource::class_resource_bytes` owns only classpath name resolution and
the Java API's missing-resource/null distinction. It intentionally returns
borrowed bytes: the JVM layer or strict transliteration must create the actual
`InputStream`/`DataInputStream`, so cursor state, callback failures, and
construction timing are not hidden in the host archive adapter.

Strict transliterations whose Java strings are stored as UTF-16 use
`ResourceSource::class_resource_bytes_utf16`. It decodes valid surrogate pairs
without loss and returns `None` for an unpaired surrogate; it never performs a
lossy replacement that could redirect a malformed Java name to a different JAR
entry. The `&str` method remains the ergonomic host/authored-code surface.
