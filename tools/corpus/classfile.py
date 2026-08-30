#!/usr/bin/env python3
"""Parse J2ME `.class` files directly from bytes and fingerprint their methods.

The original class files are the authority (rulebook R2). A decompiler is never
an input here: CFR/JD formatting and invented local names must not become symbol
identity. This module parses only the class-file structures this CLDC-era corpus
needs and computes three increasingly normalized method fingerprints:

  * ``code_sha256``   -- the exact Code-attribute bytecode (operands included);
  * ``opcode_sha256`` -- the instruction opcodes only (operands stripped);
  * ``shape_sha256``  -- opcodes plus *normalized* constant-pool meaning, so that
                          obfuscated single-letter class/member names (which an
                          obfuscator can reassign per build) do not change the
                          fingerprint (rulebook R10: match on shape, not name).

A malformed or truncated class raises :class:`ClassFormatError`; callers turn
that into a reported problem rather than a crash (parsers never panic).
"""

from __future__ import annotations

import hashlib
import json
import re
import struct
from dataclasses import dataclass, field
from typing import Any, Iterable


class ClassFormatError(ValueError):
    """A malformed, truncated, or unsupported class file."""


def sha256(data: bytes | str) -> str:
    if isinstance(data, str):
        data = data.encode("utf-8")
    return hashlib.sha256(data).hexdigest()


class Reader:
    """Big-endian byte reader that refuses to read past the end."""

    def __init__(self, data: bytes):
        self.data = memoryview(data)
        self.offset = 0

    def take(self, size: int) -> bytes:
        end = self.offset + size
        if size < 0 or end > len(self.data):
            raise ClassFormatError(
                f"truncated class data at offset {self.offset}: need {size} bytes"
            )
        value = self.data[self.offset:end].tobytes()
        self.offset = end
        return value

    def u1(self) -> int:
        return self.take(1)[0]

    def u2(self) -> int:
        return struct.unpack(">H", self.take(2))[0]

    def u4(self) -> int:
        return struct.unpack(">I", self.take(4))[0]


@dataclass
class Attribute:
    name: str
    data: bytes


@dataclass
class FieldSymbol:
    ordinal: int
    name: str
    descriptor: str
    access_flags: int


@dataclass
class MethodSymbol:
    ordinal: int
    name: str
    descriptor: str
    access_flags: int
    max_stack: int | None = None
    max_locals: int | None = None
    code_size: int = 0
    opcode_count: int = 0
    code_sha256: str | None = None
    opcode_sha256: str | None = None
    shape_sha256: str | None = None
    calls: list[str] = field(default_factory=list)  # library/adapter+game callees


@dataclass
class ClassInfo:
    member_path: str
    internal_name: str
    access_flags: int
    super_name: str | None
    interfaces: list[str]
    major_version: int
    minor_version: int
    class_sha256: str
    shape_sha256: str
    fields: list[FieldSymbol]
    methods: list[MethodSymbol]


class ConstantPool:
    def __init__(self, entries: list[Any]):
        self.entries = entries

    def get(self, index: int) -> Any:
        if index <= 0 or index >= len(self.entries) or self.entries[index] is None:
            raise ClassFormatError(f"invalid constant-pool index {index}")
        return self.entries[index]

    def utf8(self, index: int) -> str:
        entry = self.get(index)
        if entry[0] != "Utf8":
            raise ClassFormatError(f"constant {index} is not UTF-8")
        return entry[1]

    def class_name(self, index: int) -> str:
        entry = self.get(index)
        if entry[0] != "Class":
            raise ClassFormatError(f"constant {index} is not a class")
        return self.utf8(entry[1])

    def name_and_type(self, index: int) -> tuple[str, str]:
        entry = self.get(index)
        if entry[0] != "NameAndType":
            raise ClassFormatError(f"constant {index} is not a name-and-type")
        return self.utf8(entry[1]), self.utf8(entry[2])

    def member(self, index: int) -> tuple[str, str, str, str]:
        entry = self.get(index)
        if entry[0] not in {"Fieldref", "Methodref", "InterfaceMethodref"}:
            raise ClassFormatError(f"constant {index} is not a member reference")
        owner = self.class_name(entry[1])
        name, descriptor = self.name_and_type(entry[2])
        return entry[0], owner, name, descriptor

    def literal(self, index: int) -> Any:
        entry = self.get(index)
        if entry[0] == "String":
            return self.utf8(entry[1])
        if entry[0] in {"Integer", "Float", "Long", "Double"}:
            return entry[1]
        if entry[0] == "Class":
            return ("Class", self.class_name(index))
        return (entry[0],)


