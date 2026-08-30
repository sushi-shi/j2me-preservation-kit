#!/usr/bin/env python3
"""Rewrite exact UTF-8 constants in a JVM classfile constant pool.

This is useful for disposable oracle adapters when obfuscation produced members
that Java source cannot express, such as two fields with the same name but
different descriptors. It does not alter bytecode or constant-pool indices.
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path


class ClassUtf8Error(ValueError):
    pass


FIXED_PAYLOAD = {
    3: 4,   # Integer
    4: 4,   # Float
    5: 8,   # Long (two slots)
    6: 8,   # Double (two slots)
    7: 2,   # Class
    8: 2,   # String
    9: 4,   # Fieldref
    10: 4,  # Methodref
    11: 4,  # InterfaceMethodref
    12: 4,  # NameAndType
    15: 3,  # MethodHandle
    16: 2,  # MethodType
    17: 4,  # Dynamic
    18: 4,  # InvokeDynamic
    19: 2,  # Module
    20: 2,  # Package
}


def _u2(data: bytes, offset: int) -> int:
    if offset + 2 > len(data):
        raise ClassUtf8Error("truncated classfile")
    return struct.unpack_from(">H", data, offset)[0]


def rewrite_utf8(data: bytes, replacements: dict[bytes, bytes]) -> tuple[bytes, dict[bytes, int]]:
    if len(data) < 10 or data[:4] != b"\xca\xfe\xba\xbe":
        raise ClassUtf8Error("invalid classfile header")
    if not replacements:
        raise ClassUtf8Error("at least one replacement is required")
    if b"" in replacements:
        raise ClassUtf8Error("an empty UTF-8 constant cannot be a replacement key")

    constant_pool_count = _u2(data, 8)
    output = bytearray(data[:10])
    offset = 10
    index = 1
    counts = {old: 0 for old in replacements}

    while index < constant_pool_count:
        if offset >= len(data):
            raise ClassUtf8Error("truncated constant pool")
        tag = data[offset]
        output.append(tag)
        offset += 1

        if tag == 1:
            length = _u2(data, offset)
            offset += 2
            end = offset + length
            if end > len(data):
                raise ClassUtf8Error("truncated CONSTANT_Utf8 payload")
            value = data[offset:end]
            rewritten = replacements.get(value, value)
            if value in replacements:
                counts[value] += 1
            if len(rewritten) > 0xFFFF:
                raise ClassUtf8Error("replacement exceeds CONSTANT_Utf8 length limit")
            output.extend(struct.pack(">H", len(rewritten)))
            output.extend(rewritten)
            offset = end
        elif tag in FIXED_PAYLOAD:
            size = FIXED_PAYLOAD[tag]
            end = offset + size
            if end > len(data):
                raise ClassUtf8Error(f"truncated constant-pool tag {tag}")
            output.extend(data[offset:end])
            offset = end
            if tag in (5, 6):
                index += 1
        else:
            raise ClassUtf8Error(f"unsupported constant-pool tag {tag}")

        index += 1

    missing = [old for old, count in counts.items() if count == 0]
    if missing:
        rendered = ", ".join(repr(value.decode("utf-8", "backslashreplace")) for value in missing)
        raise ClassUtf8Error(f"replacement key(s) absent from constant pool: {rendered}")

    output.extend(data[offset:])
    return bytes(output), counts


def parse_replacement(value: str) -> tuple[bytes, bytes]:
    if "=" not in value:
        raise argparse.ArgumentTypeError("replacement must be OLD=NEW")
    old, new = value.split("=", 1)
    if not old:
        raise argparse.ArgumentTypeError("OLD must not be empty")
    return old.encode("utf-8"), new.encode("utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument(
        "--replace",
        action="append",
        required=True,
        type=parse_replacement,
        metavar="OLD=NEW",
    )
    arguments = parser.parse_args(argv)

    replacements: dict[bytes, bytes] = {}
    for old, new in arguments.replace:
        if old in replacements:
            parser.error(f"duplicate replacement key: {old.decode('utf-8')}")
        replacements[old] = new

    try:
        rewritten, counts = rewrite_utf8(arguments.input.read_bytes(), replacements)
        arguments.output.write_bytes(rewritten)
    except (OSError, ClassUtf8Error) as error:
        print(f"class UTF-8 rewrite failed: {error}", file=sys.stderr)
        return 1

    changes = sum(counts.values())
    print(f"rewrote {changes} constant-pool UTF-8 entr{'y' if changes == 1 else 'ies'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
