#!/usr/bin/env bash
#
# Capture Java ME reference frames for a 2D J2ME game from a real J2ME runtime.
#
# This is an *independent witness* for what the phone platform did. A typical
# obfuscated J2ME archive ships no javax.* firmware -- MIDP/LCDUI/RMS/MMAPI were
# never in the JAR -- so the platform half can only ever be reimplemented. The
# future Rust port reimplements it; so does FreeJ2ME-Plus. Running the *original
# bytecode* on a *different* reimplementation is the only way to get a second
# opinion on the platform half of the port. An emulator is NOT the handset; treat
# every capture as a second opinion, never as ground truth.
#
# Nothing third-party is vendored into this repository. This script clones and
# builds FreeJ2ME-Plus into a scratch directory outside the repo, applies the
# local patches in tools/oracle/patches/ that game.toml selects, and writes only
# PNGs plus a manifest into _reference/oracle/reference/ (git-ignored).
#
# The routes are in tools/oracle/routes/. They are meant to be *shared* with the
# future port -- the same keystrokes drive both runtimes -- so the comparison is
# "the same route into both" rather than two hand-matched runs.
#
# GAME-AGNOSTIC. Every per-game knob -- the JAR path, the canvas geometry, the
# --sound flag, the emulator patch set, and the pinned emulator commit -- is read
# from game.toml's [oracle] section, so this script carries no game-specific
# literal and is stamped into any new 2D J2ME port unchanged. See docs/ORACLE.md.
#
# Provenance: lifted, near-verbatim, from stalker-mobile's 3D Stalker oracle,
# adapted to pure-2D games (the M3G switches and the wireframe/untextured debug
# modes are dropped; there is nothing 3D to disagree about).
#
# Usage:
#   tools/oracle/capture_reference.sh [options]
#
#     --scratch DIR    where to clone/build the emulator (default $TMPDIR/<slug>-java-me)
#     --out DIR        capture root (default _reference/oracle/reference)
#     --routes A,B     only these routes, by file stem
#     --route-dir DIR  read routes from here instead of tools/oracle/routes
#     --passes N       run the whole matrix N times (default 2, to catch flakiness)
#     --build-only     build the emulator and stop
#
# Environment:
#   JAVA_HOME  a *non-headless* JDK with working AWT font metrics. The nix
#              devshell's jdk17_headless has no libfontmanager and FreeJ2ME's
#              PlatformFont needs it, so JAVA_HOME is *probed like any other
#              candidate*, not trusted. `nix shell nixpkgs#jdk` (openjdk-21) works
#              and is used as a fallback candidate. Autodetected if unset.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Help must work without game.toml or python, so answer it before anything else.
case "${1:-}" in -h|--help) sed -n '2,47p' "$0"; exit 0 ;; esac

log() { printf '[capture] %s\n' "$*" >&2; }

# ------------------------------------------------------------ per-game config
# Every per-game knob lives in game.toml's [oracle] section; this script carries
# no game-specific literal. Read them with a tiny tomllib reader (the same library
# tools/originals/corpus_common.py uses), preferring python3 on PATH and falling
# back to `nix shell nixpkgs#python3` so it also works outside the dev shell.
read_oracle_config() {
  local reader='
import sys, shlex
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib
with open(sys.argv[1], "rb") as fh:
    cfg = tomllib.load(fh)
o = cfg.get("oracle")
if not o:
    sys.exit("game.toml has no [oracle] section")
def need(k):
    if k not in o:
        sys.exit("game.toml [oracle] is missing key: " + k)
    return o[k]
def emit(name, value):
    print(name + "=" + shlex.quote(str(value)))
emit("CFG_JAR", need("jar"))
emit("CFG_CANVAS_W", need("canvas_w"))
emit("CFG_CANVAS_H", need("canvas_h"))
emit("CFG_SOUND", need("sound"))
emit("CFG_EMU_COMMIT", need("freej2me_commit"))
emit("CFG_SLUG", cfg.get("slug", "j2me"))
emit("CFG_PATCHES", " ".join(need("patches")))
if "freej2me_repo" in o:
    emit("CFG_EMU_REPO", o["freej2me_repo"])
'
  if command -v python3 >/dev/null 2>&1; then
    python3 -c "$reader" "$REPO_ROOT/game.toml"
  elif command -v nix >/dev/null 2>&1; then
    nix shell nixpkgs#python3 --command python3 -c "$reader" "$REPO_ROOT/game.toml"
  else
    echo "need python3 (or nix) to read game.toml [oracle]" >&2
    return 1
  fi
}
config="$(read_oracle_config)" || { log "FATAL: could not read [oracle] knobs from game.toml"; exit 1; }
eval "$config"

