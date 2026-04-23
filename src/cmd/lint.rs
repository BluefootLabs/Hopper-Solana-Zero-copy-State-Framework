//! `hopper lint` - account-relationship checker for Hopper programs.
//!
//! Walks every `.rs` file under the project `src/` tree, parses
//! `#[hopper::context]` / `#[accounts]` structs with `syn`, extracts
//! account constraints, and emits a cross-context relationship graph
//! in one of four formats (ASCII, Mermaid, DOT, JSON) alongside a
//! diagnostic list that calls out:
//!
//! - Accounts declared writable (`mut`) without an `#[invariant(...)]`
//!   tag on the containing context. Writable accounts should carry at
//!   least one named invariant so the receipt chain can surface
//!   meaningful failure attribution.
//! - `has_one` references that do not resolve to a sibling field in
//!   the same context.
//! - `dup` references that do not resolve.
//! - Fields with `init` but no `payer` or `space` (Anchor-parity fatal
//!   check, restated here because the context macro error is deep in
//!   proc-macro land and not all IDEs surface it promptly).
//! - `seeds = [...]` without `bump`.
//! - Token-2022 `extensions::*` constraints without a matching
//!   `token::token_program` or `mint::token_program` override pinning
//!   the account to Token-2022.
//!
//! The graph models relationships between contexts. An edge
//! `Deposit -> vault` means the `Deposit` context declares a field
//! named `vault`; edges carry the field's constraints as a
//! space-separated label. Cross-context edges (`Deposit.vault`
//! referencing the same account type `Vault` declared with
//! `#[hopper::state]`) are drawn when we can resolve the type.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use syn::{spanned::Spanned, Attribute, File, Item, ItemStruct, Meta, Type};

pub fn cmd_lint(args: &[String]) {
    if args.iter().any(|a| matches!(a.as_str(), "--help" | "-h")) {
        print_usage();
        return;
    }

    let mut project = PathBuf::from(".");
    let mut format = OutputFormat::Ascii;
    let mut fail_on_warn = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" => {
                i += 1;
                project = PathBuf::from(args.get(i).cloned().unwrap_or_default());
            }
            "--graph" => {
                i += 1;
                format = match args.get(i).map(String::as_str) {
                    Some("ascii") => OutputFormat::Ascii,
                    Some("mermaid") => OutputFormat::Mermaid,
                    Some("dot") => OutputFormat::Dot,
                    Some("json") => OutputFormat::Json,
                    Some(other) => {
                        eprintln!("unknown --graph format: {other}");
                        eprintln!("allowed: ascii, mermaid, dot, json");
                        process::exit(1);
                    }
                    None => {
                        eprintln!("`--graph` requires a value");
                        process::exit(1);
                    }
                };
            }
            "--fail-on-warn" => fail_on_warn = true,
            other => {
                eprintln!("unknown lint flag: {other}");
                print_usage();
                process::exit(1);
            }
        }
        i += 1;
    }

    let report = match lint_project(&project) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("hopper lint failed: {e}");
            process::exit(1);
        }
    };

    match format {
        OutputFormat::Ascii => render_ascii(&report),
        OutputFormat::Mermaid => render_mermaid(&report),
        OutputFormat::Dot => render_dot(&report),
        OutputFormat::Json => render_json(&report),
    }

    for d in &report.diagnostics {
        println!("{}", d.format());
    }

    let has_errors = report.diagnostics.iter().any(|d| d.level == Level::Error);
    let has_warns = report.diagnostics.iter().any(|d| d.level == Level::Warn);
    if has_errors || (fail_on_warn && has_warns) {
        process::exit(1);
    }
}

