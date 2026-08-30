#!/usr/bin/env python3
"""Scope one Java/Rust AST crosswalk body from the kit's live emitters.

The command deliberately does not guess semantic ownership.  It selects one
exact canonical Java item and one or more exact Rust items, prints their indexed
node inventories, and emits the hashes/counts needed to start a schema-2 body
row.  Original classfile Code/opcode hashes may be supplied by a per-game
bytecode reader; when present they are carried into both generated stubs.
"""

from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import json
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Sequence


ROOT = Path(__file__).resolve().parents[2]
JAVA_DUMPER = ROOT / "tools" / "ast" / "JavaAstAuditDump.java"
RUST_PACKAGE = "j2me-ast-audit"
SHA256_RE = re.compile(r"[0-9a-fA-F]{64}")


class ScopeError(RuntimeError):
    """A requested item could not be scoped unambiguously."""


@dataclass(frozen=True)
class AstItem:
    source: str
    owner: str | None
    item: str
    ast: str
    nodes: tuple[str, ...]


Runner = Callable[..., subprocess.CompletedProcess[str]]


def digest(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def node_digest(nodes: Sequence[str]) -> str:
    return digest("\n".join(nodes))


def _existing_files(values: Sequence[str], label: str) -> list[Path]:
    paths = [Path(value).resolve() for value in values]
    missing = [str(path) for path in paths if not path.is_file()]
    if missing:
        raise ScopeError(f"missing {label} file(s): {missing}")
    return paths


def parse_java_output(output: str) -> list[AstItem]:
    items = []
    for line_number, line in enumerate(output.splitlines(), 1):
        try:
            source, owner, item, encoded_ast, encoded_nodes = line.split("\t", 4)
            ast = base64.b64decode(encoded_ast, validate=True).decode("utf-8")
            raw_nodes = base64.b64decode(encoded_nodes, validate=True).decode("utf-8")
        except (binascii.Error, ValueError, UnicodeDecodeError) as error:
            raise ScopeError(
                f"invalid Java AST emitter row at output line {line_number}"
            ) from error
        items.append(
            AstItem(
                source=source,
                owner=owner,
                item=item,
                ast=ast,
                nodes=tuple(raw_nodes.splitlines()) if raw_nodes else (),
            )
        )
    return items


def parse_rust_output(output: str) -> list[AstItem]:
    items = []
    for line_number, line in enumerate(output.splitlines(), 1):
        try:
            source, item, encoded_ast, encoded_nodes = line.split("\t", 3)
            ast = bytes.fromhex(encoded_ast).decode("utf-8")
            raw_nodes = bytes.fromhex(encoded_nodes).decode("utf-8")
        except (ValueError, UnicodeDecodeError) as error:
            raise ScopeError(
                f"invalid Rust AST emitter row at output line {line_number}"
            ) from error
        items.append(
            AstItem(
                source=source,
                owner=None,
                item=item,
                ast=ast,
                nodes=tuple(raw_nodes.splitlines()) if raw_nodes else (),
            )
        )
    return items


def run_java_emitter(sources: Sequence[str], *, runner: Runner = subprocess.run) -> list[AstItem]:
    paths = _existing_files(sources, "Java source")
    with tempfile.TemporaryDirectory(prefix="j2me-crosswalk-java-") as directory:
        runner(
            ["javac", "-d", directory, str(JAVA_DUMPER)],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        result = runner(
            [
                "java",
                "-cp",
                directory,
                "JavaAstAuditDump",
                *(str(path) for path in paths),
            ],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
    return parse_java_output(result.stdout)


def run_rust_emitter(
    sources: Sequence[str],
    *,
    production_only: bool = True,
    runner: Runner = subprocess.run,
) -> list[AstItem]:
    paths = _existing_files(sources, "Rust source")
    command = ["cargo", "run", "-q", "-p", RUST_PACKAGE, "--"]
    if production_only:
        command.append("--production-only")
    command.extend(str(path) for path in paths)
    result = runner(
        command,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return parse_rust_output(result.stdout)


def select_java(items: Sequence[AstItem], owner: str, item: str) -> AstItem:
    matches = [value for value in items if value.owner == owner and value.item == item]
    if len(matches) != 1:
        sources = sorted(value.source for value in matches)
        raise ScopeError(
            f"Java selection {owner}.{item} matched {len(matches)} item(s)"
            + (f" in {sources}" if sources else "")
        )
    return matches[0]


def select_rust(items: Sequence[AstItem], source: str, item: str) -> AstItem:
    wanted = Path(source).resolve()
    matches = [
        value
        for value in items
        if Path(value.source).resolve() == wanted and value.item == item
    ]
    if len(matches) != 1:
        raise ScopeError(
            f"Rust selection {source}::{item} matched {len(matches)} item(s)"
        )
    return matches[0]


def display_path(value: str) -> str:
    path = Path(value).resolve()
    try:
        return path.relative_to(ROOT).as_posix()
    except ValueError:
        return str(path)


def build_scope(
    *,
    java: AstItem,
    rust: Sequence[AstItem],
    body_key: str,
    code_sha256: str | None = None,
    opcode_sha256: str | None = None,
) -> dict:
    java_ast_sha256 = digest(java.ast)
    rust_manifest = []
    rust_evidence = []
    rust_listings = []
    for target, value in enumerate(rust):
        file = display_path(value.source)
        ast_sha256 = digest(value.ast)
        rust_manifest.append(
            {
                "file": file,
                "item": value.item,
                "ast_sha256": ast_sha256,
                "nodes_sha256": node_digest(value.nodes),
                "node_count": len(value.nodes),
            }
        )
        rust_evidence.append(
            {
                "file": file,
                "item": value.item,
                "ast_sha256": ast_sha256,
                "nodes": list(value.nodes),
            }
        )
        rust_listings.append(
            {
                "target": target,
                "file": file,
                "item": value.item,
                "nodes": [
                    {"index": index, "text": node}
                    for index, node in enumerate(value.nodes)
                ],
            }
        )

    body = {
        "java_item": body_key,
        "java_ast_sha256": java_ast_sha256,
        "java_nodes_sha256": node_digest(java.nodes),
        "java_node_count": len(java.nodes),
        "semantic_status": "partial",
        "review": "",
        "rust": rust_manifest,
        "op": [],
        "adapt": [],
    }
    evidence = {
        "java_item": body_key,
        "java_ast_sha256": java_ast_sha256,
        "java_nodes": list(java.nodes),
        "rust": rust_evidence,
    }
    if code_sha256 is not None and opcode_sha256 is not None:
        for output in (body, evidence):
            output["code_sha256"] = code_sha256.lower()
            output["opcode_sha256"] = opcode_sha256.lower()

    return {
        "selection": {
            "java_source": display_path(java.source),
            "java_owner": java.owner,
            "java_item": java.item,
        },
        "body": body,
        "evidence": evidence,
        "java_nodes": [
            {"index": index, "text": node} for index, node in enumerate(java.nodes)
        ],
        "rust_nodes": rust_listings,
    }


def format_text(scope: dict) -> str:
    selection = scope["selection"]
    lines = [
        f"JAVA\t{selection['java_owner']}.{selection['java_item']}",
        f"SOURCE\t{selection['java_source']}",
    ]
    body = scope["body"]
    for name in ("code_sha256", "opcode_sha256"):
        if name in body:
            lines.append(f"{name}\t{body[name]}")
    for name in ("java_ast_sha256", "java_nodes_sha256", "java_node_count"):
        lines.append(f"{name}\t{body[name]}")
    lines.extend(f"J:{node['index']}\t{node['text']}" for node in scope["java_nodes"])
    for target in scope["rust_nodes"]:
        manifest = body["rust"][target["target"]]
        lines.extend(
            [
                "",
                f"RUST:{target['target']}\t{target['file']}::{target['item']}",
                f"ast_sha256\t{manifest['ast_sha256']}",
                f"nodes_sha256\t{manifest['nodes_sha256']}",
                f"node_count\t{manifest['node_count']}",
            ]
        )
        lines.extend(
            f"R:{target['target']}:{node['index']}\t{node['text']}"
            for node in target["nodes"]
        )
    lines.extend(
        [
            "",
            "SCHEMA2_BODY_JSON",
            json.dumps(body, indent=2, sort_keys=True),
            "",
            "SCHEMA2_EVIDENCE_JSON",
            json.dumps(scope["evidence"], indent=2, sort_keys=True),
        ]
    )
    return "\n".join(lines)


def _sha256(value: str) -> str:
    if not SHA256_RE.fullmatch(value):
        raise argparse.ArgumentTypeError("must be exactly 64 hexadecimal characters")
    return value.lower()


def parse_arguments(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--java-source",
        action="append",
        required=True,
        metavar="FILE",
        help="canonical Java source to parse; repeat for dependent/nested owners",
    )
    parser.add_argument("--java-owner", required=True, metavar="OWNER")
    parser.add_argument("--java-item", required=True, metavar="ITEM")
    parser.add_argument(
        "--rust",
        action="append",
        required=True,
        nargs=2,
        metavar=("FILE", "ITEM"),
        help="Rust source and exact emitter item; repeat for multiple targets",
    )
    parser.add_argument(
        "--body-key",
        help="schema-2 evidence key (default: OWNER.ITEM)",
    )
    parser.add_argument(
        "--code-sha256",
        required=True,
        type=_sha256,
        help="authoritative original classfile Code-attribute hash",
    )
    parser.add_argument(
        "--opcode-sha256",
        required=True,
        type=_sha256,
        help="authoritative original opcode-sequence hash",
    )
    parser.add_argument(
        "--include-nonproduction",
        action="store_true",
        help="include cfg(test) and other non-production Rust items",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit one machine-readable document instead of the annotated listing",
    )
    arguments = parser.parse_args(argv)
    return arguments


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_arguments(argv)
    try:
        java_items = run_java_emitter(arguments.java_source)
        rust_files = list(dict.fromkeys(target[0] for target in arguments.rust))
        rust_items = run_rust_emitter(
            rust_files,
            production_only=not arguments.include_nonproduction,
        )
        java = select_java(java_items, arguments.java_owner, arguments.java_item)
        rust = [
            select_rust(rust_items, source, item)
            for source, item in arguments.rust
        ]
        scope = build_scope(
            java=java,
            rust=rust,
            body_key=arguments.body_key
            or f"{arguments.java_owner}.{arguments.java_item}",
            code_sha256=arguments.code_sha256,
            opcode_sha256=arguments.opcode_sha256,
        )
    except (ScopeError, subprocess.CalledProcessError) as error:
        print(f"crosswalk scope failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(scope, indent=2, sort_keys=True) if arguments.json else format_text(scope))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