def parse_constant_pool(reader: Reader) -> ConstantPool:
    count = reader.u2()
    entries: list[Any] = [None] * count
    index = 1
    while index < count:
        tag = reader.u1()
        if tag == 1:
            raw = reader.take(reader.u2())
            entries[index] = ("Utf8", raw.decode("utf-8", errors="surrogateescape"))
        elif tag == 3:
            entries[index] = ("Integer", struct.unpack(">i", reader.take(4))[0])
        elif tag == 4:
            entries[index] = ("Float", struct.unpack(">f", reader.take(4))[0])
        elif tag == 5:
            entries[index] = ("Long", struct.unpack(">q", reader.take(8))[0])
            index += 1  # longs occupy two constant-pool slots
        elif tag == 6:
            entries[index] = ("Double", struct.unpack(">d", reader.take(8))[0])
            index += 1
        elif tag == 7:
            entries[index] = ("Class", reader.u2())
        elif tag == 8:
            entries[index] = ("String", reader.u2())
        elif tag in {9, 10, 11}:
            kind = {9: "Fieldref", 10: "Methodref", 11: "InterfaceMethodref"}[tag]
            entries[index] = (kind, reader.u2(), reader.u2())
        elif tag == 12:
            entries[index] = ("NameAndType", reader.u2(), reader.u2())
        elif tag == 15:
            entries[index] = ("MethodHandle", reader.u1(), reader.u2())
        elif tag == 16:
            entries[index] = ("MethodType", reader.u2())
        elif tag in {17, 18}:
            entries[index] = (
                "Dynamic" if tag == 17 else "InvokeDynamic",
                reader.u2(),
                reader.u2(),
            )
        elif tag in {19, 20}:
            entries[index] = ("Module" if tag == 19 else "Package", reader.u2())
        else:
            raise ClassFormatError(f"unsupported constant-pool tag {tag}")
        index += 1
    return ConstantPool(entries)


def parse_attributes(reader: Reader, pool: ConstantPool) -> list[Attribute]:
    attributes = []
    for _ in range(reader.u2()):
        name = pool.utf8(reader.u2())
        attributes.append(Attribute(name, reader.take(reader.u4())))
    return attributes


def canonical_type(name: str) -> str:
    """Library/adapter classes keep their name; obfuscated game classes fold.

    Game classes in this corpus are obfuscated single tokens with no package
    separator (``a``, ``b``, ``c``, ``d``, ``HG``); an obfuscator may rename
    them per build, so they must not appear in a fingerprint. Anything with a
    ``/`` is a real package (``java/lang``, ``javax/microedition``,
    ``com/nokia/...``) and stays, because those names are stable evidence.
    """
    return name if "/" in name else "<game>"


def canonical_descriptor(descriptor: str) -> str:
    return re.sub(
        r"L([^;]+);",
        lambda match: f"L{canonical_type(match.group(1))};",
        descriptor,
    )


# Number of operand bytes for opcodes with a fixed operand length. Everything
# not listed here has zero operand bytes. tableswitch/lookupswitch/wide are
# handled specially in `instructions`.
FIXED_OPERANDS = {
    16: 1,   # bipush
    17: 2,   # sipush
    18: 1,   # ldc
    19: 2,   # ldc_w
    20: 2,   # ldc2_w
    **{opcode: 1 for opcode in range(21, 26)},   # iload..aload
    **{opcode: 1 for opcode in range(54, 59)},   # istore..astore
    132: 2,  # iinc
    **{opcode: 2 for opcode in range(153, 169)}, # if* / goto / jsr (branch)
    169: 1,  # ret
    **{opcode: 2 for opcode in range(178, 185)}, # get/put static/field, invoke*
    185: 4,  # invokeinterface
    186: 4,  # invokedynamic
    187: 2,  # new
    188: 1,  # newarray
    189: 2,  # anewarray
    192: 2,  # checkcast
    193: 2,  # instanceof
    197: 3,  # multianewarray
    198: 2,  # ifnull
    199: 2,  # ifnonnull
    200: 4,  # goto_w
    201: 4,  # jsr_w
}
CP_U1 = {18}  # ldc: 1-byte pool index
CP_U2 = set(range(178, 185)) | {19, 20, 185, 186, 187, 189, 192, 193, 197}


