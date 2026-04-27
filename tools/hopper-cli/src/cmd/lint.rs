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
//! - Metaplex `metadata::*` / `master_edition::*` account keywords that
//!   are only partially declared.
//! - Program-like crates that do not opt into Hopper's on-chain
//!   `no_allocator!()` / `nostd_panic_handler!()` markers.
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

use syn::{Attribute, File, Item, ItemStruct, Type};

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
    eprintln!("project, including Metaplex context keywords and on-chain");
    eprintln!("zero-allocation markers. Prints the graph and every diagnostic");
    eprintln!("found. Exits non-zero when an error-level diagnostic is surfaced.");
}

/// Result of an inline lint pass: the count of error- and warn-level
/// diagnostics, plus the formatted lines themselves so callers can
/// print them in their own output stream.
pub struct LintSummary {
    pub errors: usize,
    pub warnings: usize,
    pub lines: Vec<String>,
}

/// Programmatic entry point for `hopper compile --lint`. Runs the
/// account-relationship checker over `project_root` (no graph output,
/// just diagnostics) and returns a structured summary the caller can
/// print and act on. Mirrors the diagnostic surface of `cmd_lint`
/// without printing or process-exiting from inside this function.
pub fn run_lint_diagnostics(project_root: &Path) -> Result<LintSummary, String> {
    let report = lint_project(project_root)?;
    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut lines = Vec::with_capacity(report.diagnostics.len());
    for d in &report.diagnostics {
        match d.level {
            Level::Error => errors += 1,
            Level::Warn => warnings += 1,
            Level::Info => {}
        }
        lines.push(d.format());
    }
    Ok(LintSummary {
        errors,
        warnings,
        lines,
    })
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
    has_program_entrypoint: bool,
    has_no_allocator: bool,
    has_panic_handler: bool,
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
    metadata_keys: Vec<String>,
    master_edition_keys: Vec<String>,
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
    let src_dirs = [root.join("src"), root.join("programs")];
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
    let mut has_program_entrypoint = false;
    let mut has_no_allocator = false;
    let mut has_panic_handler = false;

    for file_path in &files {
        let text = fs::read_to_string(file_path)
            .map_err(|e| format!("read {}: {e}", file_path.display()))?;
        has_program_entrypoint |= looks_like_program_entrypoint(&text);
        has_no_allocator |= text.contains("no_allocator!()") || text.contains("no_allocator ! ()");
        has_panic_handler |=
            text.contains("nostd_panic_handler!()") || text.contains("nostd_panic_handler ! ()");
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

    if has_program_entrypoint && !has_no_allocator {
        diagnostics.push(Diagnostic {
            level: Level::Warn,
            context: "crate".into(),
            field: None,
            message: "program entrypoint detected but no `no_allocator!()` marker found; add it near the crate root for on-chain zero-heap enforcement".into(),
            source_file: root.to_path_buf(),
        });
    }
    if has_program_entrypoint && !has_panic_handler {
        diagnostics.push(Diagnostic {
            level: Level::Warn,
            context: "crate".into(),
            field: None,
            message: "program entrypoint detected but no `nostd_panic_handler!()` marker found; add it near the crate root for deterministic on-chain panic behavior".into(),
            source_file: root.to_path_buf(),
        });
    }

    Ok(Report {
        contexts,
        states,
        diagnostics,
        has_program_entrypoint,
        has_no_allocator,
        has_panic_handler,
    })
}

fn looks_like_program_entrypoint(text: &str) -> bool {
    text.contains("hopper_entrypoint!")
        || text.contains("hopper_fast_entrypoint!")
        || text.contains("hopper_lazy_entrypoint!")
        || text.contains("program_entrypoint!")
        || text.contains("fast_entrypoint!")
        || text.contains("lazy_entrypoint!")
        || text.contains("#[hopper::program]")
        || text.contains("#[program]")
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
        let Some(last) = path.segments.last() else {
            continue;
        };
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
                metadata_keys: Vec::new(),
                master_edition_keys: Vec::new(),
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
            if ns == "metadata" {
                info.metadata_keys.push(key.clone());
                info.is_mut = true;
            }
            if ns == "master_edition" {
                info.master_edition_keys.push(key.clone());
                info.is_mut = true;
            }
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
        if meta.path.segments.len() == 3 && meta.path.segments[0].ident == "extensions" {
            info.has_extension_constraint = true;
            let joined = format!(
                "extensions::{}::{}",
                meta.path.segments[1].ident, meta.path.segments[2].ident
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
        // Metaplex metadata keywords must be complete before the
        // generated context helper can build a CreateMetadataAccountV3
        // call. Keep this in the lint pass so users see a direct
        // project-level diagnostic instead of only a proc-macro error.
        let metadata_data_any = has_any(
            &f.metadata_keys,
            &[
                "name",
                "symbol",
                "uri",
                "seller_fee_basis_points",
                "is_mutable",
            ],
        );
        let metadata_data_complete = has_all(
            &f.metadata_keys,
            &["name", "symbol", "uri", "seller_fee_basis_points"],
        );
        let metadata_cpi_any = has_any(
            &f.metadata_keys,
            &[
                "mint",
                "mint_authority",
                "payer",
                "update_authority",
                "system_program",
                "rent",
            ],
        );
        if metadata_data_any && !metadata_data_complete {
            out.push(Diagnostic {
                level: Level::Error,
                context: ctx.name.clone(),
                field: Some(f.name.clone()),
                message: "metadata fields require `metadata::{name,symbol,uri,seller_fee_basis_points}` together".into(),
                source_file: ctx.source_file.clone(),
            });
        }
        if metadata_cpi_any
            && (!metadata_data_complete
                || !has_all(
                    &f.metadata_keys,
                    &[
                        "mint",
                        "mint_authority",
                        "payer",
                        "update_authority",
                        "system_program",
                    ],
                ))
        {
            out.push(Diagnostic {
                level: Level::Error,
                context: ctx.name.clone(),
                field: Some(f.name.clone()),
                message: "metadata CPI helper requires metadata::{mint,mint_authority,payer,update_authority,system_program,name,symbol,uri,seller_fee_basis_points}; `metadata::rent` is optional".into(),
                source_file: ctx.source_file.clone(),
            });
        }

        let master_edition_any = !f.master_edition_keys.is_empty();
        if (metadata_data_any || metadata_cpi_any) && master_edition_any {
            out.push(Diagnostic {
                level: Level::Error,
                context: ctx.name.clone(),
                field: Some(f.name.clone()),
                message: "declare `metadata::*` and `master_edition::*` on separate account fields"
                    .into(),
                source_file: ctx.source_file.clone(),
            });
        }
        if master_edition_any
            && !has_all(
                &f.master_edition_keys,
                &[
                    "max_supply",
                    "mint",
                    "metadata",
                    "update_authority",
                    "mint_authority",
                    "payer",
                    "token_program",
                    "system_program",
                ],
            )
        {
            out.push(Diagnostic {
                level: Level::Error,
                context: ctx.name.clone(),
                field: Some(f.name.clone()),
                message: "master_edition helper requires master_edition::{max_supply,mint,metadata,update_authority,mint_authority,payer,token_program,system_program}; `master_edition::rent` is optional".into(),
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

fn has_all(keys: &[String], required: &[&str]) -> bool {
    required
        .iter()
        .all(|needle| keys.iter().any(|key| key == needle))
}

fn has_any(keys: &[String], needles: &[&str]) -> bool {
    needles
        .iter()
        .any(|needle| keys.iter().any(|key| key == needle))
}

// ---- renderers -------------------------------------------------------------

fn render_ascii(r: &Report) {
    println!("Hopper account-relationship graph");
    println!("=================================");
    println!(
        "[crate] entrypoint={} no_allocator={} panic_handler={}",
        r.has_program_entrypoint, r.has_no_allocator, r.has_panic_handler
    );
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
            println!(
                "  {field_id}([\"{}<br/>{}<br/>{}\"])",
                f.name, f.type_name, role
            );
            let label = f.constraints.join(" ");
            println!("  {ctx_id} -->|\"{}\"| {field_id}", label);
            // Edge to the state type if we recognize it.
            if r.states
                .iter()
                .any(|s| s == strip_generic(&f.type_name).as_str())
            {
                let state_id = node_id("st", &strip_generic(&f.type_name));
                println!(
                    "  {state_id}[/\"{}\"/]:::state",
                    strip_generic(&f.type_name)
                );
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
            if r.states
                .iter()
                .any(|s| s == strip_generic(&f.type_name).as_str())
            {
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
                "metadata_keys": f.metadata_keys,
                "master_edition_keys": f.master_edition_keys,
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
        "crate": {
            "has_program_entrypoint": r.has_program_entrypoint,
            "has_no_allocator": r.has_no_allocator,
            "has_panic_handler": r.has_panic_handler,
        },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_field(name: &str) -> FieldInfo {
        FieldInfo {
            name: name.into(),
            type_name: "AccountView".into(),
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
            metadata_keys: Vec::new(),
            master_edition_keys: Vec::new(),
        }
    }

    fn context_with(field: FieldInfo) -> ContextInfo {
        ContextInfo {
            name: "MintNft".into(),
            fields: vec![field],
            source_file: PathBuf::from("src/lib.rs"),
            has_validate_hook: true,
            has_invariant_attr: false,
        }
    }

    #[test]
    fn detects_program_entrypoints_and_allocator_markers() {
        assert!(looks_like_program_entrypoint(
            "hopper_entrypoint!(process);"
        ));
        assert!(looks_like_program_entrypoint(
            "#[hopper::program]\nmod app {}"
        ));
        assert!(!looks_like_program_entrypoint("pub fn helper() {}"));
    }

    #[test]
    fn metadata_scrape_tracks_keys_and_marks_writable() {
        let item: ItemStruct = syn::parse_str(
            r#"
            pub struct MintNft {
                #[account(metadata::mint = mint, metadata::name = name)]
                pub metadata: AccountView,
            }
            "#,
        )
        .unwrap();
        let ctx = extract_context(&item, Path::new("src/lib.rs"));
        let field = &ctx.fields[0];
        assert!(field.is_mut);
        assert!(field.metadata_keys.iter().any(|k| k == "mint"));
        assert!(field.metadata_keys.iter().any(|k| k == "name"));
    }

    #[test]
    fn partial_metadata_constraints_are_errors() {
        let mut field = empty_field("metadata");
        field.metadata_keys = vec!["name".into(), "symbol".into()];
        let ctx = context_with(field);
        let mut diagnostics = Vec::new();
        run_diagnostics(&ctx, &mut diagnostics);
        assert!(diagnostics
            .iter()
            .any(|d| { d.level == Level::Error && d.message.contains("metadata fields require") }));
    }

    #[test]
    fn complete_master_edition_constraints_are_valid() {
        let mut field = empty_field("master_edition");
        field.master_edition_keys = [
            "max_supply",
            "mint",
            "metadata",
            "update_authority",
            "mint_authority",
            "payer",
            "token_program",
            "system_program",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let ctx = context_with(field);
        let mut diagnostics = Vec::new();
        run_diagnostics(&ctx, &mut diagnostics);
        assert!(!diagnostics.iter().any(|d| d.level == Level::Error));
    }
}

// Prevent BTreeMap from being dropped as an unused import on stripped
// feature builds. The type is used transitively in render_json; keep
// it referenced explicitly for clarity.
#[allow(dead_code)]
fn _map_anchor(_m: BTreeMap<String, String>) {}
