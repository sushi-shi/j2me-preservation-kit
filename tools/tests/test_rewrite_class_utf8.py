from __future__ import annotations

import struct
import unittest

from tools.java.rewrite_class_utf8 import ClassUtf8Error, rewrite_utf8


def utf8(value: bytes) -> bytes:
    return b"\x01" + struct.pack(">H", len(value)) + value


class RewriteClassUtf8Tests(unittest.TestCase):
    def fixture(self) -> bytes:
        # Six constant-pool slots: Utf8, Class, Utf8, Long + reserved slot.
        pool = (
            utf8(b"canvas")
            + b"\x07\x00\x01"
            + utf8(b"startTime")
            + b"\x05\x00\x00\x00\x00\x00\x00\x00\x07"
        )
        return (
            b"\xca\xfe\xba\xbe"
            + b"\x00\x00\x00\x34"
            + b"\x00\x06"
            + pool
            + b"TAIL"
        )

    def test_rewrites_variable_length_constants_without_moving_indices(self) -> None:
        rewritten, counts = rewrite_utf8(
            self.fixture(), {b"canvas": b"a", b"startTime": b"a"}
        )
        self.assertEqual(counts, {b"canvas": 1, b"startTime": 1})
        self.assertIn(utf8(b"a") + b"\x07\x00\x01" + utf8(b"a"), rewritten)
        self.assertTrue(rewritten.endswith(b"TAIL"))

    def test_requires_every_requested_symbol_to_exist(self) -> None:
        with self.assertRaisesRegex(ClassUtf8Error, "absent"):
            rewrite_utf8(self.fixture(), {b"missing": b"a"})


if __name__ == "__main__":
    unittest.main()
