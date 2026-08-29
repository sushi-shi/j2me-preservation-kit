//! Formatting-independent Rust AST evidence for the Java-to-Rust audit.

use std::path::Path;

use quote::ToTokens;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, ImplItem, Item, Type};

fn is_test_only(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        let syn::Meta::List(meta) = &attribute.meta else {
            return false;
        };
        meta.path.is_ident("cfg") && meta.tokens.to_string() == "test"
    })
}

fn hex(value: &str) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.bytes() {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 15) as usize] as char);
    }
    encoded
}

fn qualified(prefix: &str, name: impl ToString) -> String {
    let name = name.to_string();
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}::{name}")
    }
}

#[derive(Default)]
struct BodyNodes {
    nodes: Vec<String>,
}

impl BodyNodes {
    fn push(&mut self, kind: &str, detail: impl ToString) {
        self.nodes.push(format!("{kind}\t{}", detail.to_string()));
    }

    fn encoded(&self) -> String {
        hex(&self.nodes.join("\n"))
    }
}

impl<'ast> Visit<'ast> for BodyNodes {
    fn visit_block(&mut self, node: &'ast syn::Block) {
        self.push("BLOCK", node.stmts.len());
        visit::visit_block(self, node);
    }

    // A macro terminated with `;` is syn::Stmt::Macro rather than
    // Expr::Macro. Without this hook, a loud boundary such as panic! or
    // unimplemented! could exist in a supposedly exhaustive body while
    // contributing no semantic node (Gothic crosswalk finding G-12).
    fn visit_stmt(&mut self, node: &'ast syn::Stmt) {
        if let syn::Stmt::Macro(statement) = node {
            self.push("MACRO", statement.mac.to_token_stream());
        }
        visit::visit_stmt(self, node);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        self.push("LOCAL", node.pat.to_token_stream());
        visit::visit_local(self, node);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        self.push("PATH", node.to_token_stream());
        visit::visit_path(self, node);
    }

    fn visit_field_value(&mut self, node: &'ast syn::FieldValue) {
        self.push("FIELD_INIT", node.member.to_token_stream());
        visit::visit_field_value(self, node);
    }

    fn visit_expr(&mut self, node: &'ast Expr) {
        match node {
            Expr::Array(value) => self.push("ARRAY", value.elems.len()),
            Expr::Assign(_) => self.push("ASSIGN", "="),
            Expr::Async(_) => self.push("ASYNC", ""),
            Expr::Await(_) => self.push("AWAIT", ""),
            Expr::Binary(value) => self.push("BINARY", value.op.to_token_stream()),
            Expr::Block(_) => self.push("BLOCK_EXPR", ""),
            Expr::Break(value) => self.push(
                "BREAK",
                value
                    .label
                    .as_ref()
                    .map(ToTokens::to_token_stream)
                    .unwrap_or_default(),
            ),
            Expr::Call(value) => self.push("CALL", value.args.len()),
            Expr::Cast(value) => self.push("CAST", value.ty.to_token_stream()),
            Expr::Closure(value) => self.push("CLOSURE", value.inputs.len()),
            Expr::Const(_) => self.push("CONST_BLOCK", ""),
            Expr::Continue(value) => self.push(
                "CONTINUE",
                value
                    .label
                    .as_ref()
                    .map(ToTokens::to_token_stream)
                    .unwrap_or_default(),
            ),
            Expr::Field(value) => self.push("FIELD", value.member.to_token_stream()),
            Expr::ForLoop(_) => self.push("FOR_LOOP", ""),
            Expr::Group(_) => self.push("GROUP", ""),
            Expr::If(_) => self.push("IF", ""),
            Expr::Index(_) => self.push("INDEX", ""),
            Expr::Infer(_) => self.push("INFER", ""),
            Expr::Let(_) => self.push("LET_EXPR", ""),
            Expr::Lit(value) => self.push("LITERAL", value.lit.to_token_stream()),
            Expr::Loop(_) => self.push("LOOP", ""),
            Expr::Macro(value) => self.push("MACRO", value.mac.to_token_stream()),
            Expr::Match(value) => self.push("MATCH", value.arms.len()),
            Expr::MethodCall(value) => {
                self.push(
                    "METHOD_CALL",
                    format!("{}\t{}", value.method, value.args.len()),
                );
            }
            Expr::Paren(_) => self.push("PAREN", ""),
            Expr::Path(value) => self.push("PATH_EXPR", value.path.to_token_stream()),
            Expr::Range(value) => self.push("RANGE", value.limits.to_token_stream()),
            Expr::RawAddr(value) => {
                self.push("RAW_ADDRESS", value.mutability.to_token_stream());
            }
            Expr::Reference(value) => self.push(
                "REFERENCE",
                if value.mutability.is_some() {
                    "mut"
                } else {
                    "shared"
                },
            ),
            Expr::Repeat(_) => self.push("REPEAT", ""),
            Expr::Return(value) => self.push("RETURN", usize::from(value.expr.is_some())),
            Expr::Struct(value) => self.push("STRUCT", value.path.to_token_stream()),
            Expr::Try(_) => self.push("TRY", "?"),
            Expr::TryBlock(_) => self.push("TRY_BLOCK", ""),
            Expr::Tuple(value) => self.push("TUPLE", value.elems.len()),
            Expr::Unary(value) => self.push("UNARY", value.op.to_token_stream()),
            Expr::Unsafe(_) => self.push("UNSAFE", ""),
            Expr::Verbatim(value) => self.push("VERBATIM", value),
            Expr::While(_) => self.push("WHILE", ""),
            Expr::Yield(value) => self.push("YIELD", usize::from(value.expr.is_some())),
            _ => self.push("UNKNOWN_EXPR", node.to_token_stream()),
        }
        visit::visit_expr(self, node);
    }
}

