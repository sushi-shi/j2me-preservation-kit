# j2me-jvm

Game-neutral JVM primitive semantics for strict Java-to-Rust transliteration of
J2ME (Java ME / MIDP) games.

Ordinary Rust and the JVM disagree on the corners that matter when you port a
game byte-for-byte: 32-bit wrapping overflow, truncating integer division and
`rem`, masked shift counts, `int`↔`long` narrowing, signed array indices, thrown
exceptions, and the system clock. This crate is the single place those semantics
live, so a transliteration can match the original's *observable behavior* rather
than approximate it.

Two spellings of the same semantics are provided: a const-fn arithmetic surface
(`i32_add`, `narrow_*`, `array_ref`, `Clock`, `JavaError`) and a
bytecode-mnemonic surface (`ishl`, `java_div`, the `jget!` / `jset!`
checked-access macros) — pick whichever reads closest to the code being ported.
It is ordinary `std` Rust.

## Where it sits

A foundation crate of the J2ME preservation stack, with no dependencies. The
device runtime (`j2me-me`) and every game body route their arithmetic and array
access through it.

## Usage

```rust
use j2me_jvm::{i32_add, java_div, JavaError};

// wrapping 32-bit add, exactly like the JVM `iadd`
assert_eq!(i32_add(i32::MAX, 1), i32::MIN);

// division by zero is an ArithmeticException, not a panic
assert_eq!(java_div(1, 0), Err(JavaError::Arithmetic));
```

## License

Dedicated to the public domain under **CC0 1.0 Universal**. See the workspace
`LICENSE`.