fn print_usage() {
    eprintln!("Usage: hopper lint [options]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --project <path>       Project root (default: current dir)");
    eprintln!("  --graph <format>       Output format: ascii | mermaid | dot | json");
    eprintln!("                         default: ascii");
    eprintln!("  --fail-on-warn         Exit 1 on warnings in addition to errors");
    eprintln!();
    eprintln!("Lints the cross-context account relationship graph in a Hopper");
    eprintln!("project. Prints the graph and every diagnostic found. Exits");
    eprintln!("non-zero when an error-level diagnostic is surfaced.");
}

enum OutputFormat {
    Ascii,
    Mermaid,
    Dot,
    Json,
}

// ---- report types ----------------------------------------------------------

struct Report {
    contexts: Vec<ContextInfo>,
    states: Vec<String>,
    diagnostics: Vec<Diagnostic>,
}

struct ContextInfo {
    name: String,
    fields: Vec<FieldInfo>,
    source_file: PathBuf,
    has_validate_hook: bool,
    has_invariant_attr: bool,
}

struct FieldInfo {
    name: String,
    type_name: String,
    constraints: Vec<String>,
    is_mut: bool,
    is_signer: bool,
    is_init: bool,
    has_space: bool,
    has_payer: bool,
    has_seeds: bool,
    has_bump: bool,
    has_one_targets: Vec<String>,
    dup_target: Option<String>,
    has_extension_constraint: bool,
    pins_token_2022: bool,
}

struct Diagnostic {
    level: Level,
    context: String,
    field: Option<String>,
    message: String,
    source_file: PathBuf,
}

#[derive(Eq, PartialEq)]
enum Level {
    Error,
    Warn,
    Info,
}

impl Diagnostic {
    fn format(&self) -> String {
        let level = match self.level {
            Level::Error => "ERROR",
            Level::Warn => "WARN ",
            Level::Info => "INFO ",
        };
        let where_ = match &self.field {
            Some(f) => format!("{}.{}", self.context, f),
            None => self.context.clone(),
        };
        format!(
            "{level} {:<28} {} ({})",
            where_,
            self.message,
            self.source_file.display()
        )
    }
}

// ---- driver ----------------------------------------------------------------

fn lint_project(root: &Path) -> Result<Report, String> {
    let src_dirs = [
        root.join("src"),
        root.join("programs"),
    ];
    let mut files: Vec<PathBuf> = Vec::new();
    for dir in &src_dirs {
        collect_rs_files(dir, &mut files);
    }
    if files.is_empty() {
        return Err(format!(
            "no Rust source files found under {} or {}",
            src_dirs[0].display(),
            src_dirs[1].display()
        ));
    }

    let mut contexts: Vec<ContextInfo> = Vec::new();
    let mut states: Vec<String> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    for file_path in &files {
        let text = fs::read_to_string(file_path)
            .map_err(|e| format!("read {}: {e}", file_path.display()))?;
        let parsed: File = match syn::parse_file(&text) {
            Ok(f) => f,
            Err(e) => {
                diagnostics.push(Diagnostic {
                    level: Level::Warn,
                    context: file_path.display().to_string(),
                    field: None,
                    message: format!("skipped (syn parse failed: {e})"),
                    source_file: file_path.clone(),
                });
                continue;
            }
        };
        for item in parsed.items {
            if let Item::Struct(s) = item {
                match classify(&s) {
                    Classify::Context => {
                        let ctx = extract_context(&s, file_path);
                        contexts.push(ctx);
                    }
                    Classify::State => {
                        states.push(s.ident.to_string());
                    }
                    Classify::Neither => {}
                }
            }
        }
    }

    for ctx in &contexts {
        run_diagnostics(ctx, &mut diagnostics);
    }

    Ok(Report { contexts, states, diagnostics })
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                if name.starts_with('.') || name == "target" {
                    continue;
                }
            }
            collect_rs_files(&p, out);
        } else if p.extension().map(|e| e == "rs").unwrap_or(false) {
            out.push(p);
        }
    }
}

enum Classify {
    Context,
    State,
    Neither,
}