def instructions(code: bytes) -> Iterable[tuple[int, int, bytes]]:
    """Yield (start_offset, opcode, operand_bytes) for each instruction."""
    offset = 0
    while offset < len(code):
        start = offset
        opcode = code[offset]
        offset += 1
        if opcode == 170:  # tableswitch
            offset += (-offset) % 4
            if offset + 12 > len(code):
                raise ClassFormatError(f"truncated tableswitch at {start}")
            low, high = struct.unpack(">ii", code[offset + 4:offset + 12])
            count = high - low + 1
            if count < 0 or count > len(code):
                raise ClassFormatError(f"invalid tableswitch range at {start}")
            offset += 12 + count * 4
        elif opcode == 171:  # lookupswitch
            offset += (-offset) % 4
            if offset + 8 > len(code):
                raise ClassFormatError(f"truncated lookupswitch at {start}")
            count = struct.unpack(">i", code[offset + 4:offset + 8])[0]
            if count < 0 or count > len(code):
                raise ClassFormatError(f"invalid lookupswitch count at {start}")
            offset += 8 + count * 8
        elif opcode == 196:  # wide
            if offset >= len(code):
                raise ClassFormatError(f"truncated wide at {start}")
            offset += 5 if code[offset] == 132 else 3
        else:
            offset += FIXED_OPERANDS.get(opcode, 0)
        if offset > len(code):
            raise ClassFormatError(f"truncated opcode {opcode:#x} at {start}")
        yield start, opcode, code[start + 1:offset]


RESOURCE_LITERAL = re.compile(
    r"(?:\.(?:png|mid|midi|amr|wav|mdl|lng)$)|^(?:[acdfi]|ldf|mi|Name)$",
    re.IGNORECASE,
)


def canonical_constant(pool: ConstantPool, index: int) -> str:
    entry = pool.get(index)
    kind = entry[0]
    if kind in {"Fieldref", "Methodref", "InterfaceMethodref"}:
        _, owner, name, descriptor = pool.member(index)
        canon_owner = canonical_type(owner)
        # Members of a game class carry obfuscated names too -> fold them.
        canon_name = name if canon_owner != "<game>" else "<member>"
        return f"{kind}:{canon_owner}.{canon_name}:{canonical_descriptor(descriptor)}"
    if kind == "Class":
        return f"Class:{canonical_type(pool.class_name(index))}"
    if kind == "String":
        value = pool.literal(index)
        # Text differs per language; fold to a class so shape is language-blind.
        return "String:resource" if RESOURCE_LITERAL.search(value) else "String:text"
    if kind in {"Integer", "Float", "Long", "Double"}:
        return f"{kind}:{pool.literal(index)!r}"
    return kind


def signed_immediate(opcode: int, operand: bytes) -> int | None:
    if opcode == 16:  # bipush
        return struct.unpack(">b", operand)[0]
    if opcode == 17:  # sipush
        return struct.unpack(">h", operand)[0]
    if 2 <= opcode <= 8:  # iconst_m1..iconst_5
        return opcode - 3
    return None


def analyze_code(code: bytes, pool: ConstantPool, method: MethodSymbol) -> None:
    opcode_bytes = bytearray()
    shape: list[str] = []
    calls: set[str] = set()
    count = 0

    for _, opcode, operand in instructions(code):
        count += 1
        opcode_bytes.append(opcode)
        token = f"{opcode:02x}"
        cp_index = None
        if opcode in CP_U1:
            cp_index = operand[0]
        elif opcode in CP_U2:
            cp_index = struct.unpack(">H", operand[:2])[0]
        if cp_index is not None:
            token += ":" + canonical_constant(pool, cp_index)
            entry = pool.get(cp_index)
            if entry[0] in {"Methodref", "InterfaceMethodref"}:
                _, owner, name, descriptor = pool.member(cp_index)
                calls.add(f"{owner}.{name}:{descriptor}")
        immediate = signed_immediate(opcode, operand)
        if immediate is not None:
            token += f":{immediate}"
        shape.append(token)

    method.code_size = len(code)
    method.opcode_count = count
    method.code_sha256 = sha256(code)
    method.opcode_sha256 = sha256(bytes(opcode_bytes))
    method.shape_sha256 = sha256("\n".join(shape))
    method.calls = sorted(calls)


