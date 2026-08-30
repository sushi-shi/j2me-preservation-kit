# Declarative class admissions

An admission plan is the small authored join between one original class,
canonical Java items, strict Rust items, and an optional differential oracle.
Store plans as `java/reconstruction/admissions/<id>.toml`.

```toml
schema_version = 1
id = "crc32"
label = "Game CRC32"
owner = "e"
java_owner = "Crc32Checksum"
java_source = "java/src/main/java/game/Crc32Checksum.java"
crosswalk_manifest = "transliteration/audits/crc32.crosswalk.toml"
variant_manifest = "java/reconstruction/variants/crc32.toml"
# Optional; otherwise every selected build in builds.toml is used.
builds = ["game_01"]

[[body]]
original_name = "a"
descriptor = "(Ljava/lang/String;)J"
java_item = "calculate(String)"
rust = [
  { file = "transliteration/game-xlat/src/crc32.rs", item = "fn:calculate" },
]

# Optional. Both commands must be noninteractive and the second must inject a
# mismatch/fault and succeed only when the oracle rejects it.
[oracle]
command = ["python3", "tools/oracle/game/crc32_oracle.py"]
canfail_command = ["python3", "tools/oracle/game/crc32_oracle.py", "--self-test"]
```

Run `just admission-scaffold <plan>` once. It reads the original corpus,
preserves the whole-game body denominator, creates an intentionally incomplete
crosswalk manifest, and records every live owner method in a variant ledger.
It never overwrites either file. Identical observations are mechanically
classified `common`; actual cross-build differences are emitted as invalid
`REVIEW_REQUIRED` rows so a human must choose and explain the policy.

After reviewing nodes from `just admission-inventory <plan>`, fill the
crosswalk decisions and run `just admission-check <plan>`. The repository-wide
`just admissions-check` discovers all plans and runs symbols, variants, live
crosswalk plus mutation proof, and the configured oracle plus mutation proof.
Discovery must exactly match the complete-class count and owner set in
`symbols.toml`; deleting or replacing a plan therefore turns the gate red.
