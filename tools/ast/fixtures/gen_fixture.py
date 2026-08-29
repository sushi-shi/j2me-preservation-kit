#!/usr/bin/env python3
"""Generate the `paint_radio_row` per-node crosswalk fixture.

This recreates the shape of the real gothic bug (docs finding G-39): the Rust
computed ``sprite_meta[1717*11+1721]`` via the flat 2-D accessor ``sm(..)`` where
the bytecode is the *ratio* ``sprite_meta[1717] / sprite_meta[1721]`` (an
``idiv``). Under one blanket operation the mismatch was invisible; under the
per-node model it cannot hide.

Running this script rewrites four committed files:

* ``paint_radio_row.evidence.toml`` — the emitted AST node inventories and the
  authoritative digests, for both a *fixed* Rust body and a *buggy* one;
* ``paint_radio_row.crosswalk.toml`` — the FINE, fully-decided manifest (green);
* ``paint_radio_row.coarse.toml``    — a single whole-body blanket op (red);
* ``paint_radio_row.buggy.toml``     — atomic decisions over the buggy Rust body,
  where the division step pairs a Java DIVIDE against a bare ``sm(..)`` call (red).

The digests are computed here so the manifest locks and the evidence agree; a
per-game port recomputes the same digests from live ``javac`` / ``syn`` /
classfile tools instead of from this generator.
"""

from __future__ import annotations

import hashlib
from pathlib import Path

HERE = Path(__file__).resolve().parent


def sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def digest(nodes: list[str]) -> str:
    return sha("\n".join(nodes))


# A "segment" is one atomic step. `kind` is op | adapt-rust | adapt-java.
# java/rust are the node lines (KIND\tdetail) that step contributes on each side.
class Seg:
    def __init__(self, kind, comment, java=None, rust=None, category=None, extra=None):
        self.kind = kind
        self.comment = comment
        self.java = java or []
        self.rust = rust or []
        self.category = category
        self.extra = extra or {}


def div_rust_fixed() -> list[str]:
    return [
        "LOCAL\ticon",
        "CALL\t2",
        "PATH_EXPR\tj2me_jvm :: java_div",
        "INDEX\t",
        "PATH_EXPR\tsprite_meta",
        "LITERAL\t1717",
        "INDEX\t",
        "PATH_EXPR\tsprite_meta",
        "LITERAL\t1721",
    ]


def div_rust_buggy() -> list[str]:
    # sm(g, 1717, 1721) — the flat 2-D accessor. No division node anywhere.
    return [
        "LOCAL\ticon",
        "METHOD_CALL\tsm\t3",
        "PATH_EXPR\tg",
        "LITERAL\t1717",
        "LITERAL\t1721",
    ]


