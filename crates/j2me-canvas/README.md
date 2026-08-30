# j2me-canvas

Game-neutral ARGB image storage and source-over compositing for J2ME
(Java ME / MIDP) game ports.

`Image` is a plain, mutable-or-immutable buffer of 32-bit ARGB pixels — the pixel
substrate that the device runtime surfaces as
`javax.microedition.lcdui.Image`. Encoded PNG/JPEG handling is deliberately
*outside* this crate: a host or game-specific resource adapter decodes bytes and
calls [`Image::from_argb`]. `source_over` is the straight Porter-Duff
source-over blend used by the runtime's blitters.

## Where it sits

This is a foundation crate of the J2ME preservation stack. It has no
dependencies. `j2me-me` (the MIDP device runtime) builds its `Graphics` surface
on top of it, and `j2me-nokia` blits into the same buffers.

```text
j2me-canvas ◄── j2me-me ◄── j2me-nokia
j2me-jvm    ◄──┘        ◄──┘
```

## Usage

```rust
use j2me_canvas::{Image, source_over};

// createImage(int, int) — a mutable, white-filled surface
let mut img = Image::create_mutable(16, 16).unwrap();

// blend a translucent red over the first pixel
let dst = img.pixels()[0];
img.pixels_mut()[0] = source_over(0x80ff_0000, dst);

// an immutable image from decoded ARGB
let sprite = Image::from_argb(2, 1, vec![0xffff_ffff, 0xff00_0000]).unwrap();
assert_eq!(sprite.width(), 2);
```

## License

Dedicated to the public domain under **CC0 1.0 Universal**. See the workspace
`LICENSE`.
