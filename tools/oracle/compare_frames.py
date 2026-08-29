#!/usr/bin/env python3
"""Compare Java ME reference frames with the future Rust port's -- EXACT, per label.

This game is pure 2D LCDUI (javax.microedition.lcdui.Graphics). Both the
reference runtime (FreeJ2ME-Plus, driven by tools/oracle/HeadlessCapture.java)
and the future Rust port are driven by the *same* route files in
tools/oracle/routes, and both write one frame per ``shot`` label, so frames pair
by label rather than through a hand-maintained table. Because there is no 3D and
no lighting to disagree about, **every frame is compared exactly**:
``differing_pixels == 0`` is the one clean state. Anything else is a real defect,
not a tolerance.

Provenance: this is the 2D-only descendant of stalker-mobile's
tools/transliteration/compare_java_me_frames.py. The PNG codec, the route reader,
the fail-closed capture-provenance check, the cross-pass stability check and the
ratchet are lifted near-verbatim; the entire 3D structural-attribution machinery
(texture palettes, angular attribution, boundary agreement, overlay masks) is
deleted because a pure-2D oracle never needs it.

Blind spots this tool is deliberately built against
---------------------------------------------------
These are the ways the *sibling* oracle went green over real bugs. Each has a
countermeasure wired in here; see docs/ORACLE.md.

1. **Vacuous comparator.** The sibling once scored a *wholly different* image as
   "6 pixels differ" because it shelled out to ``magick compare -metric AE`` and
   misparsed the result -- every golden was vacuous for weeks. So this tool
   computes the diff **in-process**: it decodes both PNGs itself and compares the
   raw pixel arrays directly (``compare_exact``). No external image tool is ever
   invoked. And it does not *trust* that code: on every run it runs an **inline
   self-test** -- ref-vs-ref must be 0, ref-vs-one-perturbed-pixel must be >0, and
   two genuinely different reference labels must differ by *many* pixels -- and
   fails the whole run if any of those misbehave.

2. **Stale / unverified captures.** The sibling's captures silently answered
   inputs that no longer existed because nobody read the provenance manifest. So
   this tool SHA-256s the jar, every route file and the emulator build recorded
   in the capture manifest and **fails closed** on any drift from the repository
   as it is now, before comparing a single pixel.

3. **The reference is not ground truth.** The sibling's emulator booted with the
   wrong key for ENABLE_SOUND, so every sound play was silently refused and a
   whole capture was a frozen 0-vs-0. So this tool checks the reference frames it
   is about to trust actually *exercised* the game: each is non-blank (a
   distinct-colour floor) and the frames within a route are not all identical
   (the sequence advanced -- it is not a frozen boot screen). (This game only
   boots at all under ``--sound 1``; see docs/ORACLE.md.)

Usage
-----
    tools/oracle/compare_frames.py --self-test        # prove the oracle bites; no port needed
    tools/oracle/compare_frames.py                    # full compare (needs a port capture)
    tools/oracle/compare_frames.py --update-ratchet   # record port agreement after review
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import pathlib
import struct
import sys
import zlib
from collections import Counter
from dataclasses import dataclass, field

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]


def _oracle_config(root: pathlib.Path) -> dict:
    """The per-game [oracle] knobs from game.toml -- this comparator holds no
    game-specific literal, so it stamps into any 2D J2ME port unchanged."""
    try:
        import tomllib
    except ModuleNotFoundError:  # pragma: no cover - Python < 3.11
        import tomli as tomllib  # type: ignore
    with (root / "game.toml").open("rb") as fh:
        return tomllib.load(fh).get("oracle", {})


# The pinned JAR is per game and lives in game.toml [oracle].jar (relative to the
# repo root); the port's capture must be compared against the same archive.
DEFAULT_JAR = REPO_ROOT / _oracle_config(REPO_ROOT)["jar"]
ROUTE_DIR = REPO_ROOT / "tools/oracle/routes"
RATCHET = REPO_ROOT / "tools/oracle/agreement.toml"
PATCH_DIR = REPO_ROOT / "tools/oracle/patches"

# A captured reference frame must carry at least this many distinct colours, or
# it is treated as blank/frozen and cannot serve as a reference (blind spot #3/#4).
MINIMUM_COLOURS = 16

# When the inline self-test compares two *different* reference labels, they must
# differ by at least this fraction of the canvas -- the direct guard against the
# "wholly different image scored as a tiny diff" failure (blind spot #1).
DISTINCT_LABEL_MIN_FRACTION = 0.02


# ---------------------------------------------------------------------------
# PNG  (dependency-free: the devshell has neither Pillow nor numpy, and a gate
# that needs an extra package is a gate that stops running)
# ---------------------------------------------------------------------------


@dataclass
class Image:
    width: int
    height: int
    # One 0xRRGGBB integer per pixel, row-major. Alpha is dropped: both runtimes
    # composite into one opaque LCDUI surface, so alpha carries no information.
    pixels: list


def _paeth(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    return b if pb <= pc else c


def read_png(path: pathlib.Path) -> Image:
    return decode_png(path.read_bytes(), str(path))


def decode_png(data: bytes, name: str) -> Image:
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{name}: not a PNG")
    pos = 8
    width = height = depth = colour = 0
    idat = bytearray()
    palette: list = []
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            width, height, depth, colour, _, _, interlace = struct.unpack(">IIBBBBB", body)
            if depth != 8:
                raise ValueError(f"{name}: bit depth {depth} is not read (captures are 8-bit)")
            if interlace:
                raise ValueError(f"{name}: interlaced PNGs are not read")
        elif kind == b"PLTE":
            palette = [(body[i], body[i + 1], body[i + 2]) for i in range(0, len(body), 3)]
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break

    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[colour]
    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    rows = []
    previous = bytearray(stride)
    offset = 0
    for _ in range(height):
        filter_type = raw[offset]
        offset += 1
        line = bytearray(raw[offset : offset + stride])
        offset += stride
        if filter_type == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 0xFF
        elif filter_type == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 0xFF
        elif filter_type == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
        elif filter_type == 4:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                upper_left = previous[i - channels] if i >= channels else 0
                line[i] = (line[i] + _paeth(left, previous[i], upper_left)) & 0xFF
        elif filter_type != 0:
            raise ValueError(f"{name}: unknown PNG filter {filter_type}")
        rows.append(line)
        previous = line

    samples: list = []
    for line in rows:
        samples.extend(line[: width * channels])

    pixels = []
    if colour == 2:
        for i in range(0, len(samples), 3):
            pixels.append((samples[i] << 16) | (samples[i + 1] << 8) | samples[i + 2])
    elif colour == 6:
        for i in range(0, len(samples), 4):
            pixels.append((samples[i] << 16) | (samples[i + 1] << 8) | samples[i + 2])
    elif colour == 0:
        for value in samples:
            pixels.append((value << 16) | (value << 8) | value)
    elif colour == 4:
        for i in range(0, len(samples), 2):
            value = samples[i]
            pixels.append((value << 16) | (value << 8) | value)
    elif colour == 3:
        for index in samples:
            red, green, blue = palette[index]
            pixels.append((red << 16) | (green << 8) | blue)
    else:
        raise ValueError(f"{name}: unsupported colour type {colour}")
    return Image(width, height, pixels)


def write_png(path: pathlib.Path, width: int, height: int, pixels: list) -> None:
    raw = bytearray()
    for row in range(height):
        raw.append(0)
        for value in pixels[row * width : (row + 1) * width]:
            raw += bytes(((value >> 16) & 0xFF, (value >> 8) & 0xFF, value & 0xFF))

    def chunk(kind: bytes, body: bytes) -> bytes:
        return (
            struct.pack(">I", len(body))
            + kind
            + body
            + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
        )

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def distinct_colours(image: Image) -> int:
    return len(set(image.pixels))


# ---------------------------------------------------------------------------
# in-process exact comparison  (NEVER shells out to an image tool)
# ---------------------------------------------------------------------------


def compare_exact(left: Image, right: Image) -> dict:
    """Count differing pixels by walking the two raw pixel arrays directly.

    This is the whole comparator. It never invokes magick/ImageMagick or any
    external proxy -- the sibling oracle's weeks of vacuous goldens came from
    exactly that. `differing_pixels == 0` is the only clean state.
    """
    if (left.width, left.height) != (right.width, right.height):
        raise ValueError(
            f"canvas mismatch {left.width}x{left.height} vs {right.width}x{right.height}"
        )
    differing = 0
    squared = 0
    for a, b in zip(left.pixels, right.pixels):
        if a == b:
            continue
        differing += 1
        for shift in (16, 8, 0):
            delta = ((a >> shift) & 0xFF) - ((b >> shift) & 0xFF)
            squared += delta * delta
    total = len(left.pixels) or 1
    return {
        "differing_pixels": differing,
        "differing_fraction": round(differing / total, 6),
        "rmse": round(math.sqrt(squared / (total * 3)), 4),
    }


# ---------------------------------------------------------------------------
# routes
# ---------------------------------------------------------------------------


@dataclass
class Shot:
    label: str
    route: str
    layer: str = "2d"

    @property
    def key(self) -> str:
        return f"{self.route}/{self.label}"


def read_routes(route_dir: pathlib.Path, only: set | None = None) -> list:
    shots = []
    for path in sorted(route_dir.glob("*.txt")):
        route = path.stem
        if only and route not in only:
            continue
        for raw in path.read_text().splitlines():
            line = raw.strip()
            if not line.startswith("shot"):
                continue
            tokens = line.split()
            named = {}
            positional = []
            for token in tokens[1:]:
                if "=" in token:
                    key, _, value = token.partition("=")
                    named[key] = value
                else:
                    positional.append(token)
            shots.append(Shot(label=positional[0], route=route, layer=named.get("layer", "2d")))
    return shots


# ---------------------------------------------------------------------------
# capture provenance  (fail-closed -- blind spot #2)
# ---------------------------------------------------------------------------


def sha256_of(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def read_manifest(path: pathlib.Path) -> dict:
    rows: dict = {}
    for line in path.read_text().splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        key, *rest = line.split("\t")
        if not rest:
            continue
        rows.setdefault(key.strip(), []).append(tuple(part.strip() for part in rest))
    return rows


def single(rows: dict, key: str) -> str | None:
    values = rows.get(key)
    if not values or len(values[0]) != 1:
        return None
    return values[0][0]


def check_capture_provenance(
    roots: dict, jar: pathlib.Path, routes: set, expected_passes: dict, require_patches: bool
) -> list:
    """Verify each capture root's manifest against the repository as it is now.

    A recorded provenance nobody verifies is worse than none, because it reads as
    evidence. So this is fail-closed: a missing or disagreeing manifest stops the
    run before any frame is compared.
    """
    failures: list = []
    manifests: dict = {}
    for side, root in roots.items():
        path = root / "manifest.tsv"
        if not path.is_file():
            failures.append(
                f"{side}: no manifest.tsv under {root}; captures without recorded "
                "provenance cannot be shown to match these routes -- recapture with "
                f"{'tools/oracle/capture_reference.sh' if side == 'reference' else 'the port capture'}"
            )
            continue
        manifests[side] = read_manifest(path)
    if failures:
        return failures

    jar_digest = sha256_of(jar) if jar.is_file() else None
    for side, rows in manifests.items():
        recorded = single(rows, "jar_sha256")
        if recorded is None:
            failures.append(f"{side}: manifest records no jar_sha256")
        elif jar_digest is not None and recorded != jar_digest:
            failures.append(
                f"{side}: captured from archive {recorded[:12]}, comparing against "
                f"{jar_digest[:12]} ({jar.name})"
            )

    # The keystrokes -- the suite's central claim.
    for side, rows in manifests.items():
        recorded = {name: digest for name, digest in rows.get("route_sha256", []) if digest}
        if not recorded:
            failures.append(
                f"{side}: manifest records no route_sha256 rows, so nothing proves "
                "these frames came from the routes being compared"
            )
            continue
        for route in sorted(routes):
            source = ROUTE_DIR / f"{route}.txt"
            if not source.is_file():
                failures.append(f"{route}: no route file at {source}")
                continue
            current = sha256_of(source)
            if route not in recorded:
                failures.append(f"{side}: {route} was never captured (no route_sha256 row)")
            elif recorded[route] != current:
                failures.append(
                    f"{side}: {route} was captured from route {recorded[route][:12]}, the "
                    f"route on disk is {current[:12]} -- these frames answer different keystrokes"
                )

    # The emulator build (reference side only).
    reference = manifests.get("reference", {})
    if reference:
        if single(reference, "emulator_commit") is None:
            failures.append("reference: manifest records no emulator_commit")
        if require_patches:
            recorded_patches = {
                pathlib.PurePosixPath(name).name: digest
                for name, digest in reference.get("emulator_patch", [])
                if digest
            }
            current_patches = {p.name: sha256_of(p) for p in sorted(PATCH_DIR.glob("*.patch"))}
            if recorded_patches != current_patches:
                added = sorted(set(current_patches) - set(recorded_patches))
                removed = sorted(set(recorded_patches) - set(current_patches))
                changed = sorted(
                    n
                    for n in set(recorded_patches) & set(current_patches)
                    if recorded_patches[n] != current_patches[n]
                )
                failures.append(
                    "reference: the captures were made with a different patched emulator; "
                    f"applied since={added}, no longer applied={removed}, edited={changed}"
                )

    for side, rows in manifests.items():
        recorded = single(rows, "passes")
        found = expected_passes.get(side, 0)
        if recorded is None:
            failures.append(f"{side}: manifest records no pass count")
        elif not recorded.isdigit() or int(recorded) != found:
            failures.append(f"{side}: manifest records {recorded} passes, {found} are on disk")
    return failures


# ---------------------------------------------------------------------------
# capture directories
# ---------------------------------------------------------------------------


def passes_of(root: pathlib.Path) -> list:
    if not root.exists():
        return []
    return sorted(root.glob("pass-*"), key=lambda path: int(path.name.split("-")[1]))


def frame_path(pass_dir: pathlib.Path, shot: Shot) -> pathlib.Path:
    return pass_dir / shot.route / f"{shot.label}.png"


def check_stability(root: pathlib.Path, shots: list) -> tuple:
    """Every pass must produce the same bytes for the same label.

    Averaging an unstable frame away would hide exactly the class of defect this
    suite exists to find, so instability is a failure of its own. The reference
    runs on wall time and steps a substituted clock; this is where that
    determinism is actually tested rather than assumed.
    """
    unstable = []
    missing = []
    dirs = passes_of(root)
    if len(dirs) < 2:
        return unstable, missing
    for shot in shots:
        digests = []
        for pass_dir in dirs:
            path = frame_path(pass_dir, shot)
            if path.exists():
                digests.append((pass_dir.name, f"{zlib.crc32(path.read_bytes()):08x}"))
            else:
                missing.append((shot.key, pass_dir.name))
        if len({digest for _, digest in digests}) > 1:
            unstable.append((shot.key, digests))
    return unstable, missing


# ---------------------------------------------------------------------------
# inline self-test  (blind spot #1 -- runs on EVERY invocation)
# ---------------------------------------------------------------------------


def inline_self_test(reference_pass: pathlib.Path, shots: list) -> list:
    """Prove the in-process comparator actually reacts, on this run's own frames.

    For each reference label:
      * ref vs itself           == 0   (a clean compare is reachable)
      * ref vs one flipped pixel == 1   (a one-pixel defect is detected, not masked)
    And across labels:
      * two genuinely different labels differ by many pixels, not ~0
        (the exact bug that made the sibling's goldens vacuous for weeks)

    Any deviation fails the whole run. This is not a separate optional test; it
    is asserted inline every time, so a comparator that has quietly gone blind
    cannot pass a real comparison in the same process.
    """
    failures: list = []
    loaded: dict = {}
    for shot in shots:
        path = frame_path(reference_pass, shot)
        if not path.exists():
            continue
        image = read_png(path)
        loaded[shot.key] = image

        same = compare_exact(image, image)["differing_pixels"]
        if same != 0:
            failures.append(f"self-test: {shot.key} vs itself reports {same} != 0")

        perturbed = Image(image.width, image.height, list(image.pixels))
        perturbed.pixels[0] ^= 0x000001  # flip one bit of one pixel
        one = compare_exact(image, perturbed)["differing_pixels"]
        if one != 1:
            failures.append(
                f"self-test: {shot.key} vs one-pixel-perturbed reports {one}, expected 1"
            )

    # Distinct-label sanity: find two labels whose frames are actually different
    # and confirm the comparator sees a large diff, not a vacuous one.
    keys = list(loaded)
    floor = None
    best = None
    if len(keys) >= 2:
        for i in range(len(keys)):
            for j in range(i + 1, len(keys)):
                a, b = loaded[keys[i]], loaded[keys[j]]
                if (a.width, a.height) != (b.width, b.height):
                    continue
                metrics = compare_exact(a, b)
                if best is None or metrics["differing_pixels"] > best[2]:
                    best = (keys[i], keys[j], metrics["differing_pixels"])
        if best is None or best[2] == 0:
            failures.append(
                "self-test: no two reference labels differ at all -- the captures are "
                "all identical, so a real comparison could never fail"
            )
        else:
            total = loaded[best[0]].width * loaded[best[0]].height
            floor = int(total * DISTINCT_LABEL_MIN_FRACTION)
            if best[2] < floor:
                failures.append(
                    f"self-test: the most-different reference labels {best[0]} and {best[1]} "
                    f"differ by only {best[2]} px (< {floor}); the comparator may be scoring "
                    "different images as near-identical (the vacuous-comparator bug)"
                )
    if not failures:
        detail = f"{len(loaded)} labels; one-pixel diff detected"
        if floor is not None and best is not None:
            detail += f"; most-different pair differs by {best[2]} px (floor {floor})"
        print(f"self-test : PASS ({detail})")
    return failures


def non_vacuity_and_liveness(reference_pass: pathlib.Path, shots: list, floor: int) -> list:
    """Every reference frame is non-blank, and each route's frames advanced.

    Guards blind spots #3/#4: a blank or frozen capture must not read as a valid
    reference. A frame below the colour floor is blank; a route whose frames are
    all byte-identical never left its first screen.
    """
    failures: list = []
    by_route: dict = {}
    for shot in shots:
        path = frame_path(reference_pass, shot)
        if not path.exists():
            failures.append(f"non-vacuity: {shot.key} has no reference frame at {path}")
            continue
        image = read_png(path)
        colours = distinct_colours(image)
        if colours < floor:
            failures.append(
                f"non-vacuity: {shot.key} has only {colours} distinct colours (floor {floor}); "
                "a blank/frozen frame cannot be a reference"
            )
        by_route.setdefault(shot.route, []).append(f"{zlib.crc32(path.read_bytes()):08x}")
    for route, digests in by_route.items():
        if len(digests) >= 2 and len(set(digests)) == 1:
            failures.append(
                f"liveness: every frame in route {route} is byte-identical -- the game never "
                "advanced past its first screen (a frozen capture)"
            )
    if not failures:
        print(
            f"non-vacuity: PASS ({len(shots)} frames >= {floor} colours; "
            f"{len(by_route)} route(s) advanced)"
        )
    return failures


# ---------------------------------------------------------------------------
# diff image (3 panels: reference | port | amplified difference)
# ---------------------------------------------------------------------------

PANEL_GAP = 4


def write_diff(path: pathlib.Path, reference: Image, port: Image) -> None:
    width, height = reference.width, reference.height
    panels = 3
    out_width = width * panels + PANEL_GAP * (panels - 1)
    canvas = [0x101010] * (out_width * height)

    def blit(panel: int, pixels: list) -> None:
        x0 = panel * (width + PANEL_GAP)
        for y in range(height):
            row = y * out_width + x0
            canvas[row : row + width] = pixels[y * width : (y + 1) * width]

    blit(0, reference.pixels)
    blit(1, port.pixels)
    amplified = []
    for a, b in zip(reference.pixels, port.pixels):
        value = 0
        for shift in (16, 8, 0):
            delta = abs(((a >> shift) & 0xFF) - ((b >> shift) & 0xFF))
            value |= min(255, delta * 4) << shift
        amplified.append(value)
    blit(2, amplified)
    write_png(path, out_width, height, canvas)


# ---------------------------------------------------------------------------
# ratchet
# ---------------------------------------------------------------------------


def load_ratchet(path: pathlib.Path) -> dict:
    if not path.exists():
        return {}
    import tomllib

    return tomllib.loads(path.read_text()).get("shots", {})


def write_ratchet(path: pathlib.Path, results: list) -> None:
    lines = [
        "# Per-label EXACT agreement between FreeJ2ME-Plus (reference) and this port.",
        "#",
        "# Regenerate with `tools/oracle/compare_frames.py --update-ratchet`, and only",
        "# after looking at every diff image the run wrote. A run fails on a regression",
        "# AND on an unrecorded improvement, so a number here can only move deliberately.",
        "# Never raise one to make a change pass.",
        "#",
        "# Every label is `2d`: `differing_pixels = 0` is the only clean state, and any",
        "# other value is a recorded defect, not a tolerance. `verdict = port-missing`",
        "# means the port has not been captured for that label yet.",
        "",
    ]
    entries = {}
    for result in results:
        entry = {"layer": result.layer, "verdict": result.verdict}
        if result.verdict == "compared":
            entry["differing_pixels"] = result.metrics.get("differing_pixels", 0)
        entries[result.key] = entry
    for key, entry in sorted(entries.items()):
        lines.append(f'[shots."{key}"]')
        lines.append(f'layer = "{entry["layer"]}"')
        lines.append(f'verdict = "{entry["verdict"]}"')
        if entry["verdict"] == "compared":
            lines.append(f'differing_pixels = {entry["differing_pixels"]}')
        lines.append("")
    path.write_text("\n".join(lines))


# ---------------------------------------------------------------------------
# result
# ---------------------------------------------------------------------------


@dataclass
class Result:
    label: str
    route: str
    layer: str
    verdict: str  # compared | port-missing | reference-missing
    ok: bool = True
    detail: str = ""
    metrics: dict = field(default_factory=dict)
    diff_image: str = ""

    @property
    def key(self) -> str:
        return f"{self.route}/{self.label}"


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--reference", default=str(REPO_ROOT / "_reference/oracle/reference"))
    parser.add_argument("--port", default=str(REPO_ROOT / "_reference/oracle/port"))
    parser.add_argument("--out", default=str(REPO_ROOT / "_reference/oracle/diff"))
    parser.add_argument("--jar", default=str(DEFAULT_JAR))
    parser.add_argument("--routes", default="", help="comma-separated route stems")
    parser.add_argument("--minimum-colours", type=int, default=MINIMUM_COLOURS)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="verify the reference provenance, run the inline comparator self-test and the "
        "non-vacuity/liveness floors, then stop -- proves the oracle bites without a port",
    )
    parser.add_argument("--update-ratchet", action="store_true")
    parser.add_argument("--allow-single-pass", action="store_true")
    parser.add_argument("--json", default="")
    options = parser.parse_args()

    reference_root = pathlib.Path(options.reference)
    port_root = pathlib.Path(options.port)
    out_root = pathlib.Path(options.out)
    only = set(filter(None, options.routes.split(","))) or None

    shots = read_routes(ROUTE_DIR, only)
    if not shots:
        print("no shots in the selected routes", file=sys.stderr)
        return 2

    failures: list = []
    print(f"routes    : {len({shot.route for shot in shots})}")
    print(f"labels    : {len(shots)}")

    # ------------------------------------------------------ reference must exist
    reference_passes = passes_of(reference_root)
    if not reference_passes:
        print(f"reference : no captures under {reference_root}", file=sys.stderr)
        print(
            "\nCapture the reference first: `just java-me-frames`.",
            file=sys.stderr,
        )
        return 2
    reference_pass = reference_passes[0]

    have_port = bool(passes_of(port_root))

    # ---------------------------------------------------------- provenance (#2)
    roots = {"reference": reference_root}
    if have_port:
        roots["port"] = port_root
    provenance = check_capture_provenance(
        roots,
        pathlib.Path(options.jar),
        {shot.route for shot in shots},
        {
            "reference": len(reference_passes),
            "port": len(passes_of(port_root)),
        },
        require_patches=True,
    )
    if provenance:
        print("capture provenance does not match this repository:", file=sys.stderr)
        for failure in provenance:
            print(f"  {failure}", file=sys.stderr)
        print(
            "\nComparing these frames would report agreement for inputs that no longer\n"
            "exist. Recapture with `just java-me-frames`.",
            file=sys.stderr,
        )
        return 2
    print("provenance: PASS (jar + routes + emulator build match the repo)")

    # ------------------------------------------ inline self-test + floors (#1,#3,#4)
    failures += inline_self_test(reference_pass, shots)
    failures += non_vacuity_and_liveness(reference_pass, shots, options.minimum_colours)

    # --------------------------------------------------------- stability (both sides)
    for side, root in (("reference", reference_root), ("port", port_root)):
        passes = passes_of(root)
        if not passes:
            if side == "port":
                continue
            failures.append(f"{side}: no captures under {root}")
            continue
        if len(passes) < 2:
            print(f"{side:9} : 1 pass -- stability NOT checked")
            if not options.allow_single_pass:
                failures.append(f"{side}: only one capture pass, so instability is undetectable")
            continue
        unstable, missing = check_stability(root, shots)
        print(f"{side:9} : {len(passes)} passes, {len(unstable)} unstable labels")
        for key, pass_name in missing:
            failures.append(f"{side}: {key} is missing from {pass_name}")
        for key, digests in unstable:
            failures.append(f"{side}: {key} is not reproducible across passes: {digests}")

    # If we are only proving the oracle bites, stop here -- no port needed.
    if options.self_test:
        print()
        if failures:
            for failure in dict.fromkeys(failures):
                print(f"FAIL {failure}")
            return 1
        print("self-test OK: reference verified, comparator reacts, frames non-vacuous")
        return 0

    # ------------------------------------------------------------- comparison
    port_passes = passes_of(port_root)
    port_pass = port_passes[0] if port_passes else None
    results: list = []
    for shot in shots:
        reference_file = frame_path(reference_pass, shot)
        if not reference_file.exists():
            results.append(
                Result(shot.label, shot.route, shot.layer, "reference-missing", False,
                       "the route did not reach this label in the reference")
            )
            continue
        port_file = frame_path(port_pass, shot) if port_pass else None
        if port_file is None or not port_file.exists():
            results.append(
                Result(shot.label, shot.route, shot.layer, "port-missing", False,
                       "the port produced no frame for this label")
            )
            continue
        reference_image = read_png(reference_file)
        port_image = read_png(port_file)
        if (reference_image.width, reference_image.height) != (port_image.width, port_image.height):
            results.append(
                Result(shot.label, shot.route, shot.layer, "compared", False,
                       f"canvas {reference_image.width}x{reference_image.height} vs "
                       f"{port_image.width}x{port_image.height}")
            )
            continue
        metrics = compare_exact(reference_image, port_image)
        detail = (
            "byte-identical"
            if metrics["differing_pixels"] == 0
            else f"{metrics['differing_pixels']} pixels differ, RMSE {metrics['rmse']}"
        )
        result = Result(shot.label, shot.route, shot.layer, "compared", True, detail, metrics)
        if metrics["differing_pixels"]:
            path = out_root / shot.route / f"{shot.label}.diff.png"
            write_diff(path, reference_image, port_image)
            result.diff_image = str(path.relative_to(REPO_ROOT))
        results.append(result)

    # --------------------------------------------------------------- ratchet
    if options.update_ratchet:
        write_ratchet(RATCHET, results)
        print(f"\nratchet rewritten: {RATCHET.relative_to(REPO_ROOT)}")
    else:
        recorded = load_ratchet(RATCHET)
        for result in results:
            entry = recorded.get(result.key)
            if entry is None:
                failures.append(
                    f"{result.key}: no ratchet entry; review the frame and run --update-ratchet"
                )
                continue
            if entry.get("verdict") != result.verdict:
                failures.append(
                    f"{result.key}: verdict {result.verdict}, ratchet says {entry.get('verdict')}"
                )
                continue
            if result.verdict != "compared":
                continue
            was, now = entry.get("differing_pixels", 0), result.metrics["differing_pixels"]
            if now > was:
                failures.append(
                    f"{result.key}: {now} pixels differ, up from {was}; the 2D layer regressed"
                )
            elif now < was:
                failures.append(
                    f"{result.key}: {now} pixels differ, down from {was}; an improvement, so "
                    "review the diff and run --update-ratchet"
                )

    # ---------------------------------------------------------------- report
    print()
    width = max((len(result.label) for result in results), default=10)
    for result in results:
        flag = "  " if result.ok else "!!"
        print(f"{flag} {result.verdict:16} {result.layer:3} {result.label:{width}}  {result.detail}")
        if result.diff_image:
            print(f"   {'':16} {'':3} {'':{width}}  {result.diff_image}")
        if not result.ok and result.verdict in ("compared", "port-missing", "reference-missing"):
            failures.append(f"{result.key}: {result.detail}")

    counts = Counter(result.verdict for result in results)
    print()
    print("  ".join(f"{verdict}={count}" for verdict, count in sorted(counts.items())))

    if options.json:
        pathlib.Path(options.json).write_text(
            json.dumps(
                {
                    "results": [
                        {
                            "key": r.key,
                            "layer": r.layer,
                            "verdict": r.verdict,
                            "ok": r.ok,
                            "detail": r.detail,
                            "metrics": r.metrics,
                            "diff_image": r.diff_image,
                        }
                        for r in results
                    ],
                    "failures": failures,
                },
                indent=2,
            )
        )

    if failures:
        print()
        for failure in dict.fromkeys(failures):
            print(f"FAIL {failure}")
        return 1
    print("\nevery label matches its recorded exact agreement")
    return 0


if __name__ == "__main__":
    sys.exit(main())
