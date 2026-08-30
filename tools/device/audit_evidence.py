#!/usr/bin/env python3
"""Emit conservative handset/device evidence directly from original bytecode.

The report is a review queue, never a profile generator. It records the exact
method that touches a device-sensitive API plus constants loaded in that same
method. Constants are deliberately labelled candidates: proving which value
reaches which call still requires bytecode review or an oracle.
"""

from __future__ import annotations

import argparse
import copy
import json
import pathlib
import sys
import zipfile
from io import BytesIO
from typing import Any

CORPUS_DIR = pathlib.Path(__file__).resolve().parents[1] / "corpus"
sys.path.insert(0, str(CORPUS_DIR))

import classfile  # noqa: E402
import classify  # noqa: E402
import corpus  # noqa: E402


VENDOR_PREFIXES = (
    "com/nokia/",
    "com/siemens/",
    "com/motorola/",
    "com/samsung/",
    "com/sony/",
    "com/sonyericsson/",
)


def configure_repo(root: pathlib.Path) -> None:
    """Point the reusable scanner at another generated-game checkout."""
    resolved = root.resolve()
    corpus.REPO = resolved
    corpus.BUILDS_TOML = resolved / "java" / "reconstruction" / "builds.toml"
    corpus.ORIGINALS = resolved / "_originals"
    classify.REPO = resolved

# kind, profile axis, exact call prefix
CALL_TRIGGERS = (
    ("canvas-game-action", "input", "javax/microedition/lcdui/Canvas.getGameAction:"),
    ("default-string-encoding", "system", "java/lang/String.getBytes:()[B"),
    ("system-property", "system", "java/lang/System.getProperty:"),
    ("connector-open", "connector", "javax/microedition/io/Connector.open:"),
    ("media-player", "media", "javax/microedition/media/Manager.createPlayer:"),
    ("media-capability", "media", "javax/microedition/media/Manager.getSupported"),
    ("media-control", "media", "javax/microedition/media/Player.getControl:"),
    ("vibration", "haptics", "javax/microedition/lcdui/Display.vibrate:"),
    ("rms-capacity", "rms", "javax/microedition/rms/RecordStore.getSizeAvailable:"),
    ("platform-request", "lifecycle", "javax/microedition/midlet/MIDlet.platformRequest:"),
    ("pointer-capability", "input", "javax/microedition/lcdui/Canvas.hasPointer"),
    ("repeat-capability", "input", "javax/microedition/lcdui/Canvas.hasRepeatEvents:"),
)

CALLBACK_TRIGGERS = {
    "keyPressed": ("key-callback", "input"),
    "keyReleased": ("key-callback", "input"),
    "keyRepeated": ("repeat-callback", "input"),
    "pointerPressed": ("pointer-callback", "input"),
    "pointerReleased": ("pointer-callback", "input"),
    "pointerDragged": ("pointer-callback", "input"),
    "sizeChanged": ("canvas-resize-callback", "display"),
    "hideNotify": ("canvas-lifecycle-callback", "lifecycle"),
    "showNotify": ("canvas-lifecycle-callback", "lifecycle"),
}


def _method_candidates(method: classfile.MethodSymbol) -> tuple[list[str], list[int | float]]:
    strings = [value for value in method.loaded_constants if isinstance(value, str)]
    numbers = [
        value
        for value in method.loaded_constants
        if isinstance(value, (int, float)) and not isinstance(value, bool)
    ]
    for value in method.numeric_immediates:
        if value not in numbers:
            numbers.append(value)
    return strings, numbers


def _class_candidates(info: classfile.ClassInfo) -> tuple[list[str], list[int | float]]:
    strings: list[str] = []
    numbers: list[int | float] = []
    for method in sorted(info.methods, key=lambda item: item.ordinal):
        method_strings, method_numbers = _method_candidates(method)
        for value in method_strings:
            if value not in strings:
                strings.append(value)
        for value in method_numbers:
            if value not in numbers:
                numbers.append(value)
    return strings, numbers


def method_findings(
    owner: str, method: classfile.MethodSymbol
) -> list[dict[str, Any]]:
    """Return call-scope candidates for one method, without dataflow claims."""
    strings, numbers = _method_candidates(method)
    common = {
        "class": owner,
        "method": method.name,
        "descriptor": method.descriptor,
        "string_candidates": strings,
        "numeric_candidates": numbers,
    }
    findings: list[dict[str, Any]] = []

    for kind, axis, prefix in CALL_TRIGGERS:
        matched = [call for call in method.calls if call.startswith(prefix)]
        if matched:
            findings.append({**common, "kind": kind, "axis": axis, "calls": matched})

    vendor_calls = [
        call for call in method.calls if call.startswith(VENDOR_PREFIXES)
    ]
    if vendor_calls:
        findings.append(
            {**common, "kind": "vendor-api", "axis": "vendor", "calls": vendor_calls}
        )

    callback = CALLBACK_TRIGGERS.get(method.name)
    if callback is not None:
        kind, axis = callback
        findings.append({**common, "kind": kind, "axis": axis, "calls": []})

    return findings


