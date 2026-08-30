//! Generic signed-index landmine detector for J2ME Rust transliterations.
//!
//! The recurring runtime-crash class across the ports: an array indexed by a
//! SIGNED value that can be `-1` (or `0xFFFF`-as-`i16`) cast to `usize` ->
//! `usize::MAX` -> out-of-bounds panic. These are FAITHFUL reads that crash
//! only because upstream state (often a recorded "collapse") left a sentinel
//! `-1` in a def/row cell. A per-node crosswalk cannot see across that
//! boundary, so this dedicated scanner is the instrument.
//!
//! It parses production Rust with `syn` and reports every `arr[<index> as
//! usize]` (and `arr[<tainted-local>]`) whose `<index>` is a signed integer
//! expression NOT provably `>= 0` in its lexical scope. A dominating guard
//! (`if idx >= 0`, `if idx != -1`, `if idx < 0 { return }`, `.max(0)`,
//! `.abs()`), an unsigned cast, or a for-counter over the array's own range
//! suppresses the site.
//!
//! Output is TSV, one finding per line:
//!   file <TAB> line <TAB> index_expr <TAB> source_kind <TAB> source_detail <TAB> annotated(0|1)
//! `annotated` is 1 when an `// index-safe: <reason>` comment sits on the line
//! or the nearest line above it (the inline half of the ratchet; the allowlist
//! TOML is the other half, applied by the Python driver).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use quote::ToTokens;
use syn::spanned::Spanned;
use syn::{Attribute, BinOp, Block, Expr, ImplItem, Item, Lit, Pat, Stmt, UnOp};

/// The lexical facts known at a program point.
#[derive(Default, Clone)]
struct Scope {
    /// Normalized expression texts proven `>= 0` here (guarded, `.max(0)`, ...).
    nonneg: HashSet<String>,
    /// Local name -> origin description of a value that flowed from a signed
    /// read (the taint that makes a later `arr[local]` a landmine).
    tainted: HashMap<String, String>,
}

impl Scope {
    fn with_nonneg(&self, facts: impl IntoIterator<Item = String>) -> Scope {
        let mut child = self.clone();
        child.nonneg.extend(facts);
        child
    }
}

/// How the signed value reached the index.
enum Source {
    /// A field/array/row read like `row[4]` or `self.combat_target`.
    Read(String),
    /// A cast to a signed type: `(x & 0xFFFF) as i16` — `-1` for `0xFFFF`.
    SignedCast(String),
    /// A local bound earlier to one of the above.
    TaintedLocal(String, String),
}

impl Source {
    fn kind(&self) -> &'static str {
        match self {
            Source::Read(_) => "signed-read",
            Source::SignedCast(_) => "signed-cast",
            Source::TaintedLocal(..) => "tainted-local",
        }
    }

    fn detail(&self) -> String {
        match self {
            Source::Read(t) | Source::SignedCast(t) => t.clone(),
            Source::TaintedLocal(name, origin) => format!("{name} = {origin}"),
        }
    }
}

struct Finding {
    line: usize,
    /// The whole indexing expression, e.g. `g.canvas.def_e[r[4] as usize]`.
    expr: String,
    source: Source,
}

/// The last path segment of an indexed base, e.g. `def_e` for
/// `g.canvas.def_e[..]` — the human-recognizable name of the table at risk.
fn base_name(base: &Expr) -> String {
    match peel(base) {
        Expr::Field(field) => match &field.member {
            syn::Member::Named(name) => name.to_string(),
            syn::Member::Unnamed(index) => index.index.to_string(),
        },
        Expr::Path(path) => path
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default(),
        Expr::Index(inner) => base_name(&inner.expr),
        Expr::MethodCall(call) => call.method.to_string(),
        other => display_text(other),
    }
}

struct FileScan {
    findings: Vec<Finding>,
}