def segments(div_rust) -> list[Seg]:
    """The atomic steps of paint_radio_row, in bytecode order."""
    return [
        Seg(
            "op",
            "byte line = (byte)((font_metrics[2] >> 16) & 255); int height = line + 5",
            java=[
                "VARIABLE\tline\tint",
                "TYPE_CAST\tbyte",
                "AND\t",
                "RIGHT_SHIFT\t",
                "ARRAY_ACCESS\t",
                "MEMBER_SELECT\tfont_metrics",
                "INT_LITERAL\t2",
                "INT_LITERAL\t16",
                "INT_LITERAL\t255",
                "VARIABLE\theight\tint",
                "PLUS\t",
                "IDENTIFIER\tline",
                "INT_LITERAL\t5",
            ],
            rust=[
                "LOCAL\tline",
                "CAST\ti8",
                "BINARY\t&",
                "METHOD_CALL\twrapping_shr\t1",
                "INDEX\t",
                "PATH_EXPR\tfont_metrics",
                "LITERAL\t2",
                "LITERAL\t16",
                "LITERAL\t255",
                "LOCAL\theight",
                "METHOD_CALL\twrapping_add\t1",
                "PATH_EXPR\tline",
                "LITERAL\t5",
            ],
        ),
        Seg(
            "op",
            "present guard: graphics != null && label != null",
            java=[
                "IF\t",
                "CONDITIONAL_AND\t",
                "NOT_EQUAL_TO\t",
                "IDENTIFIER\tgraphics",
                "NULL_LITERAL\tnull",
                "NOT_EQUAL_TO\t",
                "IDENTIFIER\tlabel",
                "NULL_LITERAL\tnull",
            ],
            rust=[
                "IF\t",
                "LET_EXPR\t",
                "TUPLE\t2",
                "PATH_EXPR\tgraphics",
                "PATH_EXPR\tlabel",
            ],
        ),
        Seg(
            "op",
            "THE FIX: icon = c.b[1717] / c.b[1721] — sprite-156 total-width / "
            "frame-width RATIO (idiv), not a single raw index",
            java=[
                "VARIABLE\ticon\tint",
                "DIVIDE\t",
                "ARRAY_ACCESS\t",
                "MEMBER_SELECT\tb",
                "IDENTIFIER\tc",
                "INT_LITERAL\t1717",
                "ARRAY_ACCESS\t",
                "MEMBER_SELECT\tb",
                "IDENTIFIER\tc",
                "INT_LITERAL\t1721",
            ],
            rust=div_rust,
        ),
        Seg(
            "adapt-rust",
            "the port hoists menu_layout_x into a local f; only the declaration is "
            "Rust-only, its uses pair inside the branch ops",
            rust=[
                "LOCAL\tf",
                "FIELD\tmenu_layout_x",
            ],
            category="representation-adapter",
        ),
        Seg(
            "op",
            "icon_x = 5 + menu_layout_x",
            java=[
                "VARIABLE\ticon_x\tint",
                "PLUS\t",
                "INT_LITERAL\t5",
                "MEMBER_SELECT\tmenu_layout_x",
            ],
            rust=[
                "LOCAL\ticon_x",
                "METHOD_CALL\twrapping_add\t1",
                "LITERAL\t5",
                "PATH_EXPR\tf",
            ],
        ),
        Seg(
            "op",
            "icon_dy = (height >> 1) - 1",
            java=[
                "VARIABLE\ticon_dy\tint",
                "MINUS\t",
                "RIGHT_SHIFT\t",
                "IDENTIFIER\theight",
                "INT_LITERAL\t1",
                "INT_LITERAL\t1",
            ],
            rust=[
                "LOCAL\ticon_dy",
                "METHOD_CALL\twrapping_sub\t1",
                "METHOD_CALL\twrapping_shr\t1",
                "PATH_EXPR\theight",
                "LITERAL\t1",
                "LITERAL\t1",
            ],
        ),
        Seg(
            "op",
            "indent = icon + 10",
            java=[
                "VARIABLE\tindent\tint",
                "PLUS\t",
                "IDENTIFIER\ticon",
                "INT_LITERAL\t10",
            ],
            rust=[
                "LOCAL\tindent",
                "METHOD_CALL\twrapping_add\t1",
                "PATH_EXPR\ticon",
                "LITERAL\t10",
            ],
        ),
        Seg(
            "adapt-java",
            "indent's outer `+ 0` (a javac-visible no-op the bytecode carries as "
            "iconst_0; iadd) is elided by the port; it cannot change the value",
            java=[
                "PLUS\t",
                "INT_LITERAL\t0",
            ],
            category="erased",
        ),
        Seg(
            "op",
            "selected branch guard + fill: if (on) setColor(palette[10]); "
            "fillRect(menu_layout_x, y, menu_layout_w, line + 2)",
            java=[
                "IF\t",
                "IDENTIFIER\ton",
                "METHOD_INVOCATION\tsetColor\t1",
                "ARRAY_ACCESS\t",
                "MEMBER_SELECT\tcolor_palette",
                "INT_LITERAL\t10",
                "METHOD_INVOCATION\tfillRect\t4",
                "MEMBER_SELECT\tmenu_layout_x",
                "IDENTIFIER\ty",
                "MEMBER_SELECT\tmenu_layout_w",
                "PLUS\t",
                "IDENTIFIER\tline",
                "INT_LITERAL\t2",
            ],
            rust=[
                "IF\t",
                "PATH_EXPR\ton",
                "METHOD_CALL\tset_color\t1",
                "INDEX\t",
                "PATH_EXPR\tcolor_palette",
                "LITERAL\t10",
                "METHOD_CALL\tfill_rect\t4",
                "PATH_EXPR\tf",
                "PATH_EXPR\ty",
                "FIELD\tmenu_layout_w",
                "METHOD_CALL\twrapping_add\t1",
                "PATH_EXPR\tline",
                "LITERAL\t2",
            ],
        ),
        Seg(
            "op",
            "toggle sprite: draw_sprite_frame(icon_x, y + icon_dy, 156, "
            "menu_context_flags[index])",
            java=[
                "METHOD_INVOCATION\tdraw_sprite_frame\t4",
                "IDENTIFIER\ticon_x",
                "PLUS\t",
                "IDENTIFIER\ty",
                "IDENTIFIER\ticon_dy",
                "INT_LITERAL\t156",
                "ARRAY_ACCESS\t",
                "MEMBER_SELECT\tmenu_context_flags",
                "IDENTIFIER\tindex",
            ],
            rust=[
                "METHOD_CALL\tdraw_sprite_frame\t4",
                "PATH_EXPR\ticon_x",
                "METHOD_CALL\twrapping_add\t1",
                "PATH_EXPR\ty",
                "PATH_EXPR\ticon_dy",
                "LITERAL\t156",
                "METHOD_CALL\tget\t1",
                "PATH_EXPR\tmenu_context_flags",
                "PATH_EXPR\tindex",
            ],
        ),
        Seg(
            "op",
            "return height",
            java=[
                "RETURN\t",
                "IDENTIFIER\theight",
            ],
            rust=[
                "PATH_EXPR\theight",
            ],
        ),
    ]


