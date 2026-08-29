# Oracle routes (per-game)

A **route** is a text script of keystrokes and screenshot points. The same route
file drives both runtimes of the frame oracle:

- the reference runtime — FreeJ2ME-Plus, via `tools/oracle/HeadlessCapture.java`;
- the future Rust port, via its own `--script` frontend.

Because both consume the *same* keystrokes, the comparison is "the same route
into both" rather than two hand-matched runs. `compare_frames.py` reads the
`shot` lines here to know which labels to pair and compare.

Routes are **per-game and not shipped in the template**: the capture script
globs `tools/oracle/routes/*.txt`, so you write your own. `00-boot.txt.example`
is a documented starting point — copy it to `00-boot.txt` and edit. (The
`.example` suffix keeps it out of the glob until you rename it.)

## Command grammar

One command per line; `#` starts a comment; blank lines are ignored. Every
command may carry trailing `key=value` tokens, and **any key a consumer does not
recognise is ignored** — that is what lets one file feed the emulator driver, the
Rust port, and `compare_frames.py` at once. The header of `HeadlessCapture.java`
is the authoritative reference.

| Command | Meaning |
| --- | --- |
| `wait <ms>` | advance time with no input; `java_frames=<n>`/`java_ms=<n>` are the reference's units, `frames=<n>` the port's |
| `tap <KEY> [hold_ms] [settle_ms]` | press, hold, release, settle |
| `hold <KEY> [ms]` / `down` | press and leave pressed |
| `release <KEY> [ms]` / `up` | release |
| `seed <n>` | reseed every loaded game `java.util.Random` in place |
| `fps <n>` | switch to deterministic gated frame stepping |
| `shot <label> [k=v ...]` | write `<label>.png` (the comparison unit) |
| `echo <text>` | log a marker |

**Keys:** `UP DOWN LEFT RIGHT FIRE SOFT1 SOFT2 SEND END STAR POUND` and
`NUM0`..`NUM9` (Nokia/MIDP key codes).

## Determinism

`fps <n>` puts the reference on gated frame stepping over FreeJ2ME's substituted
clock, so `currentTimeMillis()`/`nanoTime()` advance by each command's exact
millisecond budget instead of by host speed. `seed <n>` pins any RNG-driven
animation just before a shot that must reproduce byte-for-byte. Both are safe to
leave in place on RNG-free, statically-timed screens.

## Labels and the ratchet

Every `shot <label>` must have an entry in `tools/oracle/agreement.toml`. Add new
labels there via `compare_frames.py --update-ratchet` (after reviewing each diff),
never by hand-editing a number to make a run pass.
