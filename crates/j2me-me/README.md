# j2me-me

The reusable Java ME / MIDP **2D** device runtime for strict J2ME game ports —
idiomatic Rust whose *observable behavior* matches the Java ME contract. **2D
only** (no M3G / JSR-184).

It provides the device-runtime surface a strict 2D port draws on:

- **`graphics`** — `setColor` / `setFont` / clip / `translate`, lines,
  rectangles, rounded rectangles, triangles, images and regions, plus the
  `drawArc` / `fillArc` ellipse sector rasteriser (`Graphics`, `GraphicsError`,
  `SpriteTransform`);
- **`canvas`** — the `Canvas` / `Display` serial paint-input queue and the
  `Displayable` surface trait, including pointer, resize, repeat-key, and
  soft-command callbacks. `Display::set_current_notifying` and
  `clear_current_notifying` update visibility and dispatch the game subclass's
  ordered `showNotify` / `hideNotify` callbacks, preserving partial state when
  a callback fails;
- **`media`** — the MMAPI player model with `VolumeControl`, a `PlayerListener`
  registration, and the `getState()` integers;
- **`midlet`** — the two-phase MIDlet lifecycle boundary, without inventing
  desktop focus policy;
- **`rms`** — the monotonic-record-id `RecordStore` plus a deterministic
  host-facing snapshot seam;
- **`font`** — device-profile-selected immutable `FontSpec` descriptors,
  reviewed metrics, and glyph-provider validation;
- **`image`** — byte-array/stream/named-resource PNG factories and
  `Image.createRGBImage`.

## Where it sits

The runtime tier of the J2ME preservation stack. It builds on the neutral ARGB
buffer (`j2me-canvas`, re-exported here as `javax.microedition.lcdui.Image`) and
JVM primitive semantics (`j2me-jvm`); PNG decoding uses the pure-Rust `png`
crate. Game bodies transliterate against this surface, and the opt-in
`j2me-nokia` vendor blitter layers over its live `Graphics`.

```text
j2me-canvas ─┐
j2me-jvm   ──┼─► j2me-me ─► (game transliteration, j2me-nokia)
png        ──┘
```

The current-font latch is distilled from the sibling *Gothic 3: The Beginning*
port, where the recovered game repeatedly switches fonts on a live `Graphics`
before drawing its UI. The shared form keeps Gothic's proven per-context
default/set/get behavior, replaces its game-local font value with the reusable
device-selected `FontSpec`, and follows MIDP's rule that `setFont(null)` restores
the default. Text measurement and rasterization continue through the separate
`FontRuntime` provider boundary.

## Usage

```rust
use j2me_me::{Graphics, Image};

let mut frame = Image::create_mutable(64, 48).unwrap();
let mut g = Graphics::new(&mut frame);
g.set_color(0x00_3366);   // setColor(0x003366)
g.fill_rect(0, 0, 64, 48);
```

## License

Dedicated to the public domain under **CC0 1.0 Universal**. See the workspace
`LICENSE`.