fn body_nodes(block: &syn::Block) -> String {
    let mut nodes = BodyNodes::default();
    nodes.visit_block(block);
    nodes.encoded()
}

fn expression_nodes(expression: &Expr) -> String {
    let mut nodes = BodyNodes::default();
    nodes.visit_expr(expression);
    nodes.encoded()
}

fn field_nodes(field: &syn::Field) -> String {
    let mut nodes = BodyNodes::default();
    nodes.push(
        "FIELD_DECL",
        field
            .ident
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    nodes.visit_type(&field.ty);
    nodes.encoded()
}

fn type_name(ty: &Type) -> String {
    ty.to_token_stream().to_string()
}

fn emit(path: &Path, item: String, ast: impl ToTokens, nodes: String) {
    println!(
        "{}\t{}\t{}\t{}",
        path.display(),
        item,
        hex(&ast.to_token_stream().to_string()),
        nodes
    );
}

fn emit_items(path: &Path, items: Vec<Item>, prefix: &str, production_only: bool) {
    for item in items {
        match item {
            Item::Fn(mut function) => {
                if production_only && is_test_only(&function.attrs) {
                    continue;
                }
                function.attrs.clear();
                emit(
                    path,
                    format!("fn:{}", qualified(prefix, &function.sig.ident)),
                    &function,
                    body_nodes(&function.block),
                );
            }
            Item::Impl(block) => {
                if production_only && is_test_only(&block.attrs) {
                    continue;
                }
                let owner = qualified(prefix, type_name(&block.self_ty));
                for item in block.items {
                    match item {
                        ImplItem::Fn(mut function) => {
                            if production_only && is_test_only(&function.attrs) {
                                continue;
                            }
                            function.attrs.clear();
                            emit(
                                path,
                                format!("impl:{owner}::{}", function.sig.ident),
                                &function,
                                body_nodes(&function.block),
                            );
                        }
                        ImplItem::Const(mut constant) => {
                            if production_only && is_test_only(&constant.attrs) {
                                continue;
                            }
                            constant.attrs.clear();
                            emit(
                                path,
                                format!("const:{owner}::{}", constant.ident),
                                &constant,
                                expression_nodes(&constant.expr),
                            );
                        }
                        _ => {}
                    }
                }
            }
            Item::Struct(mut structure) => {
                if production_only && is_test_only(&structure.attrs) {
                    continue;
                }
                structure.attrs.clear();
                let owner = qualified(prefix, &structure.ident);
                for field in &structure.fields {
                    if production_only && is_test_only(&field.attrs) {
                        continue;
                    }
                    let Some(name) = &field.ident else { continue };
                    emit(
                        path,
                        format!("field:{owner}::{name}"),
                        field,
                        field_nodes(field),
                    );
                }
                emit(path, format!("struct:{owner}"), &structure, hex(""));
            }
            Item::Enum(mut enumeration) => {
                if production_only && is_test_only(&enumeration.attrs) {
                    continue;
                }
                enumeration.attrs.clear();
                let owner = qualified(prefix, &enumeration.ident);
                for variant in &enumeration.variants {
                    let nodes = variant
                        .discriminant
                        .as_ref()
                        .map(|(_, expression)| expression_nodes(expression))
                        .unwrap_or_else(|| hex(""));
                    emit(
                        path,
                        format!("variant:{owner}::{}", variant.ident),
                        variant,
                        nodes,
                    );
                }
                emit(path, format!("enum:{owner}"), &enumeration, hex(""));
            }
            Item::Const(mut constant) => {
                if production_only && is_test_only(&constant.attrs) {
                    continue;
                }
                constant.attrs.clear();
                emit(
                    path,
                    format!("const:{}", qualified(prefix, &constant.ident)),
                    &constant,
                    expression_nodes(&constant.expr),
                );
            }
            Item::Static(mut static_item) => {
                if production_only && is_test_only(&static_item.attrs) {
                    continue;
                }
                static_item.attrs.clear();
                emit(
                    path,
                    format!("static:{}", qualified(prefix, &static_item.ident)),
                    &static_item,
                    expression_nodes(&static_item.expr),
                );
            }
            Item::Mod(module) => {
                if production_only && is_test_only(&module.attrs) {
                    continue;
                }
                if let Some((_, items)) = module.content {
                    emit_items(
                        path,
                        items,
                        &qualified(prefix, module.ident),
                        production_only,
                    );
                }
            }
            _ => {}
        }
    }
}

fn emit_file(path: &Path, production_only: bool) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let file = syn::parse_file(&source).map_err(|error| format!("{}: {error}", path.display()))?;
    emit_items(path, file.items, "", production_only);
    Ok(())
}

fn main() {
    let mut arguments = std::env::args_os().skip(1).peekable();
    let production_only = arguments
        .next_if(|argument| argument == "--production-only")
        .is_some();
    let paths: Vec<_> = arguments.collect();
    if paths.is_empty() {
        eprintln!("usage: orphan-ast-audit [--production-only] RUST_SOURCE ...");
        std::process::exit(2);
    }
    for path in paths {
        if let Err(error) = emit_file(Path::new(&path), production_only) {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statement_position_macros_are_semantic_nodes() {
        let block: syn::Block = syn::parse_str("{ panic!(\"boundary\"); }").unwrap();
        let mut nodes = BodyNodes::default();
        nodes.visit_block(&block);
        assert!(nodes
            .nodes
            .iter()
            .any(|node| node.starts_with("MACRO\tpanic")));
    }
}
