# j2me-input

Generic, remappable native-key → Nokia `FullCanvas` key-code mapping for J2ME
ports. This is the single place a native host translates a physical `winit`
`KeyCode` into the fixed, game-agnostic Nokia vocabulary a transliterated
`keyPressed(int)` expects, so every port inherits sane, remappable controls
instead of hand-copying a per-game keymap.

## The Nokia vocabulary

| key            | code    |
|----------------|---------|
| D-pad U/D/L/R  | -1..=-4 |
| Fire / select  | -5      |
| soft keys L/R  | -6/-7   |
| digits 0–9     | 48..=57 |
| `*` / `#`      | 42 / 35 |

Anything outside this set is rejected by the device event queue (R10), so an
unknown or unmapped key resolves to `None` and the host drops it.

## Presets

- **`Preset::Standard`** — Arrows = D-pad, Enter/Space = Fire, F1/F2 = soft keys,
  number row + numpad = digits, `[`/`]`/`\`/numpad-`*` = `*`/`#`.
- **`Preset::Mobile`** (default) — everything in `Standard` **plus** a left-hand
  cluster mirroring gothic-mobile's and stalker-mobile's keymaps: W/A/S/D move,
  Q/E soft keys, X fires, R/F reach `*`/`#`. A strict superset — every `Standard`
  binding still works.

## Overriding without editing code

Pass an optional `[keymap]` table (from `game.toml`, a standalone `keymap.toml`,
or an env var). It selects a preset and layers per-key overrides on top:

```toml
[keymap]
preset = "mobile"       # "mobile" (default) or "standard"

KeyH  = "SoftLeft"      # bind by action name
KeyJ  = -5              # ...or by raw Nokia code (Fire)
KeyQ  = "none"          # explicitly unbind a preset key
Comma = "*"             # a symbol action (quote values containing # or *)
```

Values may be an action name (`Up`, `Fire`, `SoftLeft`, `Digit3`/`3`, `Star`/`*`,
`Pound`/`#`, …), a raw Nokia code, or `none`/`unbind`. Key names accept winit
variant names (`KeyW`, `ArrowUp`, `NumpadMultiply`) and friendly aliases (`W`,
`Up`, `Esc`, `KP5`). Bad key/action names and out-of-vocabulary codes are
reported as a `ConfigError` with a line number.

## Using it from a host

```rust
use j2me_input::{Keymap, KeyCode, Preset};

let keymap = Keymap::from_config(player_config /* Option<&str> */, Preset::Mobile)?;

// In the winit keyboard handler:
if let Some(code) = keymap.nokia_code(physical_key) {
    canvas.key_pressed(code); // feed the transliterated keyPressed(int)
}
```

A port that today hand-writes a `nokia_code(KeyCode) -> Option<i32>` swaps the
whole function for one `Keymap` built at start-up plus one `keymap.nokia_code`
call at the event site — same codes, now remappable and shared across ports.

Part of the J2ME Preservation Kit. Licensed CC0-1.0.