# ------------------------------------------------------------------- defaults
SCRATCH="${TMPDIR:-/tmp}/${CFG_SLUG}-java-me"
OUT_DIR="$REPO_ROOT/_reference/oracle/reference"
ROUTE_DIR="$REPO_ROOT/tools/oracle/routes"
PATCH_DIR="$REPO_ROOT/tools/oracle/patches"
JAR="$REPO_ROOT/$CFG_JAR"
ROUTE_FILTER=""
PASSES=2
BUILD_ONLY=0

# The emulator is always FreeJ2ME-Plus (game-agnostic), pinned per game in
# game.toml so a rerun reproduces these captures. Bump the pin deliberately.
EMU_REPO="${CFG_EMU_REPO:-https://github.com/TASEmulators/freej2me-plus.git}"
EMU_COMMIT="$CFG_EMU_COMMIT"

# Reference canvas geometry the port targets, and the --sound flag. Per game:
# some games' loaders deadlock on a white screen unless their audio thread stays
# alive under --sound 1. All three come from game.toml [oracle].
CANVAS_W="$CFG_CANVAS_W"
CANVAS_H="$CFG_CANVAS_H"
SOUND="$CFG_SOUND"

# Resolve the patch names from game.toml [oracle].patches to files under
# tools/oracle/patches/. A name maps to freej2me-plus-<name>.patch (or, as a
# fallback, <name>.patch / <name>). An empty list is valid: a game may need none.
PATCHES=()
for name in $CFG_PATCHES; do
  resolved=""
  for cand in \
      "$PATCH_DIR/freej2me-plus-$name.patch" \
      "$PATCH_DIR/$name.patch" \
      "$PATCH_DIR/$name"; do
    [ -f "$cand" ] && { resolved="$cand"; break; }
  done
  [ -n "$resolved" ] || { log "FATAL: patch '$name' (game.toml [oracle].patches) not found under $PATCH_DIR"; exit 1; }
  PATCHES+=("$resolved")
done

# sha256s of the selected patches, space-joined, for the stamp and manifest.
# Guarded so an empty patch set does not leave sha256sum reading stdin forever.
patch_shas() {
  [ "${#PATCHES[@]}" -eq 0 ] && { printf 'none '; return; }
  sha256sum "${PATCHES[@]}" | cut -d' ' -f1 | tr '\n' ' '
}

while [ $# -gt 0 ]; do
  case "$1" in
    --scratch) SCRATCH="$2"; shift 2 ;;
    --out) OUT_DIR="$2"; shift 2 ;;
    --routes) ROUTE_FILTER="$2"; shift 2 ;;
    --route-dir) ROUTE_DIR="$2"; shift 2 ;;
    --passes) PASSES="$2"; shift 2 ;;
    --build-only) BUILD_ONLY=1; shift ;;
    -h|--help) sed -n '2,47p' "$0"; exit 0 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done

find_java() {
  # Probe every candidate JDK for headless AWT font metrics, which FreeJ2ME needs
  # before it can even construct MobilePlatform. JAVA_HOME is *probed*, never
  # trusted: the nix devshell exports jdk17_headless, which has no libfontmanager
  # and fails at PlatformFont.<init> after the whole emulator has compiled.
  local probe_dir="$SCRATCH/probe"
  mkdir -p "$probe_dir"
  cat > "$probe_dir/FontProbe.java" <<'EOF'
public class FontProbe { public static void main(String[] a) throws Exception {
  java.awt.image.BufferedImage i = new java.awt.image.BufferedImage(8,8,2);
  System.out.println(i.createGraphics().getFontMetrics(new java.awt.Font("Monospace",0,12)).getHeight());
}}
EOF
  local candidates=()
  [ -n "${JAVA_HOME:-}" ] && candidates+=("$JAVA_HOME/bin")
  # Non-headless nixpkgs OpenJDKs already realised in the store.
  while IFS= read -r d; do candidates+=("$d"); done < <(
    ls -d /nix/store/*openjdk-2[0-9]*/bin /nix/store/*openjdk-1[7-9]*/bin 2>/dev/null | grep -v headless || true)
  # Last resort: materialise a JDK from nixpkgs (openjdk-21 works here).
  if command -v nix >/dev/null 2>&1; then
    local nixbin
    nixbin="$(nix shell nixpkgs#jdk --command bash -c 'dirname "$(command -v javac)"' 2>/dev/null || true)"
    [ -n "$nixbin" ] && candidates+=("$nixbin")
  fi
  local candidate
  for candidate in "${candidates[@]}"; do
    [ -x "$candidate/javac" ] || continue
    "$candidate/javac" -d "$probe_dir" "$probe_dir/FontProbe.java" >/dev/null 2>&1 || continue
    if "$candidate/java" -Djava.awt.headless=true -cp "$probe_dir" FontProbe >/dev/null 2>&1; then
      echo "$candidate"; return
    fi
  done
  log "FATAL: no JDK with working headless AWT font metrics found. Set JAVA_HOME."
  exit 1
}