def build(div_rust):
    segs = segments(div_rust)
    java_nodes: list[str] = []
    rust_nodes: list[str] = []
    decisions = []  # (type, comment, java_range|None, rust_range|None, category)
    for seg in segs:
        j0 = len(java_nodes)
        java_nodes.extend(seg.java)
        j1 = len(java_nodes) - 1
        r0 = len(rust_nodes)
        rust_nodes.extend(seg.rust)
        r1 = len(rust_nodes) - 1
        jr = [j0, j1] if seg.java else None
        rr = {"target": 0, "start": r0, "end": r1} if seg.rust else None
        decisions.append((seg.kind, seg.comment, jr, rr, seg.category))
    return java_nodes, rust_nodes, decisions


def toml_str(value: str) -> str:
    escaped = value.replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'


def toml_list(values: list[str]) -> str:
    return "[\n" + "".join(f"  {toml_str(v)},\n" for v in values) + "]"


def emit_evidence() -> str:
    java_nodes, rust_fixed, _ = build(div_rust_fixed())
    _, rust_buggy, _ = build(div_rust_buggy())
    java_ast = sha("java-ast:paint_radio_row:" + "\n".join(java_nodes))
    code = sha("code:paint_radio_row")
    opcode = sha("opcode:paint_radio_row")

    def body(item: str, rust: list[str]) -> str:
        rust_ast = sha("rust-ast:" + item + ":" + "\n".join(rust))
        return "\n".join(
            [
                "[[body]]",
                f"java_item = {toml_str(item)}",
                f"code_sha256 = {toml_str(code)}",
                f"opcode_sha256 = {toml_str(opcode)}",
                f"java_ast_sha256 = {toml_str(java_ast)}",
                f"java_nodes = {toml_list(java_nodes)}",
                "",
                "[[body.rust]]",
                'file = "transliteration/game-xlat/src/shell_paint.rs"',
                'item = "fn:paint_radio_row"',
                f"ast_sha256 = {toml_str(rust_ast)}",
                f"nodes = {toml_list(rust)}",
                "",
            ]
        )

    header = (
        "# GENERATED by tools/ast/fixtures/gen_fixture.py — do not hand-edit.\n"
        "# The live AST/bytecode evidence a per-game wrapper would emit. Both\n"
        "# bodies share the Java inventory; only the Rust div step differs.\n\n"
    )
    return header + body("paint_radio_row", rust_fixed) + body(
        "paint_radio_row_bug", rust_buggy
    )


def _digests(item: str, rust: list[str], java_nodes: list[str]):
    return {
        "code": sha("code:paint_radio_row"),
        "opcode": sha("opcode:paint_radio_row"),
        "java_ast": sha("java-ast:paint_radio_row:" + "\n".join(java_nodes)),
        "java_nodes": digest(java_nodes),
        "rust_ast": sha("rust-ast:" + item + ":" + "\n".join(rust)),
        "rust_nodes": digest(rust),
    }


def _manifest_head(reviewed: int, crosswalked: int) -> list[str]:
    return [
        "# GENERATED by tools/ast/fixtures/gen_fixture.py — do not hand-edit.",
        "schema_version = 2",
        'build = "fixture"',
        "total_body_count = 1",
        f"reviewed_body_count = {reviewed}",
        f"crosswalked_body_count = {crosswalked}",
        "",
        "[policy]",
        "blanket_max_span = 48",
        "",
    ]