fn normalize(expr: &impl ToTokens) -> String {
    expr.to_token_stream()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

fn display_text(expr: &impl ToTokens) -> String {
    expr.to_token_stream().to_string()
}

fn peel(mut expr: &Expr) -> &Expr {
    loop {
        match expr {
            Expr::Paren(inner) => expr = &inner.expr,
            Expr::Group(inner) => expr = &inner.expr,
            _ => return expr,
        }
    }
}

/// `-1`, `0`, `5` -> the integer value; handles the unary-neg literal shape.
fn int_literal(expr: &Expr) -> Option<i64> {
    match peel(expr) {
        Expr::Lit(lit) => match &lit.lit {
            Lit::Int(value) => value.base10_parse::<i64>().ok(),
            _ => None,
        },
        Expr::Unary(unary) if matches!(unary.op, UnOp::Neg(_)) => {
            int_literal(&unary.expr).map(|v| -v)
        }
        _ => None,
    }
}

fn is_unsigned_type(ty: &syn::Type) -> bool {
    matches!(
        normalize(ty).as_str(),
        "usize" | "u8" | "u16" | "u32" | "u64" | "u128"
    )
}

fn is_signed_type(ty: &syn::Type) -> bool {
    matches!(
        normalize(ty).as_str(),
        "isize" | "i8" | "i16" | "i32" | "i64" | "i128"
    )
}

/// Does `expr` (the value about to be used as an index) carry a signed value
/// that is NOT provably `>= 0` in `scope`? Returns the taint source if so.
///
/// The crash class is a stored cell that can hold the `-1` sentinel: a
/// field/array/row read, a masked short cast `... as iN`, or a local that was
/// bound to one of those. Method/function calls and pure arithmetic offsets are
/// deliberately NOT sources — a `.wrapping_mul(11)` layout offset or a
/// validated getter is not the sentinel crash we keep hitting, and treating it
/// as one buries the real landmines in noise.
fn risky_source(expr: &Expr, scope: &Scope) -> Option<Source> {
    let expr = peel(expr);

    // A dominating guard on this exact text wins over everything below.
    if scope.nonneg.contains(&normalize(expr)) {
        return None;
    }

    match expr {
        Expr::Lit(_) => int_literal(expr)
            .filter(|v| *v < 0)
            .map(|_| Source::SignedCast(display_text(expr))),
        Expr::Cast(cast) => {
            if is_signed_type(&cast.ty) {
                // `(x & 0xFFFF) as i16` — the classic `0xFFFF` -> `-1` landmine.
                Some(Source::SignedCast(display_text(expr)))
            } else {
                // `x as u16` truncates to a non-negative value; other casts are
                // not the sentinel class.
                None
            }
        }
        Expr::Index(_) | Expr::Field(_) => Some(Source::Read(display_text(expr))),
        Expr::Path(path) => {
            let name = normalize(path);
            scope
                .tainted
                .get(&name)
                .map(|origin| Source::TaintedLocal(name, origin.clone()))
        }
        _ => None,
    }
}

/// Extract the signed value that would flow into an array slot: the pre-`as
/// usize` inner if the index is an unsigned cast, else a bare tainted local.
fn index_candidate(index: &Expr, scope: &Scope) -> Option<Source> {
    match peel(index) {
        Expr::Cast(cast) if is_unsigned_type(&cast.ty) => risky_source(&cast.expr, scope),
        // A bare local index (`arr[local]`) is only risky if it was tainted; a
        // plain param/loop counter is left alone (it must already be usize).
        Expr::Path(_) => risky_source(index, scope),
        _ => None,
    }
}

// ---- guard-fact extraction -------------------------------------------------

/// `E` is proven `>= 0` when `cond` is TRUE.
fn facts_when_true(cond: &Expr) -> Vec<String> {
    match peel(cond) {
        Expr::Binary(bin) if matches!(bin.op, BinOp::And(_)) => {
            let mut facts = facts_when_true(&bin.left);
            facts.extend(facts_when_true(&bin.right));
            facts
        }
        Expr::Binary(bin) => atom_true(&bin.left, &bin.op, &bin.right)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// `E` is proven `>= 0` when `cond` is FALSE.
fn facts_when_false(cond: &Expr) -> Vec<String> {
    match peel(cond) {
        Expr::Binary(bin) if matches!(bin.op, BinOp::Or(_)) => {
            let mut facts = facts_when_false(&bin.left);
            facts.extend(facts_when_false(&bin.right));
            facts
        }
        Expr::Binary(bin) => atom_false(&bin.left, &bin.op, &bin.right)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn atom_true(left: &Expr, op: &BinOp, right: &Expr) -> Option<String> {
    let l = int_literal(left);
    let r = int_literal(right);
    match op {
        // E >= k (k >= 0), E > k (k >= -1), E != -1, E == k (k >= 0)
        BinOp::Ge(_) if r.is_some_and(|k| k >= 0) => Some(normalize(&peel(left))),
        BinOp::Gt(_) if r.is_some_and(|k| k >= -1) => Some(normalize(&peel(left))),
        BinOp::Ne(_) if r == Some(-1) => Some(normalize(&peel(left))),
        BinOp::Eq(_) if r.is_some_and(|k| k >= 0) => Some(normalize(&peel(left))),
        // reversed: k <= E (k >= 0), k < E (k >= -1)
        BinOp::Le(_) if l.is_some_and(|k| k >= 0) => Some(normalize(&peel(right))),
        BinOp::Lt(_) if l.is_some_and(|k| k >= -1) => Some(normalize(&peel(right))),
        _ => None,
    }
}

fn atom_false(left: &Expr, op: &BinOp, right: &Expr) -> Option<String> {
    let l = int_literal(left);
    let r = int_literal(right);
    match op {
        // (E < k) false -> E >= k (k >= 0); (E <= k) false -> E > k (k >= -1)
        BinOp::Lt(_) if r.is_some_and(|k| k >= 0) => Some(normalize(&peel(left))),
        BinOp::Le(_) if r.is_some_and(|k| k >= -1) => Some(normalize(&peel(left))),
        // (E == -1) false -> E != -1
        BinOp::Eq(_) if r == Some(-1) => Some(normalize(&peel(left))),
        // reversed: (k > E) false -> E >= k (k >= 0); (k >= E) false -> E > k
        BinOp::Gt(_) if l.is_some_and(|k| k >= 0) => Some(normalize(&peel(right))),
        BinOp::Ge(_) if l.is_some_and(|k| k >= -1) => Some(normalize(&peel(right))),
        _ => None,
    }
}

// ---- traversal -------------------------------------------------------------

fn block_diverges(block: &Block) -> bool {
    match block.stmts.last() {
        Some(Stmt::Expr(expr, _)) => expr_diverges(expr),
        Some(Stmt::Macro(mac)) => is_diverging_macro(&mac.mac.path),
        _ => false,
    }
}

fn expr_diverges(expr: &Expr) -> bool {
    match peel(expr) {
        Expr::Return(_) | Expr::Break(_) | Expr::Continue(_) => true,
        Expr::Macro(mac) => is_diverging_macro(&mac.mac.path),
        Expr::If(if_expr) => {
            block_diverges(&if_expr.then_branch)
                && if_expr
                    .else_branch
                    .as_ref()
                    .is_some_and(|(_, e)| expr_diverges(e))
        }
        Expr::Block(block) => block_diverges(&block.block),
        _ => false,
    }
}

fn is_diverging_macro(path: &syn::Path) -> bool {
    path.segments.last().is_some_and(|seg| {
        matches!(
            seg.ident.to_string().as_str(),
            "panic" | "unreachable" | "todo" | "unimplemented"
        )
    })
}

fn walk_block(scan: &mut FileScan, block: &Block, incoming: &Scope) {
    let mut scope = incoming.clone();
    for stmt in &block.stmts {
        match stmt {
            Stmt::Local(local) => {
                if let Some(init) = &local.init {
                    walk_expr(scan, &init.expr, &scope);
                    if let Some(name) = simple_ident(&local.pat) {
                        classify_binding(&name, &init.expr, &mut scope);
                    }
                }
            }
            Stmt::Item(Item::Fn(function)) => {
                if !is_test_only(&function.attrs) {
                    walk_block(scan, &function.block, &Scope::default());
                }
            }
            Stmt::Expr(expr, _) => {
                walk_expr(scan, expr, &scope);
                if let Expr::If(if_expr) = peel(expr) {
                    if if_expr.else_branch.is_none() && block_diverges(&if_expr.then_branch) {
                        scope.nonneg.extend(facts_when_false(&if_expr.cond));
                    }
                }
            }
            _ => {}
        }
    }
}

fn simple_ident(pat: &Pat) -> Option<String> {
    match pat {
        Pat::Ident(ident) if ident.subpat.is_none() => Some(ident.ident.to_string()),
        _ => None,
    }
}

fn classify_binding(name: &str, init: &Expr, scope: &mut Scope) {
    // A fresh binding shadows any earlier fact/taint for the name.
    scope.nonneg.remove(name);
    scope.tainted.remove(name);

    let value = peel(init);
    match value {
        Expr::Cast(cast) if is_unsigned_type(&cast.ty) => {
            if let Some(source) = risky_source(&cast.expr, scope) {
                scope.tainted.insert(
                    name.to_string(),
                    format!("{} (via as usize)", source.detail()),
                );
            } else {
                scope.nonneg.insert(name.to_string());
            }
        }
        _ => {
            if let Some(source) = risky_source(value, scope) {
                scope.tainted.insert(name.to_string(), source.detail());
            }
        }
    }
}

fn walk_expr(scan: &mut FileScan, expr: &Expr, scope: &Scope) {
    match expr {
        Expr::Index(index) => {
            if let Some(source) = index_candidate(&index.index, scope) {
                scan.findings.push(Finding {
                    line: index.index.span().start().line,
                    expr: format!("{}[{}]", base_name(&index.expr), display_text(&index.index)),
                    source,
                });
            }
            walk_expr(scan, &index.expr, scope);
            walk_expr(scan, &index.index, scope);
        }
        Expr::Binary(bin) if matches!(bin.op, BinOp::And(_)) => {
            walk_expr(scan, &bin.left, scope);
            walk_expr(
                scan,
                &bin.right,
                &scope.with_nonneg(facts_when_true(&bin.left)),
            );
        }
        Expr::Binary(bin) if matches!(bin.op, BinOp::Or(_)) => {
            walk_expr(scan, &bin.left, scope);
            walk_expr(
                scan,
                &bin.right,
                &scope.with_nonneg(facts_when_false(&bin.left)),
            );
        }
        Expr::Binary(bin) => {
            walk_expr(scan, &bin.left, scope);
            walk_expr(scan, &bin.right, scope);
        }
        Expr::If(if_expr) => {
            walk_expr(scan, &if_expr.cond, scope);
            walk_block(
                scan,
                &if_expr.then_branch,
                &scope.with_nonneg(facts_when_true(&if_expr.cond)),
            );
            if let Some((_, else_branch)) = &if_expr.else_branch {
                walk_expr(
                    scan,
                    else_branch,
                    &scope.with_nonneg(facts_when_false(&if_expr.cond)),
                );
            }
        }
        Expr::ForLoop(for_loop) => {
            walk_expr(scan, &for_loop.expr, scope);
            let mut child = scope.clone();
            if let (Some(name), true) = (
                simple_ident(&for_loop.pat),
                range_starts_nonneg(&for_loop.expr),
            ) {
                child.nonneg.insert(name);
            }
            walk_block(scan, &for_loop.body, &child);
        }
        Expr::While(while_expr) => {
            walk_expr(scan, &while_expr.cond, scope);
            walk_block(
                scan,
                &while_expr.body,
                &scope.with_nonneg(facts_when_true(&while_expr.cond)),
            );
        }
        Expr::Loop(loop_expr) => walk_block(scan, &loop_expr.body, scope),
        Expr::Block(block) => walk_block(scan, &block.block, scope),
        Expr::Unsafe(block) => walk_block(scan, &block.block, scope),
        Expr::Match(match_expr) => {
            walk_expr(scan, &match_expr.expr, scope);
            for arm in &match_expr.arms {
                walk_expr(scan, &arm.body, scope);
            }
        }
        Expr::MethodCall(call) => {
            walk_expr(scan, &call.receiver, scope);
            for arg in &call.args {
                walk_expr(scan, arg, scope);
            }
        }
        Expr::Call(call) => {
            walk_expr(scan, &call.func, scope);
            for arg in &call.args {
                walk_expr(scan, arg, scope);
            }
        }
        Expr::Cast(cast) => walk_expr(scan, &cast.expr, scope),
        Expr::Paren(inner) => walk_expr(scan, &inner.expr, scope),
        Expr::Group(inner) => walk_expr(scan, &inner.expr, scope),
        Expr::Reference(inner) => walk_expr(scan, &inner.expr, scope),
        Expr::Unary(unary) => walk_expr(scan, &unary.expr, scope),
        Expr::Field(field) => walk_expr(scan, &field.base, scope),
        Expr::Assign(assign) => {
            walk_expr(scan, &assign.left, scope);
            walk_expr(scan, &assign.right, scope);
        }
        Expr::Let(let_expr) => walk_expr(scan, &let_expr.expr, scope),
        Expr::Return(ret) => {
            if let Some(value) = &ret.expr {
                walk_expr(scan, value, scope);
            }
        }
        Expr::Break(brk) => {
            if let Some(value) = &brk.expr {
                walk_expr(scan, value, scope);
            }
        }
        Expr::Range(range) => {
            if let Some(start) = &range.start {
                walk_expr(scan, start, scope);
            }
            if let Some(end) = &range.end {
                walk_expr(scan, end, scope);
            }
        }
        Expr::Tuple(tuple) => tuple.elems.iter().for_each(|e| walk_expr(scan, e, scope)),
        Expr::Array(array) => array.elems.iter().for_each(|e| walk_expr(scan, e, scope)),
        Expr::Repeat(repeat) => {
            walk_expr(scan, &repeat.expr, scope);
            walk_expr(scan, &repeat.len, scope);
        }
        Expr::Struct(structure) => {
            for field in &structure.fields {
                walk_expr(scan, &field.expr, scope);
            }
            if let Some(rest) = &structure.rest {
                walk_expr(scan, rest, scope);
            }
        }
        Expr::Closure(closure) => walk_expr(scan, &closure.body, scope),
        Expr::Try(try_expr) => walk_expr(scan, &try_expr.expr, scope),
        Expr::Await(await_expr) => walk_expr(scan, &await_expr.base, scope),
        _ => {}
    }
}

fn range_starts_nonneg(iter: &Expr) -> bool {
    match peel(iter) {
        Expr::Range(range) => match &range.start {
            None => true,
            Some(start) => int_literal(start).is_some_and(|v| v >= 0),
        },
        _ => false,
    }
}

// ---- item iteration --------------------------------------------------------

fn is_test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let syn::Meta::List(meta) = &attr.meta else {
            return false;
        };
        meta.path.is_ident("cfg") && meta.tokens.to_string().contains("test")
    })
}

fn scan_items(scan: &mut FileScan, items: &[Item]) {
    for item in items {
        match item {
            Item::Fn(function) if !is_test_only(&function.attrs) => {
                walk_block(scan, &function.block, &Scope::default());
            }
            Item::Impl(block) if !is_test_only(&block.attrs) => {
                for impl_item in &block.items {
                    if let ImplItem::Fn(function) = impl_item {
                        if !is_test_only(&function.attrs) {
                            walk_block(scan, &function.block, &Scope::default());
                        }
                    }
                }
            }
            Item::Mod(module) if !is_test_only(&module.attrs) && module.ident != "tests" => {
                if let Some((_, items)) = &module.content {
                    scan_items(scan, items);
                }
            }
            _ => {}
        }
    }
}

fn annotated_at(lines: &[&str], line: usize) -> bool {
    if line == 0 {
        return false;
    }
    let this = lines.get(line - 1).copied().unwrap_or("");
    if this.contains("index-safe:") {
        return true;
    }
    // The nearest non-blank line above.
    let mut cursor = line.saturating_sub(1);
    while cursor >= 1 {
        let text = lines.get(cursor - 1).copied().unwrap_or("").trim();
        if text.is_empty() {
            cursor -= 1;
            continue;
        }
        return text.contains("index-safe:");
    }
    false
}

fn scan_file(path: &Path) -> Result<(), String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut scan = FileScan {
        findings: Vec::new(),
    };
    scan_items(&mut scan, &file.items);

    let lines: Vec<&str> = source.lines().collect();
    let mut seen: HashSet<(usize, String)> = HashSet::new();
    for finding in &scan.findings {
        let normalized: String = finding
            .expr
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if !seen.insert((finding.line, normalized)) {
            continue;
        }
        let annotated = u8::from(annotated_at(&lines, finding.line));
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            path.display(),
            finding.line,
            finding.expr,
            finding.source.kind(),
            finding.source.detail(),
            annotated
        );
    }
    Ok(())
}

