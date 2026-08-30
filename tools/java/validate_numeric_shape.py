#!/usr/bin/env python3
"""The R8 numeric-shape authority for the baseline's transliteration surface.

Java promotes ``byte``/``short`` to ``int`` before arithmetic and narrows the
``int`` result on a cast back; a decompiler routinely hides that. A decompiled
``a / b`` fed to a float can be *either* an integer ``idiv`` widened afterwards
(``idiv`` then ``i2f``) *or* a per-operand widened float divide (``i2f i2f
fdiv``) — the Java text is identical, the arithmetic is not. Rulebook R8 says the
bytecode is the authority and a decompiled numeric expression is never trusted.

This tool extracts, straight from the ``.class`` bytes (never a decompiler), the
ORDERED sequence of numeric opcodes each method executes — arithmetic, shifts,
bitwise ops, ``iinc``, every ``x2y`` conversion, and the ``lcmp``/``fcmp``/
``dcmp`` comparisons (which carry the NaN-ordering a transliterator must
reproduce). That sequence is the arithmetic *shape* the transliterated method
must reproduce exactly. A transliterator consults it before porting a method:
if the port's widen/narrow/convert order does not match, the port is wrong even
if it compiles and "looks like" the decompiled Java.

The authority lands in git-ignored ``_reference/numeric-shapes.json`` (evidence,
regenerable). Modes:

  * (default)     regenerate the authority file.
  * ``--check``   regenerate and compare byte-for-byte against the file on disk;
                  exit non-zero on drift, a missing file, or a nondeterministic
                  render. This is the gate.
  * ``--self-test`` prove the check goes red on a perturbed shape (rulebook R3).
  * ``--show C.n:desc`` print one method's numeric shape (for a transliterator).

Reuses ``tools/corpus/classfile.py`` (the from-scratch class parser) and
``tools/corpus/corpus.py`` (the content-hash-verified surviving payloads).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore

REPO = Path(__file__).resolve().parents[2]
CORPUS = REPO / "tools" / "corpus"
sys.path.insert(0, str(CORPUS))

import classfile  # noqa: E402  (tools/corpus/classfile.py)
import corpus  # noqa: E402  (tools/corpus/corpus.py)

OUT = REPO / "_reference" / "numeric-shapes.json"

# The baseline is the transliteration surface (builds.toml `baseline`). Its
# configured game classes are the only transliterated code. Order is fixed for
# a stable, byte-identical render.
with (REPO / "game.toml").open("rb") as handle:
    CONFIG = tomllib.load(handle)
JAVA_CONFIG = CONFIG.get("java", {})
BASELINE = corpus.load_manifest()["baseline"]
GAME_CLASSES = JAVA_CONFIG.get("baseline_classes", [])
EXPECTED_METHOD_COUNT = JAVA_CONFIG.get("expected_method_count", 0)
MINIMUM_NUMERIC_OPCODES = JAVA_CONFIG.get("minimum_numeric_opcodes", 0)
MINIMUM_METHODS_WITH_NUMERIC = JAVA_CONFIG.get("minimum_methods_with_numeric_opcodes", 0)

# The JVM's numeric-operation opcodes, in opcode order (0x60..0x98). Every one
# either does width-sensitive arithmetic, changes a value's width/type, or
# encodes NaN-ordering — exactly the decisions R8 forbids trusting a decompiler
# for. Anything outside this set (loads, stores, branches, invokes) is not part
# of the arithmetic shape and is deliberately excluded.
NUMERIC_OPCODES = {
    0x60: "iadd", 0x61: "ladd", 0x62: "fadd", 0x63: "dadd",
    0x64: "isub", 0x65: "lsub", 0x66: "fsub", 0x67: "dsub",
    0x68: "imul", 0x69: "lmul", 0x6A: "fmul", 0x6B: "dmul",
    0x6C: "idiv", 0x6D: "ldiv", 0x6E: "fdiv", 0x6F: "ddiv",
    0x70: "irem", 0x71: "lrem", 0x72: "frem", 0x73: "drem",
    0x74: "ineg", 0x75: "lneg", 0x76: "fneg", 0x77: "dneg",
    0x78: "ishl", 0x79: "lshl", 0x7A: "ishr", 0x7B: "lshr",
    0x7C: "iushr", 0x7D: "lushr",
    0x7E: "iand", 0x7F: "land", 0x80: "ior", 0x81: "lor",
    0x82: "ixor", 0x83: "lxor",
    0x84: "iinc",
    0x85: "i2l", 0x86: "i2f", 0x87: "i2d",
    0x88: "l2i", 0x89: "l2f", 0x8A: "l2d",
    0x8B: "f2i", 0x8C: "f2l", 0x8D: "f2d",
    0x8E: "d2i", 0x8F: "d2l", 0x90: "d2f",
    0x91: "i2b", 0x92: "i2c", 0x93: "i2s",
    0x94: "lcmp",
    0x95: "fcmpl", 0x96: "fcmpg", 0x97: "dcmpl", 0x98: "dcmpg",
}

# Documentation block emitted into the file so a reader (or a transliterator)
# knows exactly which opcodes define the shape.
TRACKED = [NUMERIC_OPCODES[k] for k in sorted(NUMERIC_OPCODES)]


class ShapeError(RuntimeError):
    pass


def baseline_payload() -> bytes:
    for build in corpus.builds():
        if build.build_id == BASELINE:
            return build.payload
    raise ShapeError(f"baseline {BASELINE} not found among surviving payloads")


def _numeric_shape(code: bytes) -> list[str]:
    """The ordered numeric-opcode mnemonics of one method's Code attribute."""
    shape: list[str] = []
    for _, opcode, _operand in classfile.instructions(code):
        mnemonic = NUMERIC_OPCODES.get(opcode)
        if mnemonic is not None:
            shape.append(mnemonic)
    return shape


