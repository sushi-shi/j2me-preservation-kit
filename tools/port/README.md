# Whole-port completeness gates

The per-node crosswalk answers whether each admitted Java/Rust body is reviewed.
These tools answer the surrounding whole-program questions without moving any
game judgment into the preservation kit.

## 1. Ownership and call-path closure

```sh
python3 tools/port/validate_completeness.py \
  transliteration/audits/completeness.toml
```

The live coordinator reads the baseline JAR through `tools/corpus/classfile.py`
and every production Rust declaration through `j2me-ast-audit`. The manifest
must cover exactly:

- every field of the configured baseline game classes;
- every Rust declaration below the configured production roots; and
- every selected direct bytecode call edge.

Minimal schema:

```toml
schema_version = 1

[inventory]
# Defaults to game.toml [java].baseline_classes when omitted.
java_classes = ["a", "b"]
rust_roots = ["transliteration/game-xlat/src", "apps/game/src"]
rust_exclude = ["**/generated/*.rs"]
# "game" or "game-and-platform". The latter also inventories calls into the
# configured Java/Java-ME/vendor namespaces.
call_scope = "game"
platform_prefixes = ["java/", "javax/", "com/nokia/"]

[ratchet]
java_fields = 37
java_methods = 112
java_call_edges = 164
rust_declarations = 205

[[field_ownership]]
java = "a.b:I" # exact original owner.name:descriptor
classification = "state" # state/constant/aggregate/derived/host/erased
reason = "Persistent score field."
rust = [{ file = "transliteration/game-xlat/src/state.rs", item = "field:GameState::score", type_contains = "i32" }]

[[rust_declaration]]
file = "transliteration/game-xlat/src/state.rs"
item = "field:GameState::score"
ast_sha256 = "..."
classification = "java-field"
owner = "a.b:I"

[[rust_declaration]]
file = "apps/game/src/host.rs"
item = "fn:dispatch_frame"
ast_sha256 = "..."
classification = "host-adapter"
reason = "Desktop event-loop boundary."

[[call_edge]]
caller = "a.c:()V"
callee = "b.d:(I)V"
classification = "path" # path/runtime/dispatch-adapter/erased/dead
reason = "Direct translated call through the frame adapter."
rust_path = [
  { file = "transliteration/game-xlat/src/lib.rs", item = "fn:tick", contains = ["draw_world"] },
  { file = "apps/game/src/host.rs", item = "fn:draw_world", contains = ["render"] },
]
```

Every `rust_path` hop is selected from the live AST inventory, must itself have
reverse ownership, and must contain its reviewed AST marker. The manifest owns
the semantic claim; string markers are only a can-fail witness inside a fully
hash-locked declaration.

## 2. Optional linked-binary reachability

```sh
python3 tools/port/native_reachability.py \
  transliteration/audits/completeness.toml
```

Do not add this command to a game's green gate battery until `[native] enabled =
true`. Disabled invocation fails instead of becoming a green skip. The current
backend is deliberately described as Linux/ELF: it reads demangled `objdump`
calls and resolves cross-crate GOT calls through configured `readelf`
relocations. A different executable format needs its own backend.

```toml
[native]
enabled = true
build_command = ["cargo", "build", "-p", "game-linux", "--bin", "game-linux"]
binary = "target/debug/game-linux"
relocation_type = "R_X86_64_RELATIVE"
roots = ["game_linux::main"]
reachable_symbol_count = 18420

[native.floor]
relocations = 2000
functions = 3000
resolved_edges = 5000

[[native.target]]
file = "apps/game/src/main.rs"
item = "fn:frame"
symbol = "game_linux::frame"
match = "prefix" # exact/prefix/contains
expect = "reachable"
category = "production"
reason = "Registered frame callback."

[[native.target]]
file = "transliteration/game-xlat/src/oracle.rs"
item = "fn:oracle_fixture"
symbol = "game_xlat::oracle::oracle_fixture"
expect = "unreachable"
category = "oracle-only"
reason = "Differential process only; must not enter the shipped closure."
```

Native targets must also exist in `[[rust_declaration]]`; an oracle target must
carry the `oracle-infrastructure` reverse classification.

## 3. Method/signature variants

```sh
python3 tools/port/validate_variants.py java/reconstruction/variants/world.toml
python3 tools/port/validate_variants.py java/reconstruction/variants/world.toml --inventory
```

Use `identity = "ordinal"` when all builds retain the same method-table layout.
Use `identity = "signature"` when methods appear or disappear. In both modes,
the ledger locks exact presence, name, descriptor, shape hash, and build group.

```toml
schema_version = 1
owner = "a"
identity = "signature"
builds = ["nokia-en", "sony-en"]
expected_build_count = 2
expected_method_keys = 1

[[method]]
key = "a:()V"
classification = "device-policy"
reason = "Sony changes the soft-key branch."
observation = [
  { builds = ["nokia-en"], present = true, name = "a", descriptor = "()V", shape_sha256 = "..." },
  { builds = ["sony-en"], present = true, name = "a", descriptor = "()V", shape_sha256 = "..." },
]
```

## Can-fail proofs

```sh
python3 tools/port/validate_completeness.py --self-test
python3 tools/port/native_reachability.py --self-test
python3 tools/port/validate_variants.py --self-test
python3 -m unittest tools.tests.test_port_gates
```

The negative controls remove a field decision, orphan a Rust declaration, drop
a call edge, change a Rust path marker, leak an oracle target, and change a
method shape. Each perturbation must turn its corresponding gate red.