fn classify(s: &ItemStruct) -> Classify {
    for attr in &s.attrs {
        let path = attr.path();
        let Some(last) = path.segments.last() else { continue };
        match last.ident.to_string().as_str() {
            "context" | "accounts" | "hopper_context" => return Classify::Context,
            "state" | "account" | "hopper_state" => return Classify::State,
            _ => {}
        }
    }
    Classify::Neither
}

fn extract_context(s: &ItemStruct, source_file: &Path) -> ContextInfo {
    let name = s.ident.to_string();
    let has_validate_hook = s.attrs.iter().any(|a| a.path().is_ident("validate"));
    let has_invariant_attr = s.attrs.iter().any(|a| a.path().is_ident("invariant"));

    let mut fields: Vec<FieldInfo> = Vec::new();
    if let syn::Fields::Named(named) = &s.fields {
        for f in &named.named {
            let Some(ident) = &f.ident else { continue };
            let type_name = type_to_string(&f.ty);
            let mut info = FieldInfo {
                name: ident.to_string(),
                type_name,
                constraints: Vec::new(),
                is_mut: false,
                is_signer: false,
                is_init: false,
                has_space: false,
                has_payer: false,
                has_seeds: false,
                has_bump: false,
                has_one_targets: Vec::new(),
                dup_target: None,
                has_extension_constraint: false,
                pins_token_2022: false,
            };
            for attr in &f.attrs {
                if attr.path().is_ident("signer") {
                    info.is_signer = true;
                    info.constraints.push("signer".into());
                } else if attr.path().is_ident("account") {
                    scrape_account_attr(attr, &mut info);
                }
            }
            fields.push(info);
        }
    }

    ContextInfo {
        name,
        fields,
        source_file: source_file.to_path_buf(),
        has_validate_hook,
        has_invariant_attr,
    }
}