def _method_code(payload: bytes, class_member: str) -> dict[str, bytes]:
    """Map (name, descriptor) -> raw Code bytes for one class, by re-reading the
    Code attribute directly (classfile.py hashes the code but does not retain the
    bytes; we re-walk the class to keep this tool self-contained and honest)."""
    raw = {name: data for name, data in corpus.jar_members(payload)}
    if class_member not in raw:
        raise ShapeError(f"{class_member} absent from {BASELINE}")
    return raw[class_member]


def build_shapes(payload: bytes | None = None) -> dict:
    """The authoritative numeric-shape table for the configured game classes.

    A pure function of the class bytes — deterministic, so the render is
    byte-identical across runs.
    """
    if payload is None:
        payload = baseline_payload()
    members = {name: data for name, data in corpus.jar_members(payload)}

    methods: list[dict] = []
    for class_name in GAME_CLASSES:
        member = f"{class_name}.class"
        if member not in members:
            raise ShapeError(f"{member} absent from {BASELINE}")
        info = classfile.parse_class(member, members[member])
        # Re-walk the class for the raw Code bytes (classfile keeps the hash,
        # not the bytes). Parsing twice is cheap and keeps one parser of record.
        code_by_key = _raw_code_by_key(members[member])
        for method in info.methods:
            key = (method.name, method.descriptor)
            code = code_by_key.get(key)
            shape = _numeric_shape(code) if code is not None else []
            methods.append(
                {
                    "class": class_name,
                    "ordinal": method.ordinal,
                    "name": method.name,
                    "descriptor": method.descriptor,
                    "abstract": code is None,
                    "numeric_shape": shape,
                    "shape_sha256": classfile.sha256("\n".join(shape)),
                }
            )
    return {
        "build": BASELINE,
        "note": (
            "R8 numeric-shape authority: the ordered JVM numeric opcodes each "
            "transliterated method must reproduce. Generated from class bytes; "
            "never hand-edit (regenerate with tools/java/validate_numeric_shape.py)."
        ),
        "opcodes_tracked": TRACKED,
        "classes": GAME_CLASSES,
        "method_count": len(methods),
        "methods": methods,
    }


def _raw_code_by_key(class_bytes: bytes) -> dict[tuple[str, str], bytes]:
    """(name, descriptor) -> raw Code attribute bytes, parsed from the class.

    A minimal re-walk built on classfile.py's primitives so this tool never
    depends on a decompiler and never second-guesses the one class parser.
    """
    reader = classfile.Reader(class_bytes)
    if reader.u4() != 0xCAFEBABE:
        raise classfile.ClassFormatError("invalid class magic")
    reader.u2()  # minor
    reader.u2()  # major
    pool = classfile.parse_constant_pool(reader)
    reader.u2()  # access flags
    reader.u2()  # this_class
    super_index = reader.u2()
    _ = super_index
    for _ in range(reader.u2()):  # interfaces
        reader.u2()
    for _ in range(reader.u2()):  # fields
        reader.u2()
        reader.u2()
        reader.u2()
        classfile.parse_attributes(reader, pool)
    out: dict[tuple[str, str], bytes] = {}
    for _ in range(reader.u2()):  # methods
        reader.u2()  # access
        name = pool.utf8(reader.u2())
        descriptor = pool.utf8(reader.u2())
        code = None
        for attr in classfile.parse_attributes(reader, pool):
            if attr.name == "Code":
                sub = classfile.Reader(attr.data)
                sub.u2()  # max_stack
                sub.u2()  # max_locals
                code = sub.take(sub.u4())
        if code is not None:
            out[(name, descriptor)] = code
    return out


def render(data: dict) -> bytes:
    """Deterministic JSON bytes (stable key order, trailing newline)."""
    text = json.dumps(data, indent=1, ensure_ascii=True)
    return (text + "\n").encode("utf-8")


def _shape_index(data: dict) -> dict[str, list[str]]:
    return {
        f"{m['class']}.{m['name']}:{m['descriptor']}": m["numeric_shape"]
        for m in data["methods"]
    }


