# Migrating a port from coarse operations to per-node crosswalk

The first crosswalk format (`transliteration/audits/method-audit.toml` schema 1,
still live in gothic) let one `operation` blanket a whole body:

```toml
# COARSE (schema 1): one operation, whole body, one label.
operation = [{ semantic = "paint the radio row",
               java_node_ranges = [[0, 103]],
               rust_node_ranges = [{ target = 0, start = 0, end = 47 }] }]
```

Every node had "an owner", but the owner was a blanket. That is exactly how the
`paint_radio_row` raw-index-vs-ratio bug (finding G-39) passed as "crosswalked":
the single Rust `sm(1717, 1721)` call node was never paired, node-for-node,
against the Java `array-index / array-index / DIVIDE` nodes, so the divergence
was invisible. **A coarse crosswalk is not a verified crosswalk.**

Schema 2 (`tools/ast/validate_crosswalk.py`) replaces the blanket with a
per-node decision model and refuses to call a body `crosswalked` until every node
on both sides is decided exactly once, under a semantically-atomic step or a
categorized one-sided adaptation.

## What changes in the manifest

| Schema 1 (coarse) | Schema 2 (per-node) |
| --- | --- |
| `operation = [{ semantic, java_node_ranges, rust_node_ranges }]` | `op = [{ semantic, java/java_range, rust/rust_range }]`, **one op per atomic step** |
| `adaptation = [{ side, reason, … }]` | `adapt = [{ category, reason, java/rust … }]` with a category from the fixed set |
| one op may span the whole body | an op may not span more than `policy.blanket_max_span` (48) nodes per side |
| `semantic_status = "crosswalked"` on any full-coverage body | `crosswalked` **requires** zero undecided nodes |
| node counts (`java_node_count`, `rust_node_counts`) | same, plus `java_nodes_sha256` and per-target `nodes_sha256` node-inventory locks |
| — | `code_sha256` / `opcode_sha256` / `java_ast_sha256` / per-target `ast_sha256`, re-derived from live evidence |

## Recommended per-body procedure

1. **Emit both node inventories.** Run `JavaAstAuditDump` for the canonical Java
   body and `j2me-ast-audit` for each Rust target; these are the indexed node
   lists you will reference. (`--show-java-nodes` / `--show-rust-nodes` style
   dumps make the indices visible.)
2. **Walk the bytecode in order.** For each atomic step the original performs,
   write one `op` whose `semantic` states what the step computes and whose `java`
   / `rust` indices name the nodes that implement it on each side. Keep each op to
   a single step — if you are tempted to write "and then… and then…", split it.
3. **Watch the operator parity.** When a step contains a `/`, `%`, or shift on the
   Java side, confirm the paired Rust nodes actually realize it (`a / b` or an
   audited `j2me-jvm` helper such as `i32_div`, `i64_rem`, `i32_shl`, or
   `i64_ushr`). This is where `paint_radio_row` broke: a division paired against
   a bare `sm(..)` call now fails the gate. Cite `op.shape_note` only for a
   representation the verifier cannot derive from those known helpers.
4. **Categorize every one-sided node.** A Rust-only host/representation adapter or
   a Java-only erased/no-op node goes in `adapt` with a category and a reason.
5. **Lock the digests.** Record `code_sha256`, `opcode_sha256`, `java_ast_sha256`,
   `java_nodes_sha256`, and per-target `ast_sha256` / `nodes_sha256`; the wrapper
   recomputes them from live sources so any later drift is red.
6. **Do not mark `crosswalked` until coverage is 100%.** `--coverage` shows
   `decided/total` per body; a partial body is honest work-in-progress, not a
   verified one. `--strict` requires zero undecided nodes across the manifest.

## Coverage as the burn-down number

`validate_crosswalk.py --coverage` prints, per body and overall, how many nodes
are still undecided. That number is the migration's burn-down: it starts at the
node count of every not-yet-fine body and must reach zero. Re-crosswalking the
existing coarse bodies to per-node coverage is mechanical but not automatic — each
blanket must be decomposed by a human who, in doing so, gets the chance to catch
the next `paint_radio_row`.
