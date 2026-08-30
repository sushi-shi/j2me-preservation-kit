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

Its `java.io` layer owns the exact four-field state of
`ByteArrayInputStream` (`buf`, `pos`, `mark`, and `count`). A
`DataInputStream` owns and delegates to that stream rather than keeping a
second cursor, so ranged streams, mark/reset, and partial consumption before an
`EOFException` remain observable exactly as they are on the JVM. This is an
ordinary `std` runtime layer; only the separate bounded serialization codecs
are intended to be `no_std`.

`DataOutputStream` owns a `ByteArrayOutputStream`-compatible memory sink.
Closing this exact pairing is a no-op: accumulated bytes remain available, an
earlier byte-array snapshot remains an independent copy, and the sink remains
writable, matching Java's byte-array stream contract.

Its `java.lang.Thread` layer is cooperative and deterministic: constructing a
thread allocates identity, `start()` queues that identity exactly once, and the
host explicitly dispatches the queued Runnable. During dispatch,
`current_thread()` exposes the same identity that Java's
`Thread.currentThread()` would return. The runtime never invents native-thread
timing, so headless tests and web hosts share the same ordering contract.

## Usage

```rust
use j2me_jvm::{i32_add, java_div, JavaError};

// wrapping 32-bit add, exactly like the JVM `iadd`
assert_eq!(i32_add(i32::MAX, 1), i32::MIN);

// division by zero is an ArithmeticException, not a panic
assert_eq!(java_div(1, 0), Err(JavaError::Arithmetic));
```

An owned stream can move through a `DataInputStream` without copying its
backing allocation or losing its cursor:

```rust
use j2me_jvm::{ByteArrayInputStream, DataInputStream};

let stream = ByteArrayInputStream::new_range(vec![0, 0x12, 0x34, 9], 1, 2);
let mut input = DataInputStream::from_stream(stream);
assert_eq!(input.read_unsigned_short().unwrap(), 0x1234);

let stream = input.into_inner();
assert_eq!(stream.position(), 3);
assert_eq!(stream.available(), 0);
```

A host can drive Java Runnable work without racing the transliterated game:

```rust
use j2me_jvm::{RunnableId, ThreadRuntime};

let mut threads = ThreadRuntime::new();
let thread = threads.new_thread(Some(RunnableId(7)));
threads.start(thread).unwrap();
threads
    .dispatch_next(|threads, running, target| {
        assert_eq!(threads.current_thread(), Some(running));
        assert_eq!(target, RunnableId(7));
        Ok::<_, ()>(())
    })
    .unwrap();
```

## License

Dedicated to the public domain under **CC0 1.0 Universal**. See the workspace
`LICENSE`.