// ---- self-test -------------------------------------------------------------

fn scan_source_for_test(source: &str) -> Vec<(String, String)> {
    let file = syn::parse_file(source).expect("fixture parses");
    let mut scan = FileScan {
        findings: Vec::new(),
    };
    scan_items(&mut scan, &file.items);
    scan.findings
        .iter()
        .map(|f| (f.expr.clone(), f.source.kind().to_string()))
        .collect()
}

fn self_test() -> i32 {
    let flag_cases = [
        // the live D29 def_e[row[4]] family
        ("fn f(g: &G, r: &[i32]) -> i32 { g.canvas.def_e[r[4] as usize][13] }", "def_e[r[4]]"),
        // the container_contents (short) & 0xFFFF as i16 family
        (
            "fn f(g: &G, row: &[i32]) -> i32 { g.canvas.container_contents[(row[17] & 0xFFFF) as i16 as usize] }",
            "0xFFFF as i16",
        ),
        // a tainted local carrying a signed read to a bare index
        (
            "fn f(g: &G, r: &[i32]) -> V { let c = r[4] as usize; g.canvas.def_e[c].clone() }",
            "tainted local",
        ),
        // signed field read
        (
            "fn f(g: &G) -> i32 { g.canvas.def_e[g.canvas.combat_target as usize][0] }",
            "field read",
        ),
    ];
    for (source, label) in flag_cases {
        if scan_source_for_test(source).is_empty() {
            eprintln!("self-test FAIL: expected a landmine for {label}: {source}");
            return 1;
        }
    }

    let clean_cases = [
        // same-condition && guard: `if def >= 0 && def_r[def as usize]...`
        "fn f(g: &G) { for slot in 0..n { let def = g.canvas.inventory[2][slot][4]; if def >= 0 && g.canvas.def_r[def as usize][6] == 20 { g.x += 1; } } }",
        // || short-circuit: the auto-target != -1 guard
        "fn f(g: &G, w: usize) -> bool { g.canvas.player_state[w] == -1 || g.canvas.inventory[4][g.canvas.player_state[w] as usize][4] == -1 }",
        // early-return negative guard dominating the rest
        "fn f(g: &G, r: &[i32]) -> i32 { if r[4] < 0 { return 0; } g.canvas.def_e[r[4] as usize][0] }",
        // .max(0) clamp
        "fn f(g: &G, r: &[i32]) -> i32 { g.canvas.def_e[(r[4]).max(0) as usize][0] }",
        // unsigned cast is never the -1 landmine
        "fn f(g: &G, r: &[i32]) -> i32 { g.canvas.def_e[(r[4] & 0xFFFF) as u16 as usize][0] }",
        // plain for-counter over a range is not flagged
        "fn f(g: &G) { for i in 0..g.canvas.def_e.len() { let _ = g.canvas.def_e[i as usize]; } }",
    ];
    for source in clean_cases {
        let findings = scan_source_for_test(source);
        if !findings.is_empty() {
            eprintln!("self-test FAIL: expected NO landmine but got {findings:?}: {source}");
            return 1;
        }
    }

    println!(
        "index-safety self-test OK: {} seeded landmines flagged, {} guarded/safe sites suppressed",
        flag_cases.len(),
        clean_cases.len()
    );
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--self-test") {
        std::process::exit(self_test());
    }
    if args.is_empty() {
        eprintln!("usage: j2me-index-safety [--self-test] RUST_SOURCE ...");
        std::process::exit(2);
    }
    for arg in &args {
        if let Err(error) = scan_file(Path::new(arg)) {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_test_passes() {
        assert_eq!(self_test(), 0);
    }

    #[test]
    fn nested_or_guard_suppresses_both_indexes() {
        let source = "fn f(g: &G, w: usize) -> bool { g.canvas.player_state[w] == -1 || g.canvas.inventory[4][g.canvas.player_state[w] as usize][4] == -1 || g.canvas.def_u[g.canvas.inventory[4][g.canvas.player_state[w] as usize][4] as usize][6] != 0 }";
        assert!(scan_source_for_test(source).is_empty());
    }
}
