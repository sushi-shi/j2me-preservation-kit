# j2me-input

Generic, remappable physical-key → semantic handset-key mapping for J2ME ports.
The selected `j2me-device::InputFragment` performs the second hop to the raw
integer delivered to Java. Consequently desktop ergonomics and phone behavior
remain independently visible and testable.

## Nokia compatibility

| key            | code    |
|----------------|---------|
| D-pad U/D/L/R  | -1..=-4 |
| Fire / select  | -5      |
| soft keys L/R  | -6/-7   |
| digits 0–9     | 48..=57 |
| `*` / `#`      | 42 / 35 |

The legacy `nokia_code` helpers remain as explicitly named compatibility
wrappers over the opt-in `j2me-device-nokia` implementation. New hosts call
`Keymap::raw_code(key, &profile.input)`.

## Presets

- **`Preset::Standard`** — Arrows = D-pad, Enter/Space = Fire, F1/F2 = soft keys,
  number row + numpad = digits, `[`/`]`/`\`/numpad-`*` = `*`/`#`.
- **`Preset::Mobile`** (default) — everything in `Standard` **plus** a left-hand
  cluster mirroring the established preservation-port keymaps: W/A/S/D move,
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
if let Some(code) = keymap.raw_code(physical_key, &device_profile.input) {
    canvas.key_pressed(code); // feed the transliterated keyPressed(int)
}
```

A port that hand-writes a key-code function replaces it with a `Keymap` and a
reviewed device profile. Raw numeric config overrides remain available for a
phone with nonstandard codes; named bindings stay portable.

Part of the J2ME Preservation Kit. Licensed CC0-1.0.