JAVA_BIN="$(find_java)"
JAVA_VERSION="$("$JAVA_BIN/java" -version 2>&1 | head -1)"
log "using JDK at $JAVA_BIN ($JAVA_VERSION)"

[ -f "$JAR" ] || { log "FATAL: missing $JAR -- materialize _originals first (just bootstrap <resources>)"; exit 1; }

# ---------------------------------------------------------------- build emulator
EMU="$SCRATCH/freej2me-plus"
CLASSES="$EMU/build-oracle/classes"
DRIVER="$SCRATCH/driver"
# A stamp over everything that decides what gets built, so repeated capture runs
# do not pay for a full recompile. Any change to the pin, a patch, the driver, or
# the JDK invalidates it.
STAMP_WANT="$EMU_COMMIT $(patch_shas)$(sha256sum "$REPO_ROOT/tools/oracle/HeadlessCapture.java" | cut -d' ' -f1) $JAVA_BIN"
STAMP_FILE="$SCRATCH/build.stamp"
if [ -f "$STAMP_FILE" ] && [ "$(cat "$STAMP_FILE")" = "$STAMP_WANT" ] && [ -d "$CLASSES" ] && [ -d "$DRIVER" ]; then
  log "emulator and driver already built for this pin; skipping rebuild"
  [ "$BUILD_ONLY" = "1" ] && exit 0
else
  if [ ! -d "$EMU/.git" ]; then
    log "cloning FreeJ2ME-Plus into $EMU"
    mkdir -p "$SCRATCH"
    git clone "$EMU_REPO" "$EMU"
  fi
  git -C "$EMU" fetch --depth 200 origin >/dev/null 2>&1 || true
  git -C "$EMU" checkout -q --force "$EMU_COMMIT"
  git -C "$EMU" clean -qfd -e build -e build-oracle

  # The patches game.toml selected, applied in listed order. For a pure-2D game
  # these are the two non-M3G hooks lifted from the Stalker oracle:
  #   freej2me-plus-deterministic-clock.patch
  #     Adds a disabled-by-default clock hook used only by HeadlessCapture. The
  #     game's System.currentTimeMillis()/nanoTime() are already routed through
  #     MIDletEnhancements by FreeJ2ME's bytecode rewriter; the hook lets the
  #     capture driver freeze that clock and advance it by each route command's
  #     exact millisecond budget, so animations are a function of frame count and
  #     not of host speed.
  #   freej2me-plus-deterministic-input.patch
  #     Adds a disabled-by-default Display hook that drains the ordinary Canvas
  #     key callback at the route boundary, eliminating host-thread races without
  #     bypassing the game's own input delivery path.
  # The two M3G patches the Stalker oracle also carried are deliberately NOT here
  # for a pure-2D game that never touches javax.microedition.m3g.
  for patch in "${PATCHES[@]}"; do
    log "applying $(basename "$patch")"
    # Upstream mixes LF and CRLF; ignore whitespace-only differences while
    # keeping the pinned-revision context and ordinary hunk checks.
    git -C "$EMU" apply --ignore-space-change "$patch"
  done

  log "compiling emulator"
  rm -rf "$EMU/build-oracle"; mkdir -p "$CLASSES"
  find "$EMU/src" -name '*.java' -not -path "$EMU/src/libretro/*" > "$SCRATCH/srcs.txt"
  "$JAVA_BIN/javac" -g -nowarn -source 8 -target 8 -encoding utf-8 \
    -d "$CLASSES" "@$SCRATCH/srcs.txt" 2>&1 | grep -vE 'warning|^Note:' || true
  cp -r "$EMU/resources/." "$CLASSES/" 2>/dev/null || true

  log "compiling headless capture driver"
  rm -rf "$DRIVER"; mkdir -p "$DRIVER"
  "$JAVA_BIN/javac" -cp "$CLASSES" -d "$DRIVER" "$REPO_ROOT/tools/oracle/HeadlessCapture.java"

  printf '%s' "$STAMP_WANT" > "$STAMP_FILE"