fn scrape_account_attr(attr: &Attribute, info: &mut FieldInfo) {
    let _ = attr.parse_nested_meta(|meta| {
        let ident = meta.path.get_ident().map(|i| i.to_string());
        // Handle two-segment paths (token::mint, extensions::non_transferable, ...).
        if meta.path.segments.len() == 2 {
            let ns = meta.path.segments[0].ident.to_string();
            let key = meta.path.segments[1].ident.to_string();
            let joined = format!("{ns}::{key}");
            info.constraints.push(joined.clone());
            if ns == "token" && key == "token_program" {
                let _ = meta.value().and_then(|v| v.parse::<syn::Expr>()).map(|e| {
                    let s = quote_to_string(&e);
                    if s.contains("TOKEN_2022_PROGRAM_ID") {
                        info.pins_token_2022 = true;
                    }
                });
            }
            if ns == "mint" && key == "token_program" {
                let _ = meta.value().and_then(|v| v.parse::<syn::Expr>()).map(|e| {
                    let s = quote_to_string(&e);
                    if s.contains("TOKEN_2022_PROGRAM_ID") {
                        info.pins_token_2022 = true;
                    }
                });
            }
            if ns == "extensions" {
                info.has_extension_constraint = true;
            }
            // Eat remaining tokens so nested parser does not error.
            let _ = meta.value().and_then(|v| v.parse::<syn::Expr>());
            return Ok(());
        }
        // Three-segment extension paths: extensions::transfer_hook::authority.
        if meta.path.segments.len() == 3
            && meta.path.segments[0].ident == "extensions"
        {
            info.has_extension_constraint = true;
            let joined = format!(
                "extensions::{}::{}",
                meta.path.segments[1].ident,
                meta.path.segments[2].ident
            );
            info.constraints.push(joined);
            let _ = meta.value().and_then(|v| v.parse::<syn::Expr>());
            return Ok(());
        }
        let Some(name) = ident else {
            return Ok(());
        };
        match name.as_str() {
            "mut" => {
                info.is_mut = true;
                info.constraints.push("mut".into());
            }
            "signer" => {
                info.is_signer = true;
                info.constraints.push("signer".into());
            }
            "init" => {
                info.is_init = true;
                info.constraints.push("init".into());
            }
            "space" => {
                info.has_space = true;
                info.constraints.push("space".into());
                let _ = meta.value().and_then(|v| v.parse::<syn::Expr>());
            }
            "payer" => {
                info.has_payer = true;
                info.constraints.push("payer".into());
                let _ = meta.value().and_then(|v| v.parse::<syn::Ident>());
            }
            "seeds" => {
                info.has_seeds = true;
                info.constraints.push("seeds".into());
                // Consume the =[...] value.
                let content;
                if let Ok(eq) = meta.value() {
                    let _ = syn::bracketed!(content in eq);
                    let _ = content
                        .parse_terminated(<syn::Expr as syn::parse::Parse>::parse, syn::Token![,]);
                }
            }
            "seeds_fn" => {
                info.has_seeds = true;
                info.constraints.push("seeds_fn".into());
                let _ = meta.value().and_then(|v| v.parse::<syn::Expr>());
            }
            "bump" => {
                info.has_bump = true;
                info.constraints.push("bump".into());
                if meta.input.peek(syn::Token![=]) {
                    let _ = meta.value().and_then(|v| v.parse::<syn::Expr>());
                }
            }
            "has_one" => {
                if let Ok(ident) = meta.value().and_then(|v| v.parse::<syn::Ident>()) {
                    info.has_one_targets.push(ident.to_string());
                    info.constraints.push(format!("has_one={}", ident));
                }
            }
            "dup" => {
                if let Ok(ident) = meta.value().and_then(|v| v.parse::<syn::Ident>()) {
                    info.dup_target = Some(ident.to_string());
                    info.constraints.push(format!("dup={}", ident));
                }
            }
            "owner" | "address" | "close" | "realloc" | "zero" | "sweep" => {
                info.constraints.push(name);
                if meta.input.peek(syn::Token![=]) {
                    let _ = meta.value().and_then(|v| v.parse::<syn::Expr>());
                }
            }
            "constraint" => {
                info.constraints.push("constraint".into());
                let _ = meta.value().and_then(|v| v.parse::<syn::Expr>());
            }
            other => {
                info.constraints.push(other.to_string());
                if meta.input.peek(syn::Token![=]) {
                    let _ = meta.value().and_then(|v| v.parse::<syn::Expr>());
                }
            }
        }
        Ok(())
    });
}

fn quote_to_string(expr: &syn::Expr) -> String {
    use quote::ToTokens;
    let mut s = String::new();
    expr.to_tokens(&mut proc_macro2::TokenStream::new());
    let ts = expr.to_token_stream();
    s.push_str(&ts.to_string());
    s
}

fn type_to_string(ty: &Type) -> String {
    use quote::ToTokens;
    ty.to_token_stream().to_string()
}