def parse_code_attribute(data: bytes, pool: ConstantPool, method: MethodSymbol) -> None:
    reader = Reader(data)
    method.max_stack = reader.u2()
    method.max_locals = reader.u2()
    code = reader.take(reader.u4())
    analyze_code(code, pool, method)
    for _ in range(reader.u2()):  # exception table
        reader.take(8)
    parse_attributes(reader, pool)
    if reader.offset != len(reader.data):
        raise ClassFormatError("trailing bytes in Code attribute")


def parse_member(reader: Reader, pool: ConstantPool, ordinal: int, is_method: bool):
    access_flags = reader.u2()
    name = pool.utf8(reader.u2())
    descriptor = pool.utf8(reader.u2())
    attributes = parse_attributes(reader, pool)
    if is_method:
        symbol = MethodSymbol(ordinal, name, descriptor, access_flags)
        code_attributes = [a for a in attributes if a.name == "Code"]
        if len(code_attributes) > 1:
            raise ClassFormatError(f"method {name}{descriptor} has >1 Code attribute")
        if code_attributes:
            parse_code_attribute(code_attributes[0].data, pool, symbol)
        return symbol
    return FieldSymbol(ordinal, name, descriptor, access_flags)


def class_shape(
    super_name: str | None,
    interfaces: list[str],
    fields: list[FieldSymbol],
    methods: list[MethodSymbol],
) -> str:
    """A name-blind structural fingerprint of the whole class.

    Two classes with equal ``shape_sha256`` are the same program element even if
    the obfuscator renamed the class and its members between builds (R10).
    """
    field_tokens = sorted(
        f"{canonical_descriptor(item.descriptor)}:{item.access_flags & 0x005F:04x}"
        for item in fields
    )
    method_tokens = sorted(
        ":".join(
            [
                item.name if item.name in {"<init>", "<clinit>"} else "<method>",
                canonical_descriptor(item.descriptor),
                f"{item.access_flags & 0x0D7F:04x}",
                item.shape_sha256 or "no-code",
            ]
        )
        for item in methods
    )
    shape = {
        "super": canonical_type(super_name) if super_name else None,
        "interfaces": sorted(canonical_type(name) for name in interfaces),
        "fields": field_tokens,
        "methods": method_tokens,
    }
    return sha256(json.dumps(shape, sort_keys=True, separators=(",", ":")))


def parse_class(member_path: str, data: bytes) -> ClassInfo:
    reader = Reader(data)
    if reader.u4() != 0xCAFEBABE:
        raise ClassFormatError(f"{member_path}: invalid class magic")
    minor_version = reader.u2()
    major_version = reader.u2()
    pool = parse_constant_pool(reader)
    access_flags = reader.u2()
    internal_name = pool.class_name(reader.u2())
    super_index = reader.u2()
    super_name = pool.class_name(super_index) if super_index else None
    interfaces = [pool.class_name(reader.u2()) for _ in range(reader.u2())]
    fields = [parse_member(reader, pool, i, False) for i in range(reader.u2())]
    methods = [parse_member(reader, pool, i, True) for i in range(reader.u2())]
    parse_attributes(reader, pool)
    if reader.offset != len(data):
        raise ClassFormatError(f"{member_path}: {len(data) - reader.offset} trailing bytes")
    return ClassInfo(
        member_path=member_path,
        internal_name=internal_name,
        access_flags=access_flags,
        super_name=super_name,
        interfaces=interfaces,
        major_version=major_version,
        minor_version=minor_version,
        class_sha256=sha256(data),
        shape_sha256=class_shape(super_name, interfaces, fields, methods),
        fields=fields,
        methods=methods,
    )

