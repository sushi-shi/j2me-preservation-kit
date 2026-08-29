# Three-authority AST audit

The reusable audit has three pieces:

- `JavaAstAuditDump.java` emits formatting-independent `javac` AST items and a
  stable pre-order node list, including field and initializer structure.
- `j2me-ast-audit` emits formatting-independent `syn` AST items and semantic
  body/declaration nodes, including statement-position macros.
- `validate_crosswalk.py` validates a game manifest in which original bytecode
  and opcode hashes, complete Java/Rust AST hashes, and semantic node ranges are
  recorded. Every Java and Rust node must have exactly one owner.

The validator is intentionally manifest-driven. A per-game wrapper is still
responsible for extracting the original classfile denominator, selecting exact
source/Rust items, recomputing every stored hash, and proving reverse ownership
of every production Rust declaration. Do not turn this generic validator into a
substitute for those authority checks.