fn run_diagnostics(ctx: &ContextInfo, out: &mut Vec<Diagnostic>) {
    let sibling_names: std::collections::BTreeSet<String> =
        ctx.fields.iter().map(|f| f.name.clone()).collect();
    for f in &ctx.fields {
        // init without payer or space.
        if f.is_init && !f.has_payer {
            out.push(Diagnostic {
                level: Level::Error,
                context: ctx.name.clone(),
                field: Some(f.name.clone()),
                message: "`init` requires `payer = <field>`".into(),
                source_file: ctx.source_file.clone(),
            });
        }
        if f.is_init && !f.has_space {
            out.push(Diagnostic {
                level: Level::Error,
                context: ctx.name.clone(),
                field: Some(f.name.clone()),
                message: "`init` requires `space = <expr>`".into(),
                source_file: ctx.source_file.clone(),
            });
        }
        // seeds without bump.
        if f.has_seeds && !f.has_bump {
            out.push(Diagnostic {
                level: Level::Error,
                context: ctx.name.clone(),
                field: Some(f.name.clone()),
                message: "`seeds` or `seeds_fn` requires `bump` (inferred or stored)".into(),
                source_file: ctx.source_file.clone(),
            });
        }
        // has_one references.
        for target in &f.has_one_targets {
            if !sibling_names.contains(target) {
                out.push(Diagnostic {
                    level: Level::Error,
                    context: ctx.name.clone(),
                    field: Some(f.name.clone()),
                    message: format!("`has_one = {target}` does not match any sibling field"),
                    source_file: ctx.source_file.clone(),
                });
            }
        }
        // dup references.
        if let Some(target) = &f.dup_target {
            if !sibling_names.contains(target) {
                out.push(Diagnostic {
                    level: Level::Error,
                    context: ctx.name.clone(),
                    field: Some(f.name.clone()),
                    message: format!("`dup = {target}` does not match any sibling field"),
                    source_file: ctx.source_file.clone(),
                });
            }
        }
        // Token-2022 extension without token_program pin.
        if f.has_extension_constraint && !f.pins_token_2022 {
            out.push(Diagnostic {
                level: Level::Warn,
                context: ctx.name.clone(),
                field: Some(f.name.clone()),
                message: "`extensions::*` constraints should be paired with `token::token_program = TOKEN_2022_PROGRAM_ID` so a legacy SPL account does not short-circuit the TLV scan".into(),
                source_file: ctx.source_file.clone(),
            });
        }
        // Writable without context-level invariant or validate hook.
        if f.is_mut && !ctx.has_invariant_attr && !ctx.has_validate_hook {
            out.push(Diagnostic {
                level: Level::Warn,
                context: ctx.name.clone(),
                field: Some(f.name.clone()),
                message: "writable field declared with no `#[validate]` hook and no `#[invariant]` on the containing context; receipt-chain attribution will be empty on failure".into(),
                source_file: ctx.source_file.clone(),
            });
        }
    }
}

// ---- renderers -------------------------------------------------------------

fn render_ascii(r: &Report) {
    println!("Hopper account-relationship graph");
    println!("=================================");
    if r.contexts.is_empty() {
        println!("(no #[hopper::context] / #[accounts] structs found)");
        return;
    }
    for ctx in &r.contexts {
        println!();
        println!("[context] {}", ctx.name);
        if ctx.has_validate_hook {
            println!("  [validate hook present]");
        }
        for f in &ctx.fields {
            let role = if f.is_signer {
                "signer"
            } else if f.is_mut {
                "mut"
            } else {
                "ro"
            };
            let c = if f.constraints.is_empty() {
                String::new()
            } else {
                format!(" ({})", f.constraints.join(" "))
            };
            println!("  |- {} [{}] {}{}", f.name, role, f.type_name, c);
        }
    }
    println!();
    println!("[states] {}", r.states.join(", "));
}

fn render_mermaid(r: &Report) {
    println!("```mermaid");
    println!("graph TD");
    for ctx in &r.contexts {
        let ctx_id = node_id("ctx", &ctx.name);
        println!("  {ctx_id}[\"{}\"]:::ctx", ctx.name);
        for f in &ctx.fields {
            let field_id = node_id(&format!("f{}", ctx.name), &f.name);
            let role = if f.is_signer {
                "signer"
            } else if f.is_mut {
                "mut"
            } else {
                "ro"
            };
            println!("  {field_id}([\"{}<br/>{}<br/>{}\"])", f.name, f.type_name, role);
            let label = f.constraints.join(" ");
            println!("  {ctx_id} -->|\"{}\"| {field_id}", label);
            // Edge to the state type if we recognize it.
            if r.states.iter().any(|s| s == strip_generic(&f.type_name).as_str()) {
                let state_id = node_id("st", &strip_generic(&f.type_name));
                println!("  {state_id}[/\"{}\"/]:::state", strip_generic(&f.type_name));
                println!("  {field_id} -.-> {state_id}");
            }
        }
    }
    println!("  classDef ctx fill:#CECBF6,stroke:#3C3489;");
    println!("  classDef state fill:#9FE1CB,stroke:#085041;");
    println!("```");
}

