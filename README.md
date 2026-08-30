# __TITLE__ — J2ME recovery & native Rust port

A game-preservation project: recover __TITLE__ from its surviving J2ME builds and
reimplement it as a maintainable native Rust game for Linux and the browser,
following the shared method in `../PLAYBOOK.md` (the j2me home).

The reusable source repository is **J2ME Preservation Kit** (suggested GitHub
slug: `j2me-preservation-kit`). The `_template` directory name remains unchanged
so the existing generator and local game layout continue to work.

This repository is **resource-free** and dedicated to the public domain (CC0): it
contains only recovered-by-hand reconstruction and our own code. The original
game binaries are never committed — they live in a private resources location and
are materialized locally by an explicit fetch.

## Getting started

```sh
# materialize + verify the corpus from the private resources location
python3 tools/originals/fetch.py <resources-checkout-or-its-originals-dir>
#   or set the env var named in game.toml (resources_env)

# reconcile the materialized corpus against builds.toml, and prove the gate bites
python3 tools/originals/verify.py
python3 tools/originals/verify.py --self-test
```

With `nix` + `just`:

```sh
nix develop
just bootstrap <resources>
just check-affected       # only content-hash-invalidated gate groups
just watch-affected       # rerun those groups as their inputs change
just check
```

`tools/gates/gates.toml` is the per-project dependency manifest. The reusable
runner hashes every declared input plus its command definition, and caches a
fingerprint only after that group succeeds. Git status is not consulted. The
full `just check` remains the final/milestone battery and refreshes the cache
only after every gate and can-fail proof passes.

## Preserve reusable work here

This stamped repository is standalone, but `_template` is the source of future
ports. Any game-neutral crate, script, manifest schema, oracle/AST/gate engine,
test helper, Just recipe, or workflow explanation discovered while porting a
game must be added back to `_template` with its tests and documentation. Keep
only game facts and adapters local: recovered hashes/builds, canonical game
source, semantic mappings, variant exclusions, oracle vectors, and per-node
crosswalk rows.

`_template` itself owns and tests the portable `j2me-*` crate sources. The home
generator deliberately excludes those source directories when stamping a game
and emits a consumer `Cargo.toml` from `scaffold/`, pinned by public Git URL and
exact revision. Generated games must never carry copied portable crate trees.

The scaffold also carries the reusable byte-level corpus classifier, content
resource catalog, dual-decompiler driver, numeric-opcode authority, canonical
Java compiler/packager, and Java ME API stubs. Their game-specific facts live in
`game.toml [java]`; empty Phase-0 values intentionally keep Phase-2 checks red
until a baseline and canonical class closure have been reviewed.

## License

The original work authored for this repository is dedicated to the public
domain under **CC0 1.0 Universal**; see [LICENSE](LICENSE) for the complete legal
code. That dedication does not apply to surviving game distributions or other
third-party works. Such bytes are not part of this repository and remain with
their respective rightsholders.

## Status

Phase 0 (resource-free foundation) is scaffolded together with the reusable
Phase-3 support layers: `j2me-codec`, `j2me-jvm`, `j2me-canvas`, `j2me-me`, the
line-oracle engine, and both real AST walkers. The generated game translation is
still an honest zero-coverage adapter. See `docs/STATUS.md` and the provenance
authority `java/reconstruction/builds.toml`; Phase 1 onward follows
`../PLAYBOOK.md`.