def _assert_non_vacuous(data: dict) -> None:
    """Guard against a vacuous authority (GATES.md rule 2): the extractor must
    actually have found numeric opcodes across many methods, not compare empty
    lists against empty lists."""
    total_ops = sum(len(m["numeric_shape"]) for m in data["methods"])
    with_ops = sum(1 for m in data["methods"] if m["numeric_shape"])
    if not GAME_CLASSES or EXPECTED_METHOD_COUNT <= 0:
        raise ShapeError("game.toml [java] must define baseline_classes and expected_method_count")
    if data["method_count"] != EXPECTED_METHOD_COUNT:
        raise ShapeError(
            f"{data['method_count']} methods; expected exactly {EXPECTED_METHOD_COUNT}"
        )
    if total_ops < MINIMUM_NUMERIC_OPCODES or with_ops < MINIMUM_METHODS_WITH_NUMERIC:
        raise ShapeError(
            f"numeric shape looks vacuous: {total_ops} opcodes across "
            f"{with_ops} methods (extractor likely broken)"
        )


def generate() -> int:
    data = build_shapes()
    _assert_non_vacuous(data)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(render(data))
    total = sum(len(m["numeric_shape"]) for m in data["methods"])
    print(
        f"numeric-shape: wrote {OUT.relative_to(REPO)} — "
        f"{data['method_count']} methods, {total} numeric opcodes across "
        f"{', '.join(GAME_CLASSES)}."
    )
    return 0


def check() -> int:
    """Regenerate and compare byte-for-byte to the on-disk authority."""
    first = render(build_shapes())
    second = render(build_shapes())
    if first != second:
        print("numeric-shape --check FAIL: render is nondeterministic", file=sys.stderr)
        return 1
    _assert_non_vacuous(json.loads(first))
    if not OUT.is_file():
        print(
            f"numeric-shape --check FAIL: {OUT.relative_to(REPO)} is missing; "
            f"run `just numeric-shape` first",
            file=sys.stderr,
        )
        return 1
    on_disk = OUT.read_bytes()
    if on_disk != first:
        want = _shape_index(json.loads(first))
        have = _shape_index(json.loads(on_disk))
        drifted = [k for k in want if want.get(k) != have.get(k)]
        print(
            "numeric-shape --check FAIL: on-disk authority differs from a fresh "
            f"regeneration ({len(drifted)} method(s) drifted). First few: "
            f"{drifted[:5]}",
            file=sys.stderr,
        )
        return 1
    print(f"numeric-shape --check OK: {OUT.relative_to(REPO)} is byte-identical to a regen.")
    return 0


def self_test() -> int:
    """R3 can-fail proof: perturb one method's numeric shape by exactly one
    opcode and confirm the byte-identical comparison the gate relies on goes
    red and names the perturbed method."""
    data = build_shapes()
    _assert_non_vacuous(data)
    clean = render(data)

    # Find a real method that actually has a numeric shape to perturb.
    victim = next(
        (m for m in data["methods"] if m["numeric_shape"]),
        None,
    )
    if victim is None:
        print("self-test FAIL: no method carries a numeric shape", file=sys.stderr)
        return 1

    perturbed = json.loads(clean.decode("utf-8"))
    for m in perturbed["methods"]:
        if (m["class"], m["name"], m["descriptor"]) == (
            victim["class"],
            victim["name"],
            victim["descriptor"],
        ):
            # Swap one integer op for its float sibling: the exact R8 confusion
            # (idiv vs fdiv) a decompiler hides. Fall back to appending a
            # conversion if the first op is not idiv.
            shape = list(m["numeric_shape"])
            if "idiv" in shape:
                shape[shape.index("idiv")] = "fdiv"
            else:
                shape[0] = "i2f" if shape[0] != "i2f" else "i2b"
            m["numeric_shape"] = shape
            m["shape_sha256"] = classfile.sha256("\n".join(shape))
            break

    dirty = render(perturbed)
    if dirty == clean:
        print("self-test FAIL: perturbing a shape did not change the render", file=sys.stderr)
        return 1

    want = _shape_index(json.loads(clean))
    have = _shape_index(json.loads(dirty))
    drifted = [k for k in want if want.get(k) != have.get(k)]
    expected = f"{victim['class']}.{victim['name']}:{victim['descriptor']}"
    if drifted != [expected]:
        print(
            f"self-test FAIL: expected exactly [{expected}] to drift, got {drifted}",
            file=sys.stderr,
        )
        return 1
    print(
        "numeric-shape --self-test OK: a one-opcode perturbation of "
        f"{expected} is caught by the byte-identical check (gate can go red)."
    )
    return 0


def show(query: str) -> int:
    data = build_shapes()
    index = _shape_index(data)
    if query not in index:
        # Be forgiving: allow "C.name" without descriptor to list matches.
        matches = [k for k in index if k.startswith(query)]
        if not matches:
            print(f"no method matches {query!r}", file=sys.stderr)
            return 1
        for k in matches:
            print(f"{k}\n    {index[k]}")
        return 0
    print(f"{query}\n    {index[query]}")
    return 0


def main(argv: list[str]) -> int:
    if not argv:
        return generate()
    if argv[0] == "--check":
        return check()
    if argv[0] == "--self-test":
        return self_test()
    if argv[0] == "--show" and len(argv) >= 2:
        return show(argv[1])
    print(__doc__)
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
