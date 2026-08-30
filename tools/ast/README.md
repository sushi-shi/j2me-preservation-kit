# Per-node Java/Rust AST crosswalk

Three pieces turn "the transliteration is verified" into something a machine can
enforce, node by node:

- `JavaAstAuditDump.java` emits formatting-independent `javac` AST items and a
  stable pre-order node inventory (`KIND\tdetail` per line), including fields and
  initializers.
- `j2me-ast-audit` emits formatting-independent `syn` AST items and the matching
  pre-order body/declaration node inventory, including statement-position macros.
- `validate_crosswalk.py` is the generic verifier. It reads a **manifest** of
  reviewer decisions + locked digests and an **evidence** table (what the live
  emitters/classfile reader currently produce), and enforces that **every** Java
  node and **every** Rust node carries its own explicit decision and comment.

## Why per-node

A crosswalk that pairs whole bodies proves almost nothing. Gothic's coarse audit
mapped 365 of 429 bodies entirely-Java-to-entirely-Rust under one blanket
"operation", and a real bug survived it (`paint_radio_row`: the Rust computed
`sprite_meta[1717*11+1721]` via the flat `sm(..)` accessor where the bytecode is
the *ratio* `sprite_meta[1717] / sprite_meta[1721]`). Under one blanket nobody had
to pair that single Rust call node against the Java `array-index / array-index /
DIVIDE` nodes, so the divergence was invisible.

## The contract (all can-fail proven)

1. **Exhaustive per-node coverage.** Every Java node and every Rust node is
   decided exactly once; undecided nodes are counted and, under `--strict`, fatal.
   A body is only `crosswalked` when nothing is undecided.
2. **Coarse-blanket rejection.** A single paired `op` may not span more than
   `policy.blanket_max_span` (default 48) nodes on either side — a whole-body
   blanket is rejected and must be decomposed into atomic steps.
3. **Atomic operator-realization parity.** Inside a paired `op`, a Java `DIVIDE` /
   `REMAINDER` / shift must be realized on the Rust side (`a / b` or the sanctioned
   `j2me-jvm` div/rem/signed-shift/unsigned-shift helper families). A Java
   division paired against a bare `sm(..)` call — the exact gothic bug — is red.
4. **Literal / index parity.** Inside a paired `op`, the numeric constants each
   side carries — array/string index literals, integer/char literals, numeric
   arguments — must be equal in value (hex/decimal/typed forms and immediately
   nested Rust unary-minus literals normalized, so Rust `0xff` matches Java `255`
   and Rust `-1` matches Java `-1`). A Rust `entity_row(..)[13]` paired against the
   faithful Java `[10]` — the real `build_dialogue_menu` crash — is red. A narrow
   sanctioned transform needs both a reasoned `op.literal_note` and an exact
   `op.literal_delta`; `op.shape_note` is reserved for broader representation
   differences.
5. **Crossing-ownership rejection.** Normal AST nesting may produce `A-B-A` in
   pre-order: one decision owns a parent and later argument around a nested
   receiver owned by another. `A-B-A-B` (or longer alternation) is different:
   two semantic decisions cross each other. The validator rejects it unless an
   exact body-level `interleave` owner group documents why the cross-language
   representation genuinely requires that ordering.
6. **Hash locks.** The bytecode Code-attribute digest, the full javac AST digest,
   each syn AST digest, and the pre-order node-inventory digests are all locked;
   a one-node drift breaks a lock.

## Manifest shape (schema 2)

```toml
schema_version = 2
total_body_count = <all bytecode bodies>
reviewed_body_count = <len(body)>
crosswalked_body_count = <bodies fully decided + semantic_status = "crosswalked">

[policy]
blanket_max_span = 48

[[body]]
java_item = "paint_radio_row(...)"
code_sha256 = "..."          # authoritative bytecode Code-attr (wrapper recomputes)
opcode_sha256 = "..."
java_ast_sha256 = "..."
java_nodes_sha256 = "..."
java_node_count = 71
semantic_status = "crosswalked"
review = "human prose"
rust = [
  { file = "…/shell_paint.rs", item = "fn:paint_radio_row",
    ast_sha256 = "…", nodes_sha256 = "…", node_count = 66 },
]
op = [
  # one semantically-atomic step; java/rust name the exact node indices (or ranges)
  { semantic = "icon = c.b[1717] / c.b[1721] — RATIO (idiv)",
    java_range = [[31, 40]], rust_range = [{ target = 0, start = 33, end = 41 }] },
  # a reviewed literal-only representation transform locks its exact multiset
  { semantic = "use the host's one-based row representation",
    java = [41], rust = ["0:42"],
    literal_note = "The host representation stores this row one-based.",
    literal_delta = { java_only = [10], rust_only = [11] } },
  # …one op per atomic step…
]
adapt = [
  # the only legal one-sided nodes; each categorized + reasoned
  { category = "representation-adapter", reason = "hoisted local",
    rust_range = [{ target = 0, start = 42, end = 43 }] },
  { category = "erased", reason = "javac `+ 0` no-op the port elides",
    java = [62, 66] },
]

# Only for a genuine representation-level A-B-A-B ownership crossing. Owner
# names are the zero-based decision positions above. Unknown owners, owner pairs
# that do not actually cross, and an empty reason are all rejected.
interleave = [
  { side = "rust", target = 0, owners = ["op:4", "adapt:0"],
    reason = "Rust stages the fallible call and its typed adapter across one call AST." },
]
```

