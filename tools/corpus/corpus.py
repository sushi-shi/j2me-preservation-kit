#!/usr/bin/env python3
"""Load every surviving J2ME JAR by its recorded content identity.

The corpus includes the configured baseline and every archived variant as
differential evidence. JAD descriptors carry no bytecode and are excluded.
"""

from __future__ import annotations

import hashlib
import io
import zipfile
from dataclasses import dataclass
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore

REPO = Path(__file__).resolve().parents[2]
BUILDS_TOML = REPO / "java" / "reconstruction" / "builds.toml"
ORIGINALS = REPO / "_originals"


class CorpusError(RuntimeError):
    pass


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


@dataclass(frozen=True)
class Build:
    build_id: str
    sha256: str
    payload: bytes
    declared_language: str
    official: object
    provenance_class: str
    collection: str

    @property
    def size(self) -> int:
        return len(self.payload)


def load_manifest() -> dict:
    with BUILDS_TOML.open("rb") as handle:
        return tomllib.load(handle)


def jar_entries(manifest: dict) -> list[tuple[str, dict]]:
    rows = [
        (collection, entry)
        for collection in ("payload", "archived")
        for entry in manifest.get(collection, [])
        if entry.get("kind") == "jar"
    ]
    baseline = manifest.get("baseline")
    return sorted(rows, key=lambda row: (row[1].get("id") != baseline, row[1]["id"]))


def _resolve_payload_bytes(entry: dict) -> bytes:
    wanted = entry["sha256"]
    for reference in entry.get("containers", []):
        if not reference.startswith("_originals/"):
            continue
        path = REPO / reference
        if not path.is_file():
            continue
        data = path.read_bytes()
        if sha256(data) == wanted:
            return data
    marker = "inside _originals/"
    for reference in entry.get("containers", []):
        if not reference.startswith(marker):
            continue
        path = ORIGINALS / reference[len(marker):]
        if not path.is_file():
            continue
        try:
            with zipfile.ZipFile(path) as container:
                for member in container.namelist():
                    if member.endswith("/"):
                        continue
                    data = container.read(member)
                    if sha256(data) == wanted:
                        return data
        except zipfile.BadZipFile as error:
            raise CorpusError(f"recorded container is not a ZIP archive: {path}") from error
    raise CorpusError(
        f"payload {entry['id']} ({wanted[:12]}) not found in _originals; "
        f"materialize it with `just bootstrap <{load_manifest().get('slug', 'game')}-resources>`"
    )


def builds() -> list[Build]:
    manifest = load_manifest()
    result = []
    for collection, entry in jar_entries(manifest):
        payload = _resolve_payload_bytes(entry)
        if len(payload) != entry["bytes"]:
            raise CorpusError(f"{entry['id']}: byte count does not match builds.toml")
        try:
            with zipfile.ZipFile(io.BytesIO(payload)) as jar:
                jar.infolist()
        except zipfile.BadZipFile as error:
            raise CorpusError(f"{entry['id']}: recorded JAR is not a ZIP archive") from error
        result.append(
            Build(
                build_id=entry["id"],
                sha256=entry["sha256"],
                payload=payload,
                declared_language=entry.get("declared_language", "unknown"),
                official=entry.get("official", False),
                provenance_class=entry.get("provenance_class", "unknown"),
                collection=collection,
            )
        )
    if not result:
        raise CorpusError("no JAR entries found in builds.toml")
    return result


def jar_members(payload: bytes) -> list[tuple[str, bytes]]:
    with zipfile.ZipFile(io.BytesIO(payload)) as jar:
        return [
            (name, jar.read(name))
            for name in sorted(jar.namelist())
            if not name.endswith("/")
        ]