fi

[ "$BUILD_ONLY" = "1" ] && { log "build only; stopping"; exit 0; }

# ---------------------------------------------------------------- routes
routes=()
for file in "$ROUTE_DIR"/*.txt; do
  stem="$(basename "$file" .txt)"
  if [ -n "$ROUTE_FILTER" ]; then
    case ",$ROUTE_FILTER," in *",$stem,"*) ;; *) continue ;; esac
  fi
  routes+=("$stem")
done
[ "${#routes[@]}" -gt 0 ] || { log "FATAL: no routes selected"; exit 1; }
log "routes: ${routes[*]}"

# A scoped or lower-pass rerun must not inherit an older selected route from a
# pass it will not rewrite. Preserve unrelated routes, but remove this run's
# route directories from every existing pass before recording anything.
for pass_dir in "$OUT_DIR"/pass-*; do
  [ -d "$pass_dir" ] || continue
  for stem in "${routes[@]}"; do
    rm -rf "$pass_dir/$stem"
  done
  rmdir "$pass_dir" 2>/dev/null || true
done

run_route() { # run_route <out-dir> <route-stem>
  local dir="$1" stem="$2"
  rm -rf "$dir"; mkdir -p "$dir"
  # A per-run working directory: FreeJ2ME writes its RMS record store beside the
  # process, so a fresh directory is what makes every route a fresh install.
  local work="$SCRATCH/work/$$-$RANDOM"
  rm -rf "$work"; mkdir -p "$work"
  if ! ( cd "$work" && "$JAVA_BIN/java" \
      --add-opens=java.base/java.lang=ALL-UNNAMED \
      --add-opens=java.base/java.util=ALL-UNNAMED \
      -Djava.awt.headless=true -Dfile.encoding=ISO_8859_1 \
      -cp "$CLASSES:$DRIVER" HeadlessCapture \
      --jar "$JAR" --width "$CANVAS_W" --height "$CANVAS_H" \
      --out "$dir" --script "$ROUTE_DIR/$stem.txt" --log 3 --backlight 0 --sound "$SOUND" \
      > "$dir/run.log" 2>&1 ); then
    log "FATAL: $stem failed; see $dir/run.log"
    rm -rf "$work"
    return 1
  fi
  rm -rf "$work"
  local shots
  shots="$(find "$dir" -name '*.png' | wc -l)"
  if [ "$shots" -eq 0 ]; then
    log "FATAL: $stem exited cleanly but wrote no frames; see $dir/run.log"
    return 1
  fi
  # Stamp the route beside the frames it produced, so a partial re-run cannot
  # vouch for a route it did not run.
  sha256sum "$ROUTE_DIR/$stem.txt" | cut -d' ' -f1 > "$dir/route.sha256"
  log "  $stem: $shots frames"
}

mkdir -p "$OUT_DIR"
for pass in $(seq 1 "$PASSES"); do
  log "pass $pass of $PASSES"
  for stem in "${routes[@]}"; do
    run_route "$OUT_DIR/pass-$pass/$stem" "$stem"
  done
done

# ---------------------------------------------------------------- manifest
row() { printf '%s\t%s\n' "$1" "$2"; }
{
  echo "# Java ME reference captures -- machine-readable provenance."
  echo "# Regenerate with tools/oracle/capture_reference.sh."
  row captured_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  row repo_head "$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
  row jar "$(basename "$JAR")"
  row jar_sha256 "$(sha256sum "$JAR" | cut -d' ' -f1)"
  row canvas "${CANVAS_W}x${CANVAS_H}"
  row emulator_repo "$EMU_REPO"
  row emulator_commit "$EMU_COMMIT"
  for patch in "${PATCHES[@]}"; do
    printf 'emulator_patch\t%s\t%s\n' "tools/oracle/patches/$(basename "$patch")" \
      "$(sha256sum "$patch" | cut -d' ' -f1)"
  done
  row jvm "$JAVA_VERSION"
  row passes "$PASSES"
  row routes_captured "${routes[*]}"
  for stamp in "$OUT_DIR"/pass-1/*/route.sha256; do
    [ -f "$stamp" ] || continue
    printf 'route_sha256\t%s\t%s\n' "$(basename "$(dirname "$stamp")")" "$(cat "$stamp")"
  done
} > "$OUT_DIR/manifest.tsv"

log "captures written to $OUT_DIR"