Node references: a Java node is a bare index; a Rust node is `"target:index"`.
Rust `interleave` groups likewise require that exact `target`; Java groups must
not contain one. Crossings and known owners are checked independently for every
Rust target, so a target-0 waiver cannot discharge a target-1 crossing.
Contiguous atomic steps may use `java_range = [[a,b]]` / `rust_range = [{ target,
start, end }]`; the blanket cap still applies to the resulting node count.
`op.shape_note` is the explicit escape hatch for a legitimate cross-language
representation where the operator or literal parity would otherwise fire. A
literal-only transform instead uses the narrower `op.literal_note` together with
`op.literal_delta = { java_only = [...], rust_only = [...] }`. Both arrays are
required; they are exact one-sided integer multisets, so order is irrelevant but
duplicates matter. Prose without the delta, a wrong delta, or a stale delta after
the inventories come back into parity is red; subtracting the declared deltas
must leave equal literal multisets.
`op.blanket_ok` + `op.blanket_reason` similarly justifies a rare genuinely-large
atomic step.

## Running it

```sh
python3 tools/ast/validate_crosswalk.py MANIFEST --evidence EVIDENCE [--strict] [--coverage]
python3 tools/ast/validate_crosswalk.py --self-test          # prove it can fail
```

## Scoping one live body

`scope_crosswalk.py` is the game-agnostic coordinator for starting a body row.
It invokes the shipped `JavaAstAuditDump` and `j2me-ast-audit` emitters rather
than maintaining a third AST model, selects the exact named items, and prints:

- indexed `J:N` and `R:TARGET:N` node inventories for the human crosswalk;
- AST and node-inventory SHA-256 locks plus node counts;
- a schema-2 `body` stub with empty `op` / `adapt` decisions and
  `semantic_status = "partial"`;
- the matching live `evidence` object consumed by the generic validator.

For example, after the port's bytecode reader has supplied the authoritative
Code-attribute and opcode hashes:

```sh
python3 tools/ast/scope_crosswalk.py \
  --java-source java/src/main/java/example/InkEngine.java \
  --java-owner InkEngine \
  --java-item 'savedGameExistsInRMS(String)' \
  --rust transliteration/game-xlat/src/lib.rs \
         fn:ink_engine_saved_game_exists_in_rms \
  --body-key 'ExtBase.savedGameExistsInRMS(Ljava/lang/String;)Z' \
  --code-sha256 <64-hex-code-hash> \
  --opcode-sha256 <64-hex-opcode-hash>
```

Repeat `--java-source` when the canonical item may live among several source
files and repeat `--rust FILE ITEM` for a body with multiple Rust targets. The
default annotated output ends with copyable JSON body/evidence objects;
`--json` emits the whole report as one machine-readable document. Both original
bytecode hashes are required, so a source-only match cannot accidentally become
an admissible row. The scoper never invents node ownership: split the indexed
lists into atomic `op` decisions and categorized one-sided `adapt` decisions,
then run `validate_crosswalk.py --strict --coverage` against live evidence.

`fixtures/` holds three self-contained reproductions, regenerated by
`fixtures/gen_fixture.py`. `paint_radio_row.crosswalk.toml` (fine, green),
`.coarse.toml` (whole-body blanket → red), `.buggy.toml` (the div-vs-`sm()` shape
→ red), and `.evidence.toml` exercise the coverage/blanket/operator teeth.
`entity_row_index.crosswalk.toml` (index 10 both sides, green),
`.badindex.toml` (Rust `[13]` vs Java `[10]` → red), and `.evidence.toml` exercise
the literal/index parity tooth. `temporal_interleave.crosswalk.toml` is a
reasoned A-B-A-B staging difference (green); `.unjustified.toml` removes only
that owner group and goes red against the same `.evidence.toml`.

## Port responsibility

The verifier is deliberately evidence-driven. A per-game wrapper is still
responsible for extracting the original classfile Code-attribute digests,
selecting exact source/Rust items, emitting live evidence through the two
emitters, recomputing every digest, and proving reverse ownership of every
production Rust declaration. See `docs/CROSSWALK_MIGRATION.md` for how a port on
the old coarse-operation format adopts this schema.
