# Java-to-Rust transliteration contract

This is the binding guide for **implementation #1** of the Rust game: a strict,
mechanical transliteration of the recovered/canonical Java (`java/…/`) into the
per-game crate `__SLUG__-game-xlat` (lib name `game_xlat`).

Implementation #1 exists to be *provably the same program*, not to be good Rust.
It is the executable specification a later idiomatic implementation (#2) is
validated against, so its only virtues are fidelity and speed of construction.
**Do not make it pretty. Do not refactor it. Do not "improve" a name or fold a
redundant operation.** Every rule below exists because breaking it silently
changes behaviour.

This chapter is the per-game companion to the home playbook's faithful-arithmetic
and device-runtime contracts (`docs/ARITHMETIC_AND_RUNTIME.md` in the j2me home);
this file governs the transliteration crate, that one governs the whole method.
For the 2D-vs-3D fork see the home's `docs/FORK_2D_3D.md`; for the gates that
enforce all of this, `docs/GATES.md`.

> **Fork note.** If the numeric-opcode authority (Phase 2) finds **zero
> float/double opcodes** across the game's classes, the game is pure-integer:
> `j2me-jvm` ships **no** float helpers and none appear in transliterated code —
> if a `float`/`double` ever surfaces, stop, it is a decompilation error, not a
> licence to invent an `f32`. A 3D/M3G game is not pure-integer; keep `f32`/`f64`
> and **preserve float parenthesisation bit-for-bit** (IEEE addition is not
> associative). Let the opcode authority, not a guess, decide which you are.

## Scope

| Layer | Crate | Style |
| --- | --- | --- |
| Game classes | `__SLUG__-game-xlat` | Transliteration; this document |
| Device runtime | `j2me-me` | Idiomatic Rust implementing the Java ME contracts |
| Neutral 2D buffer | `j2me-canvas` | Idiomatic Rust ARGB framebuffer |
| Java primitives | `j2me-jvm` | Idiomatic Rust implementing JVM integer semantics |
| Reader/codec primitives | `j2me-codec` | Idiomatic Rust, `no_std` bounded readers |
| Host | `apps/<slug>-<platform>` | Idiomatic Rust |

Only `__SLUG__-game-xlat` is transliterated. The layers beneath it (`j2me-*`) are
ordinary Rust whose *observable behaviour* must match the Java ME / JVM
specification; they are the reusable, game-neutral crates and are extended only
when this game's baseline actually uses and verifies the surface. Any independent
format parser you write (a game-specific `-formats` crate) is **not** part of the
port — it is a second, separately-derived implementation used only as an oracle
(see *Cross-check oracles*).

Reusable improvements discovered during admission belong back in `_template`:
runtime primitives, process/oracle plumbing, AST walkers and validators, gate
routing, schemas, tests, recipes, and workflow prose. The per-game repository
keeps only facts that cannot be generalized without weakening evidence—build
hashes, canonical bodies and names, variant decisions, oracle adapters/vectors,
and the exhaustive node mappings themselves.

## The single most important rule

**Java promotes `byte`, `short` and `char` to `int` before arithmetic.** A cast
back is a *narrowing of an `int` result*, not saturating small-width arithmetic.

```java
n = (short) (n + 1);   // n : short
```
```rust
n = (n as i32 + 1) as i16;   // widen to int, add, narrow back to short
```

Writing `n + 1` on an `i16` is wrong twice over: it panics at `i16::MAX` in debug
builds, and wraps at a different width than Java in release. `as i32` first,
narrow last, always. `byte` narrows with `as i8`, `short` with `as i16`, `char`
with `as u16`. Keep `overflow-checks = true` in the dev profile so the debug
panic catches exactly this.

## Primitive mapping

| Java | Rust | Notes |
| --- | --- | --- |
| `boolean` | `bool` | |
| `byte` | `i8` | signed; `byte[]` is **`Vec<i8>`**, never `Vec<u8>`; a `byte[]` param is `&[i8]` / `&mut [i8]` |
| `short` | `i16` | |
| `char` | `u16` | **unsigned** 16-bit; a `char[]` is `Vec<u16>`; in arithmetic it **zero**-extends to `int` |
| `int` | `i32` | |
| `long` | `i64` | |
| `float` / `double` | `f32` / `f64` | 3D/pure-float games only; preserve parenthesisation exactly |
| `String` | `String` / `&str` | usually only resource paths and format tag strings |
| `T[]` | `Vec<T>` | fixed length after construction; never `push` unless Java does |
| `Object[]` (nullable) | `Vec<Option<T>>` | lazily-loaded / clearable slots |
| reference | field on a `*State` struct / `Option<Handle>` | see *Statics and ownership* |
| `null` | `None` | |

**`byte[]` as `Vec<i8>` is load-bearing** — this is the sign-extension rule the
whole contract turns on. A read of `data[i]` yields an `i8`, exactly as the JVM's
`baload` **sign-extends** the byte into an `int`; `data[i] & 255` then
re-zero-extends. Using `u8` changes both raw comparisons against negative
sentinels (a signature byte such as `-119`, a `< 0` check) and every shift/mask
that assumed the sign bit. Convert to `u8` **only** at the true host boundary
(handing bytes to a decoder outside the port, hashing) — never inside the
transliterated logic.

### Identity-bearing `Object` values

The value-oriented rows above apply only when Java reference identity cannot be
observed. Once a body reaches `new Integer`, `Object[]`, `instanceof`,
`checkcast`, or virtual `Object.equals`, use `j2me-jvm`'s generic object seam:

- store `JavaObjectRef`; use `Option<JavaObjectRef>` for Java `null`;
- let the host own the arena and class table behind `JavaObjectRuntime`;
- use `new_integer` for `new Integer` so equal values still receive fresh
  identities;
- use the provided cast and reference-array wrappers so null, negative-length,
  bounds, and cast failures occur before/at the same operation as on the JVM;
- obtain String payloads as exact UTF-16 with `string_utf16`; and
- route every `receiver.equals(argument)` through `object_equals`, even when the
  handles are equal or their current payloads look equal.

The seam deliberately does not provide a heap or a default equality policy.
The host adapter must perform dynamic dispatch, array component checks, and
allocation, and it may re-enter or fail. Keep those calls in Java evaluation
order and reread live game fields after a callback whenever the source does.

## Arithmetic

See `docs/ARITHMETIC_AND_RUNTIME.md` (home) for the full contract; the rules that
govern day-to-day transliteration:

**Never a bare operator on an integer.** Route every integer op through
`j2me-jvm`:

| Java op | Rust |
| --- | --- |
| `+ - *` | `wrapping_add` / `wrapping_sub` / `wrapping_mul` |
| `/` | `j2me_jvm::java_div(a, b)?` (`long`: `java_ldiv`) — traps `i32::MIN / -1` |
| `%` | `j2me_jvm::java_rem(a, b)?` (`long`: `java_lrem`) — follows the dividend's sign |
| `<< >>` | `wrapping_shl` / `wrapping_shr` — Java masks the count (5 bits int, 6 bits long) |
| `>>>` | `((x as u32) >> (n & 31)) as i32` (`long`: `((x as u64) >> (n & 63)) as i64`) |
| `& \| ^` | direct (`&` `\|` `^`) — bitwise ops do not overflow |

A debug-only panic where Java wrapped is a divergence; release-mode silence hides
it. `as i32` first, narrow last. Division/remainder helpers return
`Result<_, JavaError>`: at a site the original did not guard, `.expect(...)` on a
provably-non-zero constant divisor (a panic there is the faithful
`ArithmeticException`); inside a `try/catch` region, propagate with `?`. Even a
nonzero **constant** divisor routes through the helper — uniformity keeps review
mechanical and the one real `i32::MIN / -1` site from hiding.

`Math.abs(int)` is **not** `i32::abs`: `Math.abs(i32::MIN)` returns `i32::MIN`
(overflow), Rust's panics. Transliterate as `if x < 0 { x.wrapping_neg() } else { x }`.

### Opcode-shape authority

The numeric-opcode authority (Phase 2 — the ordered arithmetic/conversion opcodes
for every original method, extracted from the shipped `.class` files) is the
guard that the *decompilation* is faithful. Before transliterating a method, read
its shape and confirm the decompiled Java's arithmetic **multiset and structure**
match it. The Java source is what you transliterate; the shape proves it wasn't
mis-decompiled. A divergence in the *multiset* (an op present in one and not the
other) is a blocker; a divergence only in javac's internal *evaluation order*
(e.g. an `iinc` the decompiler rendered as `x + 1`) is expected and is not — you
transliterate the Java expression verbatim, preserving its parenthesisation.

## Statics and ownership

Java `static` fields become fields on one `*State` struct per class; a top-level
`Game` struct aggregates them. Methods become **free functions** taking the state
by `&mut`, never `self`-methods — `self` methods conflict the moment a method
needs two sub-structs at once.

```java
public final class Adler32 {
    private int sum = 1;
    public final void update(byte[] data, int offset, int length) { … }
}
```
```rust
pub struct Adler32State { pub sum: i32 }
impl Default for Adler32State { fn default() -> Self { Self { sum: 1 } } }
pub fn update(s: &mut Adler32State, data: &[i8], offset: i32, length: i32) { … }
```

A purely-static utility class (no instance/`static` mutable state) becomes **free
functions with no state parameter**. Only the mutable state of a class needs a
`*State` struct.

Maintain an ownership ledger (`ownership.tsv`) that assigns every Java field to a
single Rust owner and type; consult it before porting a class and never invent a
field's home. One Java static has exactly one persistent Rust owner: no second
copy, no copy-in/copy-out; screens/HUD/host borrow or derive a frame-local view.

### Accepted deviations

Any structural deviation is recorded here **before** it is written, each naming
the exact site and arguing why it is not observable. Typical, allowable ones:

- **Reset-then-use singletons.** A `static` engine whose every use is immediately
  preceded by `.reset()` may be replaced by a freshly-constructed local when a
  fresh instance is *bit-identical* to a reset one — the alias is not observable.
- **Deferred cross-boundary calls.** A call that crosses into an as-yet-unported
  class (a resource read, an `Image` wrap) is **deferred** with an explicit
  marker; the transliterated core is driven directly from injected bytes by the
  oracle until the boundary class lands.

## Overloads

Rust has no method overloading. When two Java methods share a name, the primary
keeps the base `snake_case` name and the secondary is disambiguated by a suffix
naming what distinguishes it (e.g. a two-arg convenience delegating to the
three-arg form becomes `…_default`). Where the original recovered an overload
only in raw-obfuscated code, the raw form gets a `_code` suffix and the semantic
form keeps the base name.

## Naming

`camelCase` → `snake_case`, mechanically. Constants stay `SCREAMING_SNAKE_CASE`.
Keep the recovered semantic names from the named-Java crosswalk — they are
reviewed evidence, not suggestions. Do not rename, abbreviate, or "improve" a
name during transliteration.

Each ported file opens with a provenance header naming the Java file and the
original obfuscated `.class`:

```rust
//! Transliterated from `java/…/Adler32.java`
//! (original `an.class` in `<the pinned build>.jar`).
//!
//! Implementation #1: strict transliteration. See `docs/TRANSLITERATION.md`.
```

## Exceptions

`j2me_jvm::JavaError` enumerates the raiseable exceptions; `JavaResult<T>` is the
alias.

- A method the original wraps in `try { } catch (Exception e) { … }` returns
  `Result<_, JavaError>`, and the call site does exactly what the catch block did.
- A method the original does **not** guard may panic. That is faithful: the
  MIDlet would have died too. An explicit `throw new …Exception()` at a leaf is
  transliterated as a `panic!` reproducing the exact predicate — an uncaught
  throw terminates.
- Inside a guarded region, array access goes through the checked accessors
  (`j2me_jvm::array_ref` / `array_mut`, which return `JavaResult`); outside one,
  index directly (`arr[i as usize]`), which panics on a bad index exactly as
  `baload`/`aaload` would.

## Preserved defects

Bugs in the shipped bytecode are transliterated **faithfully**, each carrying a
comment naming the Java file and line. A reviewer who "fixes" one has broken the
port. Shipped *behaviour* changes belong in the modern engine (as data or a
cleared config flag), **never** in the transliteration. Record each in a table
here, e.g.:

| Site | Defect |
| --- | --- |
| `<File>.java:<line>` | A sign-extending mask, a signed-overflow accumulator, or a redundant identity mask left in the shipped code — reproduced verbatim (wrapping arithmetic + the signed helper), with the site named. Pin each with an oracle test that proves the transliteration matches an independent reference *including* the defect. |

## No-ops

`System.gc()`, `Thread.yield()`, `e.printStackTrace()` and discarded allocations
become nothing or a single log line — **unless** ordering between them is
observable, in which case the surrounding order is preserved.

## Cross-check oracles ("two implementations, one truth")

A class is not done when it compiles — compilation is not equivalence. Each unit
is gated by an **independent** second implementation over **real** blobs from
`_originals/…`, following the `*_oracle.rs` pattern and the project rulebook
(`docs/GATES.md`). This is the per-unit companion to the two shared oracles in
`tools/oracle/`: the `line_oracle.py` line-differential oracle and the
FreeJ2ME-Plus exact-pixel **frame oracle** (`compare_frames.py`).

Every oracle carries:

- **Liveness** — assert real work happened (blobs processed, frames decoded),
  never `0`. A unit that processes 0 blobs must **fail**, not pass vacuously.
- **Count floors** — a non-vacuity floor for the corpus (record the baseline the
  first green run establishes; a run below the floor fails).
- **A negative control** — a one-unit perturbation (a corrupted byte, a
  cross-frame mismatch) proven to turn the gate red, so an agreement that survived
  a real mismatch cannot read as a pass.
- **Loud failure when `_originals/` is absent** — never a skip that reads green.

For Java reflection/trace harnesses, make the entry point an ordered dispatcher
from the start. Put each admitted body or tightly coupled tranche in its own
bounded `trace…` helper and pass a small explicit context (or only the reflected
members it consumes). A monolithic `main` eventually exceeds the classfile
limit of 65,535 Code bytes per method; splitting only after `javac` reports
`code too large` creates avoidable integration conflicts. Helper extraction must
leave trace-line order and hostile failure observations byte-for-byte unchanged.

## Verification

A class is done when its cross-check oracle passes over the real corpus, its
opcode multiset reconciles with the numeric-opcode authority, and every deviation
and preserved defect is recorded here — **not** when it compiles.
