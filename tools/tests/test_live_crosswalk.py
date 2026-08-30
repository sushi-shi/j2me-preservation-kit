from __future__ import annotations

import unittest

from tools.ast.live_crosswalk import (
    BodySpec,
    LiveCrosswalkError,
    RustTarget,
    require_spec_keys,
)


def spec(key: str) -> BodySpec:
    return BodySpec(
        key=key,
        original_owner="a",
        original_name="m",
        descriptor="()V",
        java_source="Game.java",
        java_owner="Game",
        java_item="method()",
        rust=(RustTarget("game.rs", "fn:method"),),
    )


class LiveCrosswalkTests(unittest.TestCase):
    def test_requires_exact_ordered_manifest_closure(self) -> None:
        require_spec_keys(
            {"body": [{"java_item": "a"}, {"java_item": "b"}]},
            (spec("a"), spec("b")),
        )
        with self.assertRaisesRegex(LiveCrosswalkError, "exactly match"):
            require_spec_keys(
                {"body": [{"java_item": "b"}, {"java_item": "a"}]},
                (spec("a"), spec("b")),
            )

    def test_rejects_an_unconfigured_manifest_row(self) -> None:
        with self.assertRaisesRegex(LiveCrosswalkError, "exactly match"):
            require_spec_keys(
                {"body": [{"java_item": "a"}, {"java_item": "extra"}]},
                (spec("a"),),
            )


if __name__ == "__main__":
    unittest.main()