fn render_dot(r: &Report) {
    println!("digraph hopper_accounts {{");
    println!("  rankdir=LR;");
    println!("  node [fontname=\"Inter\"];");
    for ctx in &r.contexts {
        let ctx_id = node_id("ctx", &ctx.name);
        println!(
            "  {ctx_id} [label=\"{}\" shape=box style=\"filled,rounded\" fillcolor=\"#CECBF6\"];",
            ctx.name
        );
        for f in &ctx.fields {
            let field_id = node_id(&format!("f{}", ctx.name), &f.name);
            let role = if f.is_signer {
                "signer"
            } else if f.is_mut {
                "mut"
            } else {
                "ro"
            };
            println!(
                "  {field_id} [label=\"{}\\n{}\\n{}\" shape=ellipse fillcolor=\"#FAEEDA\" style=filled];",
                f.name, f.type_name, role
            );
            let label = f.constraints.join(" ");
            println!("  {ctx_id} -> {field_id} [label=\"{label}\"];");
            if r.states.iter().any(|s| s == strip_generic(&f.type_name).as_str()) {
                let state_id = node_id("st", &strip_generic(&f.type_name));
                println!(
                    "  {state_id} [label=\"{}\" shape=folder fillcolor=\"#9FE1CB\" style=filled];",
                    strip_generic(&f.type_name)
                );
                println!("  {field_id} -> {state_id} [style=dashed];");
            }
        }
    }
    println!("}}");
}

fn render_json(r: &Report) {
    let mut contexts_json: Vec<serde_json::Value> = Vec::with_capacity(r.contexts.len());
    for ctx in &r.contexts {
        let mut fields_json: Vec<serde_json::Value> = Vec::with_capacity(ctx.fields.len());
        for f in &ctx.fields {
            fields_json.push(serde_json::json!({
                "name": f.name,
                "type": f.type_name,
                "is_mut": f.is_mut,
                "is_signer": f.is_signer,
                "is_init": f.is_init,
                "has_seeds": f.has_seeds,
                "has_bump": f.has_bump,
                "has_one": f.has_one_targets,
                "dup": f.dup_target,
                "has_extension_constraint": f.has_extension_constraint,
                "pins_token_2022": f.pins_token_2022,
                "constraints": f.constraints,
            }));
        }
        contexts_json.push(serde_json::json!({
            "name": ctx.name,
            "source_file": ctx.source_file,
            "has_validate_hook": ctx.has_validate_hook,
            "has_invariant_attr": ctx.has_invariant_attr,
            "fields": fields_json,
        }));
    }
    let diagnostics_json: Vec<serde_json::Value> = r
        .diagnostics
        .iter()
        .map(|d| {
            serde_json::json!({
                "level": match d.level {
                    Level::Error => "error",
                    Level::Warn => "warn",
                    Level::Info => "info",
                },
                "context": d.context,
                "field": d.field,
                "message": d.message,
                "source_file": d.source_file,
            })
        })
        .collect();
    let out = serde_json::json!({
        "contexts": contexts_json,
        "states": r.states,
        "diagnostics": diagnostics_json,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".into())
    );
}

fn node_id(prefix: &str, name: &str) -> String {
    let mut out = String::with_capacity(prefix.len() + 1 + name.len());
    out.push_str(prefix);
    out.push('_');
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

fn strip_generic(ty: &str) -> String {
    ty.split_whitespace()
        .collect::<String>()
        .split('<')
        .next()
        .unwrap_or(ty)
        .to_string()
}

// Prevent BTreeMap from being dropped as an unused import on stripped
// feature builds. The type is used transitively in render_json; keep
// it referenced explicitly for clarity.
#[allow(dead_code)]
fn _map_anchor(_m: BTreeMap<String, String>) {}
