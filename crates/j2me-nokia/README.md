# j2me-nokia

`com.nokia.mid.ui.DirectGraphics` / `DirectUtils` — the Nokia UI vendor blitter,
as an **opt-in** extension over the neutral MIDP device runtime (`j2me-me`).

Games that render through a Nokia `FullCanvas` with `drawPixels` (rather than
standard `Graphics.drawImage`) depend on this crate; games that use only
standard MIDP do not. It wraps a *live* `j2me_me::Graphics` via
`get_direct_graphics`, sharing that `Graphics`' current translate + clip, so a
`setClip` after `getDirectGraphics` is honored exactly as on a handset.

**The pixel-format conversions are the contract.** 4444→8888 replicates each
4-bit nibble into both halves of the channel (`0xF → 0xFF`); 8888→4444 truncates
to the high nibble. The pair is exact on a round trip, which the sprite-bake
pattern relies on. Only the implemented formats
(`TYPE_USHORT_4444_ARGB`, `TYPE_INT_8888_ARGB`) and manipulations
(`0`, `FLIP_HORIZONTAL`) are accepted; anything else is rejected with a typed
error rather than silently misinterpreted.

## Where it sits

The optional top of the J2ME preservation stack. It depends on `j2me-canvas`,
`j2me-jvm`, and `j2me-me`. Add it only when a game needs the Nokia
`drawPixels` / `getPixels` path.

## Usage

```rust
use j2me_me::{Graphics, Image};
use j2me_nokia::get_direct_graphics;

let mut frame = Image::create_mutable(32, 32).unwrap();
let mut g = Graphics::new(&mut frame);
let dg = get_direct_graphics(&mut g); // getDirectGraphics(g)
// dg.draw_pixels_4444(...) / dg.get_pixels_8888(...)
let _ = dg;
```

## License

Dedicated to the public domain under **CC0 1.0 Universal**. See the workspace
`LICENSE`.