def _body_head(item: str, java_nodes, rust, semantic: bool, review: str) -> list[str]:
    d = _digests(item, rust, java_nodes)
    lines = [
        "[[body]]",
        f"java_item = {toml_str(item)}",
        f"code_sha256 = {toml_str(d['code'])}",
        f"opcode_sha256 = {toml_str(d['opcode'])}",
        f"java_ast_sha256 = {toml_str(d['java_ast'])}",
        f"java_nodes_sha256 = {toml_str(d['java_nodes'])}",
        f"java_node_count = {len(java_nodes)}",
    ]
    if semantic:
        lines.append('semantic_status = "crosswalked"')
    lines.append(f"review = {toml_str(review)}")
    lines.append("rust = [")
    lines.append(
        "  { file = \"transliteration/game-xlat/src/shell_paint.rs\", "
        'item = "fn:paint_radio_row", '
        f"ast_sha256 = {toml_str(d['rust_ast'])}, "
        f"nodes_sha256 = {toml_str(d['rust_nodes'])}, "
        f"node_count = {len(rust)} }},"
    )
    lines.append("]")
    return lines


def _decision_lines(decisions) -> list[str]:
    ops = []
    adapts = []
    for kind, comment, jr, rr, category in decisions:
        if kind == "op":
            ops.append(
                f"  {{ semantic = {toml_str(comment)}, "
                f"java_range = [[{jr[0]}, {jr[1]}]], "
                f"rust_range = [{{ target = {rr['target']}, start = {rr['start']}, "
                f"end = {rr['end']} }}] }},"
            )
        elif kind == "adapt-rust":
            adapts.append(
                f"  {{ category = {toml_str(category)}, reason = {toml_str(comment)}, "
                f"rust_range = [{{ target = {rr['target']}, start = {rr['start']}, "
                f"end = {rr['end']} }}] }},"
            )
        elif kind == "adapt-java":
            adapts.append(
                f"  {{ category = {toml_str(category)}, reason = {toml_str(comment)}, "
                f"java_range = [[{jr[0]}, {jr[1]}]] }},"
            )
    lines = ["op = ["] + ops + ["]"]
    lines += ["adapt = ["] + adapts + ["]"]
    return lines


def emit_fine() -> str:
    java_nodes, rust, decisions = build(div_rust_fixed())
    review = (
        "The type-102 context/settings row painter, crosswalked node-for-node. Each "
        "atomic step is its own decision with its own comment; the division step "
        "(icon = c.b[1717] / c.b[1721]) is realized on the Rust side by java_div, so "
        "the operator-realization parity check is satisfied."
    )
    lines = _manifest_head(1, 1)
    lines += _body_head("paint_radio_row", java_nodes, rust, True, review)
    lines += _decision_lines(decisions)
    return "\n".join(lines) + "\n"


def emit_coarse() -> str:
    java_nodes, rust, _ = build(div_rust_fixed())
    review = (
        "COARSE (rejected): a single blanket operation over the whole body — the very "
        "shape that let the gothic paint_radio_row bug through. The verifier rejects "
        "it as a coarse blanket because one op spans the entire multi-node body."
    )
    lines = _manifest_head(1, 1)
    lines += _body_head("paint_radio_row", java_nodes, rust, True, review)
    lines += [
        "op = [",
        "  { semantic = \"map the whole radio row\", "
        f"java_range = [[0, {len(java_nodes) - 1}]], "
        f"rust_range = [{{ target = 0, start = 0, end = {len(rust) - 1} }}] }},",
        "]",
        "adapt = []",
    ]
    return "\n".join(lines) + "\n"


def emit_buggy() -> str:
    java_nodes, rust, decisions = build(div_rust_buggy())
    review = (
        "BUGGY (rejected): the same atomic decomposition, but the Rust div step is the "
        "flat accessor sm(g, 1717, 1721) — no division node at all. The "
        "operator-realization parity check pairs a Java DIVIDE against a bare call and "
        "goes red, pointing at exactly the step that carried the real bug."
    )
    lines = _manifest_head(1, 0)
    lines += _body_head("paint_radio_row_bug", java_nodes, rust, False, review)
    lines += _decision_lines(decisions)
    return "\n".join(lines) + "\n"


def main() -> None:
    (HERE / "paint_radio_row.evidence.toml").write_text(emit_evidence())
    (HERE / "paint_radio_row.crosswalk.toml").write_text(emit_fine())
    (HERE / "paint_radio_row.coarse.toml").write_text(emit_coarse())
    (HERE / "paint_radio_row.buggy.toml").write_text(emit_buggy())
    java_nodes, rust, _ = build(div_rust_fixed())
    print(
        f"wrote fixture: {len(java_nodes)} Java nodes, {len(rust)} Rust nodes "
        f"(fine + coarse + buggy manifests, shared evidence)"
    )


if __name__ == "__main__":
    main()