def parse_descriptor(data: bytes) -> dict[str, str]:
    """Parse manifest/JAD continuation lines without interpreting values."""
    text = data.decode("utf-8", errors="surrogateescape")
    unfolded: list[str] = []
    for raw in text.replace("\r\n", "\n").replace("\r", "\n").split("\n"):
        if raw.startswith(" ") and unfolded:
            unfolded[-1] += raw[1:]
        else:
            unfolded.append(raw)
    result: dict[str, str] = {}
    for line in unfolded:
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        result[key.strip()] = value.lstrip()
    return result


def header_findings(attributes: dict[str, str]) -> list[dict[str, str]]:
    findings = []
    for key, value in attributes.items():
        lowered = key.lower()
        if lowered.startswith(
            ("nokia-", "siemens-", "motorola-", "samsung-", "sony-ericsson-", "sonyericsson-")
        ):
            axis = "vendor"
        elif lowered in {"microedition-configuration", "microedition-profile"}:
            axis = "system"
        elif lowered == "midlet-data-size":
            axis = "rms"
        elif lowered in {"midlet-install-notify", "midlet-delete-notify"}:
            axis = "connector"
        else:
            continue
        findings.append({"axis": axis, "key": key, "value": value})
    return findings


def jar_manifest_findings(payload: bytes) -> list[dict[str, str]]:
    with zipfile.ZipFile(BytesIO(payload)) as jar:
        members = {name.upper(): name for name in jar.namelist()}
        name = members.get("META-INF/MANIFEST.MF")
        if name is None:
            return []
        return header_findings(parse_descriptor(jar.read(name)))


def analyze() -> dict[str, Any]:
    builds = []
    for build in classify.analyze():
        findings = []
        class_candidates = []
        for info in sorted(build.game_classes, key=lambda item: item.internal_name):
            class_findings = [
                finding
                for method in sorted(info.methods, key=lambda item: item.ordinal)
                for finding in method_findings(info.internal_name, method)
            ]
            if not class_findings:
                continue
            strings, numbers = _class_candidates(info)
            class_candidates.append(
                {
                    "class": info.internal_name,
                    "string_candidates": strings,
                    "numeric_candidates": numbers,
                }
            )
            findings.extend(class_findings)
        builds.append(
            {
                "build_id": build.build_id,
                "sha256": build.sha256,
                "manifest": jar_manifest_findings(build.payload),
                "classes": class_candidates,
                "methods": findings,
            }
        )

    descriptors = []
    manifest = corpus.load_manifest()
    for collection in ("payload", "archived"):
        for entry in manifest.get(collection, []):
            if entry.get("kind") != "jad":
                continue
            data = corpus.payload_bytes(entry)
            descriptors.append(
                {
                    "id": entry["id"],
                    "sha256": entry["sha256"],
                    "headers": header_findings(parse_descriptor(data)),
                }
            )

    return {
        "schema_version": 1,
        "policy": (
            "Review queue only: method constants are call-scope candidates and class "
            "constants are class-scope candidates, not proven call arguments; this "
            "report never selects a vendor or profile."
        ),
        "builds": builds,
        "jads": sorted(descriptors, key=lambda item: item["id"]),
    }


def self_test() -> int:
    method = classfile.MethodSymbol(
        ordinal=0,
        name="keyPressed",
        descriptor="(I)V",
        access_flags=4,
        calls=[
            "java/lang/String.getBytes:()[B",
            "java/lang/System.getProperty:(Ljava/lang/String;)Ljava/lang/String;",
            "javax/microedition/io/Connector.open:(Ljava/lang/String;)Ljavax/microedition/io/Connection;",
            "javax/microedition/lcdui/Canvas.getGameAction:(I)I",
        ],
        loaded_constants=["wireless.messaging.sms.smsc", "sms://"],
        numeric_immediates=[-6, -7, 21, 22],
    )
    clean = method_findings("FixtureCanvas", method)
    kinds = {finding["kind"] for finding in clean}
    expected = {
        "canvas-game-action",
        "default-string-encoding",
        "system-property",
        "connector-open",
        "key-callback",
    }
    if kinds != expected:
        print(f"device-evidence self-test fixture mismatch: {sorted(kinds)}", file=sys.stderr)
        return 1

    broken = copy.deepcopy(method)
    broken.calls = [
        call for call in broken.calls if not call.startswith("java/lang/System.getProperty:")
    ]
    broken_kinds = {finding["kind"] for finding in method_findings("FixtureCanvas", broken)}
    if "system-property" in broken_kinds or len(broken_kinds) != len(kinds) - 1:
        print("device-evidence self-test did not detect a removed API edge", file=sys.stderr)
        return 1

    print("device-evidence self-test: removing one device-sensitive call removed exactly one finding")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo",
        type=pathlib.Path,
        default=corpus.REPO,
        help="generated-game repository to inspect (default: this checkout)",
    )
    parser.add_argument("--out", type=pathlib.Path)
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args()
    if arguments.self_test:
        return self_test()

    configure_repo(arguments.repo)
    output = arguments.out or corpus.REPO / "_reference" / "device-evidence.json"
    report = analyze()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(report, sort_keys=True, indent=2, ensure_ascii=True) + "\n",
        encoding="utf-8",
    )
    method_count = sum(len(build["methods"]) for build in report["builds"])
    header_count = sum(len(build["manifest"]) for build in report["builds"])
    header_count += sum(len(jad["headers"]) for jad in report["jads"])
    print(
        f"device-evidence: {method_count} method finding(s), {header_count} descriptor "
        f"finding(s); wrote {output}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
