from __future__ import annotations

import base64
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "ast"))

from scope_crosswalk import (  # noqa: E402
    AstItem,
    ScopeError,
    build_scope,
    format_text,
    parse_java_output,
    parse_rust_output,
    run_java_emitter,
    run_rust_emitter,
    select_java,
    select_rust,
)
from validate_crosswalk import load_evidence  # noqa: E402


def _digest(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


class CrosswalkScoperTests(unittest.TestCase):
    def test_emitter_rows_decode_and_exact_selection_rejects_ambiguity(self) -> None:
        ast = "return x != null;"
        nodes = "RETURN\t\nNOT_EQUAL_TO\t\nNULL_LITERAL\tnull"
        encoded = lambda value: base64.b64encode(value.encode()).decode()
        java = parse_java_output(
            f"A.java\tEngine\tm(String)\t{encoded(ast)}\t{encoded(nodes)}\n"
        )
        self.assertEqual(
            select_java(java, "Engine", "m(String)").nodes[2],
            "NULL_LITERAL\tnull",
        )
        with self.assertRaisesRegex(ScopeError, "matched 0 item"):
            select_java(java, "Engine", "other()")
        with self.assertRaisesRegex(ScopeError, "matched 2 item"):
            select_java(java + java, "Engine", "m(String)")

        rust_ast = "fn m() -> bool { true }"
        rust_nodes = "BLOCK\t1\nLITERAL\ttrue"
        rust = parse_rust_output(
            f"port.rs\tfn:m\t{rust_ast.encode().hex()}\t{rust_nodes.encode().hex()}\n"
        )
        self.assertEqual(select_rust(rust, "port.rs", "fn:m").nodes[0], "BLOCK\t1")

    def test_scope_contains_indexed_nodes_and_schema2_hash_evidence(self) -> None:
        java = AstItem(
            source=str(ROOT / "Game.java"),
            owner="Game",
            item="exists(String)",
            ast="return get(name) != null;",
            nodes=("RETURN\t", "NOT_EQUAL_TO\t", "NULL_LITERAL\tnull"),
        )
        rust = AstItem(
            source=str(ROOT / "src" / "lib.rs"),
            owner=None,
            item="fn:exists",
            ast="fn exists() { get()?.is_some() }",
            nodes=("BLOCK\t1", "METHOD_CALL\tis_some\t0"),
        )
        code = "A" * 64
        opcode = "b" * 64
        scope = build_scope(
            java=java,
            rust=[rust],
            body_key="Original.exists(Ljava/lang/String;)Z",
            code_sha256=code,
            opcode_sha256=opcode,
        )

        body = scope["body"]
        evidence = scope["evidence"]
        self.assertEqual(body["semantic_status"], "partial")
        self.assertEqual(body["op"], [])
        self.assertEqual(body["adapt"], [])
        self.assertEqual(body["java_node_count"], 3)
        self.assertEqual(body["java_ast_sha256"], _digest(java.ast))
        self.assertEqual(
            body["java_nodes_sha256"], _digest("\n".join(java.nodes))
        )
        self.assertEqual(body["code_sha256"], code.lower())
        self.assertEqual(body["rust"][0]["node_count"], 2)
        self.assertEqual(body["rust"][0]["ast_sha256"], _digest(rust.ast))
        self.assertEqual(scope["java_nodes"][1]["index"], 1)
        self.assertEqual(scope["rust_nodes"][0]["nodes"][1]["index"], 1)
        self.assertEqual(evidence["java_nodes"], list(java.nodes))
        self.assertEqual(evidence["rust"][0]["nodes"], list(rust.nodes))
        loaded = load_evidence({"body": [evidence]})
        self.assertEqual(
            loaded["Original.exists(Ljava/lang/String;)Z"].java_nodes,
            java.nodes,
        )
        rendered = format_text(scope)
        self.assertIn("J:1\tNOT_EQUAL_TO\t", rendered)
        self.assertIn("R:0:1\tMETHOD_CALL\tis_some\t0", rendered)
        self.assertIn("SCHEMA2_BODY_JSON", rendered)
        self.assertIn("SCHEMA2_EVIDENCE_JSON", rendered)
        json.dumps(scope)  # The complete report remains machine-readable.

    def test_runners_reuse_the_shipped_emitters(self) -> None:
        calls: list[list[str]] = []
        with tempfile.TemporaryDirectory() as directory:
            java_source = Path(directory) / "Game.java"
            rust_source = Path(directory) / "lib.rs"
            java_source.touch()
            rust_source.touch()

            encoded = lambda value: base64.b64encode(value.encode()).decode()
            java_nodes = encoded("RETURN\t")
            rust_ast = "fn m() {}".encode().hex()
            rust_nodes = "BLOCK\t0".encode().hex()
            java_stdout = (
                f"{java_source}\tGame\tm()\t{encoded('return;')}\t"
                f"{java_nodes}\n"
            )
            rust_stdout = (
                f"{rust_source}\tfn:m\t{rust_ast}\t{rust_nodes}\n"
            )

            def runner(
                command: list[str], **_kwargs: object
            ) -> subprocess.CompletedProcess[str]:
                calls.append(command)
                stdout = java_stdout if command[0] == "java" else ""
                if command[0] == "cargo":
                    stdout = rust_stdout
                return subprocess.CompletedProcess(command, 0, stdout, "")

            self.assertEqual(
                run_java_emitter([str(java_source)], runner=runner)[0].item, "m()"
            )
            self.assertEqual(
                run_rust_emitter([str(rust_source)], runner=runner)[0].item,
                "fn:m",
            )

        self.assertEqual(calls[0][0], "javac")
        self.assertIn(
            str(ROOT / "tools" / "ast" / "JavaAstAuditDump.java"), calls[0]
        )
        self.assertEqual(calls[1][0], "java")
        self.assertEqual(
            calls[2][:6],
            ["cargo", "run", "-q", "-p", "j2me-ast-audit", "--"],
        )
        self.assertIn("--production-only", calls[2])


if __name__ == "__main__":
    unittest.main()
