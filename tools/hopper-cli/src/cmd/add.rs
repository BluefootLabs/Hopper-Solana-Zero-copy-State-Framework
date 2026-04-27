//! `hopper add` — incremental scaffolding for an existing project.
//!
//! Three sub-flags, any combination:
//!
//! - `-i / --instruction <name>`: create `src/instructions/<name>.rs`,
//!   ensure `src/instructions/mod.rs` re-exports it, ensure
//!   `mod instructions;` is wired into `src/lib.rs`. If the project
//!   uses the `#[hopper::program]` style dispatch (the minimal
//!   template), inject a stub `#[instruction(N)] pub fn <name>(...)`
//!   into the program block at the next-available discriminator. If
//!   the project uses the manual `match *disc` dispatch (the
//!   `nft-mint`, `token-2022-vault`, and `defi-vault` templates),
//!   skip the auto-injection and print a "wire it in by hand" hint
//!   pointing at the line number of the match block.
//! - `-s / --state <name>`: create or extend `src/state.rs` with a
//!   `#[hopper::state(disc = N, version = 1)] pub struct <Name>`,
//!   discriminator = max-existing + 1.
//! - `-e / --error <name>`: create or extend `src/errors.rs` with a
//!   `pub enum <Name>` of program errors.
//!
//! All edits are idempotent: running `hopper add -i transfer` twice
//! errors on the second run rather than overwriting silently. Quasar
//! follows the same rule.
//!
//! Why split this out from `hopper init`? The wizard takes a project
//! from zero to one — `add` takes it from one to many. A user
//! shouldn't have to leave the CLI to scaffold a second instruction.

use std::fs;
use std::path::Path;
use std::process;

use crate::style;
use crate::workspace;

pub fn cmd_add(args: &[String]) {
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print_add_usage();
        if args.is_empty() {
            process::exit(1);
        }
        return;
    }

    let mut instruction: Option<String> = None;
    let mut state: Option<String> = None;
    let mut error: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let take_value = |i: &mut usize, label: &str| -> String {
            if *i + 1 >= args.len() {
                eprintln!("{label} requires a name");
                process::exit(1);
            }
            *i += 2;
            args[*i - 1].clone()
        };
        match args[i].as_str() {
            "-i" | "--instruction" => instruction = Some(take_value(&mut i, "-i/--instruction")),
            "-s" | "--state" => state = Some(take_value(&mut i, "-s/--state")),
            "-e" | "--error" => error = Some(take_value(&mut i, "-e/--error")),
            other => {
                eprintln!("Unknown add flag: {other}");
                print_add_usage();
                process::exit(1);
            }
        }
    }

    if instruction.is_none() && state.is_none() && error.is_none() {
        eprintln!(
            "Specify at least one of -i/--instruction, -s/--state, or -e/--error"
        );
        print_add_usage();
        process::exit(1);
    }

    let cwd = workspace::current_dir().unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });
    let project_root = workspace::find_project_root(&cwd).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });

    let lib_rs = project_root.join("src").join("lib.rs");
    if !lib_rs.exists() {
        eprintln!(
            "{}: src/lib.rs not found at {} — are you inside a Hopper project?",
            style::fail("error"),
            project_root.display()
        );
        process::exit(1);
    }

    if let Some(name) = instruction {
        if let Err(err) = run_instruction(&project_root, &name) {
            eprintln!("{} {err}", style::fail("add instruction:"));
            process::exit(1);
        }
    }
    if let Some(name) = state {
        if let Err(err) = run_state(&project_root, &name) {
            eprintln!("{} {err}", style::fail("add state:"));
            process::exit(1);
        }
    }
    if let Some(name) = error {
        if let Err(err) = run_error(&project_root, &name) {
            eprintln!("{} {err}", style::fail("add error:"));
            process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Instruction
// ---------------------------------------------------------------------------

fn run_instruction(project_root: &Path, name: &str) -> Result<(), String> {
    let snake = validate_ident(name, "instruction")?;
    let pascal = snake_to_pascal(&snake);

    let instructions_dir = project_root.join("src").join("instructions");
    if !instructions_dir.exists() {
        fs::create_dir_all(&instructions_dir)
            .map_err(|err| format!("create {}: {err}", instructions_dir.display()))?;
        println!(
            "  {} {}",
            style::success("created"),
            style::dim(&display_rel(project_root, &instructions_dir))
        );
    }

    let file_path = instructions_dir.join(format!("{snake}.rs"));
    if file_path.exists() {
        return Err(format!(
            "src/instructions/{snake}.rs already exists — pick a different name or delete it first"
        ));
    }

    let body = render_instruction_template(&snake, &pascal);
    fs::write(&file_path, body)
        .map_err(|err| format!("write {}: {err}", file_path.display()))?;
    println!(
        "  {} {}",
        style::success("created"),
        style::dim(&display_rel(project_root, &file_path))
    );

    // Re-export from instructions/mod.rs.
    let mod_path = instructions_dir.join("mod.rs");
    let existing_mod = fs::read_to_string(&mod_path).unwrap_or_default();
    let needle = format!("mod {snake};");
    if !existing_mod.contains(&needle) {
        let separator = if existing_mod.is_empty() || existing_mod.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        let updated = format!(
            "{existing_mod}{separator}mod {snake};\npub use {snake}::*;\n"
        );
        fs::write(&mod_path, updated)
            .map_err(|err| format!("write {}: {err}", mod_path.display()))?;
        println!(
            "  {} {}",
            style::success("updated"),
            style::dim(&display_rel(project_root, &mod_path))
        );
    }

    // Ensure src/lib.rs has `mod instructions;`. Insert before the
    // first `#[hopper::program]` line, or at the top of the file if
    // none — that's the safest spot since module-level macros need
    // their imports already in scope.
    let lib_rs = project_root.join("src").join("lib.rs");
    let lib_content = fs::read_to_string(&lib_rs)
        .map_err(|err| format!("read {}: {err}", lib_rs.display()))?;
    if !lib_content.contains("mod instructions") {
        let insert = "mod instructions;\nuse instructions::*;\n\n";
        let updated = if let Some(pos) = lib_content.find("#[hopper::program]") {
            format!("{}{insert}{}", &lib_content[..pos], &lib_content[pos..])
        } else if let Some(pos) = lib_content.find("#[cfg(target_os = \"solana\")]") {
            format!("{}{insert}{}", &lib_content[..pos], &lib_content[pos..])
        } else {
            format!("{insert}{lib_content}")
        };
        fs::write(&lib_rs, updated)
            .map_err(|err| format!("write {}: {err}", lib_rs.display()))?;
        println!(
            "  {} {} (added `mod instructions;`)",
            style::success("updated"),
            style::dim(&display_rel(project_root, &lib_rs))
        );
    }

    // Try to wire into the dispatch.
    match try_wire_dispatch(&lib_rs, &snake, &pascal)? {
        DispatchWiring::HopperProgram { discriminator } => {
            println!(
                "  {} dispatch: `#[instruction({discriminator})] pub fn {snake}` injected into `#[hopper::program]`",
                style::success("wired")
            );
        }
        DispatchWiring::Manual => {
            println!(
                "  {} dispatch: project uses a manual `match *disc` block — wire `{snake}` into it by hand",
                style::warn("hint")
            );
        }
        DispatchWiring::None => {
            println!(
                "  {} no dispatch site detected; the file is at {}",
                style::warn("hint"),
                display_rel(project_root, &file_path)
            );
        }
    }

    Ok(())
}

enum DispatchWiring {
    HopperProgram { discriminator: u32 },
    Manual,
    None,
}

/// Inject a stub instruction into the `#[hopper::program] mod app {
/// ... }` block, picking the next-available `#[instruction(N)]`
/// discriminator. Returns the chosen discriminator or a hint that the
/// project uses a manual dispatch we shouldn't touch.
fn try_wire_dispatch(
    lib_rs: &Path,
    snake: &str,
    pascal: &str,
) -> Result<DispatchWiring, String> {
    let content = fs::read_to_string(lib_rs)
        .map_err(|err| format!("read {}: {err}", lib_rs.display()))?;

    if let Some(program_start) = content.find("#[hopper::program]") {
        // Find the opening `{` of the program mod and its matching
        // closing `}` by tracking brace depth from there. Skip
        // strings and line/block comments to avoid being fooled by
        // `'{'` literals inside doc comments. Hopper's templates
        // don't currently put strings here, but it's two extra lines
        // and removes a class of "weird bugs nobody can debug."
        let after_attr = &content[program_start..];
        let open = after_attr.find('{').ok_or_else(|| {
            "found `#[hopper::program]` but no opening brace afterwards".to_string()
        })?;
        let body_start = program_start + open + 1; // first byte inside the block

        let close_offset = match find_matching_close_brace(&content[body_start..]) {
            Some(n) => n,
            None => {
                return Err(
                    "found `#[hopper::program]` but the block is not balanced".to_string(),
                )
            }
        };
        let body_end = body_start + close_offset; // position of the `}` itself

        // Discover existing `#[instruction(N)]` discriminators inside
        // the program body.
        let body_text = &content[body_start..body_end];
        let mut max_disc: i64 = -1;
        for line in body_text.lines() {
            let trimmed = line.trim();
            // Match either `#[instruction(N)]` or
            // `#[instruction(disc = N)]` — both shapes show up in
            // Hopper's templates.
            if let Some(rest) = trimmed.strip_prefix("#[instruction(") {
                if let Some(close) = rest.find(')') {
                    let arg = rest[..close].trim();
                    let num = if let Some(eq) = arg.find('=') {
                        arg[eq + 1..].trim()
                    } else {
                        arg
                    };
                    if let Ok(n) = num.parse::<i64>() {
                        max_disc = max_disc.max(n);
                    }
                }
            }
        }
        let next_disc: u32 = (max_disc + 1) as u32;

        let stub = format!(
            "\n    #[hopper::pipeline]\n    #[instruction({next_disc})]\n    pub fn {snake}(ctx: Context<{pascal}>) -> ProgramResult {{\n        // TODO: implement `{snake}`\n        let _ = ctx;\n        Ok(())\n    }}\n",
        );

        // Insert just before the closing `}` of the program block.
        let updated = format!("{}{stub}{}", &content[..body_end], &content[body_end..]);
        fs::write(lib_rs, updated)
            .map_err(|err| format!("write {}: {err}", lib_rs.display()))?;
        return Ok(DispatchWiring::HopperProgram {
            discriminator: next_disc,
        });
    }

    if content.contains("match *disc") {
        return Ok(DispatchWiring::Manual);
    }

    Ok(DispatchWiring::None)
}

/// Given a string starting after a `{`, return the byte offset of the
/// matching closing `}`. Tracks string literals and line/block
/// comments so we don't double-count.
fn find_matching_close_brace(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth: i64 = 1;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        // Line comment: skip until newline.
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment: skip until `*/`.
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        // Skip string literal contents.
        if c == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn render_instruction_template(snake: &str, pascal: &str) -> String {
    format!(
        "//! `{snake}` instruction handler.\n//!\n//! Generated by `hopper add -i {snake}`. Wire any new accounts into\n//! the `Context` struct, then implement the body.\n\nuse hopper::prelude::*;\n\n#[hopper::context]\npub struct {pascal} {{\n    #[signer]\n    pub authority: AccountView,\n    // Add accounts here. The `Context` derives bounds, segment maps,\n    // and the borrow registry from this struct's fields.\n}}\n\nimpl<'info> {pascal} {{\n    /// Run the `{snake}` instruction.\n    #[inline(always)]\n    pub fn {snake}(&self) -> ProgramResult {{\n        // TODO: implement `{snake}` business logic.\n        Ok(())\n    }}\n}}\n"
    )
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

fn run_state(project_root: &Path, name: &str) -> Result<(), String> {
    let snake = validate_ident(name, "state")?;
    let pascal = snake_to_pascal(&snake);
    let path = project_root.join("src").join("state.rs");

    let (existing, action) = match fs::read_to_string(&path) {
        Ok(s) => (Some(s), "updated"),
        Err(_) => (None, "created"),
    };

    let next_disc = match &existing {
        Some(text) => find_max_state_disc(text).map(|d| d + 1).unwrap_or(1),
        None => 1,
    };

    let new_struct = format!(
        "\n#[derive(Clone, Copy)]\n#[repr(C)]\n#[hopper::state(disc = {next_disc}, version = 1)]\npub struct {pascal} {{\n    pub authority: TypedAddress<Authority>,\n    pub bump: u8,\n}}\n"
    );

    let body = match existing {
        Some(text) => {
            // Same struct already in there? Bail rather than appending a
            // duplicate that would fail to compile.
            let needle = format!("pub struct {pascal} ");
            if text.contains(&needle) {
                return Err(format!("`{pascal}` already declared in src/state.rs"));
            }
            format!("{}{}", text.trim_end_matches('\n'), new_struct)
        }
        None => format!(
            "//! Program state. Each struct declares a discriminator + layout\n//! version that the Hopper runtime uses to identify and version\n//! account data on the wire.\n\nuse hopper::prelude::*;\n{new_struct}"
        ),
    };

    fs::write(&path, body).map_err(|err| format!("write {}: {err}", path.display()))?;
    println!(
        "  {} {} ({} disc={next_disc})",
        style::success(action),
        style::dim(&display_rel(project_root, &path)),
        pascal,
    );
    Ok(())
}

fn find_max_state_disc(s: &str) -> Option<i64> {
    let mut max: Option<i64> = None;
    for line in s.lines() {
        let trimmed = line.trim();
        // Match `#[hopper::state(disc = N` and `#[state(disc = N`.
        let prefixes = ["#[hopper::state(", "#[state("];
        let after = prefixes
            .iter()
            .find_map(|p| trimmed.strip_prefix(p));
        let Some(after) = after else { continue };
        let Some(disc_pos) = after.find("disc") else { continue };
        let after_disc = &after[disc_pos..];
        let Some(eq) = after_disc.find('=') else { continue };
        let rest = &after_disc[eq + 1..];
        let mut num = String::new();
        for ch in rest.chars() {
            if ch.is_ascii_digit() {
                num.push(ch);
            } else if !num.is_empty() {
                break;
            } else if !ch.is_whitespace() {
                break;
            }
        }
        if let Ok(n) = num.parse::<i64>() {
            max = Some(max.map_or(n, |m| m.max(n)));
        }
    }
    max
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

fn run_error(project_root: &Path, name: &str) -> Result<(), String> {
    let snake = validate_ident(name, "error")?;
    let pascal = snake_to_pascal(&snake);
    let path = project_root.join("src").join("errors.rs");

    let (existing, action) = match fs::read_to_string(&path) {
        Ok(s) => (Some(s), "updated"),
        Err(_) => (None, "created"),
    };

    let new_enum = format!(
        "\n/// Errors raised by `{snake}`-related instructions.\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n#[repr(u32)]\npub enum {pascal} {{\n    Unknown = 0,\n}}\n\nimpl From<{pascal}> for ProgramError {{\n    fn from(err: {pascal}) -> Self {{\n        ProgramError::Custom(err as u32)\n    }}\n}}\n"
    );

    let body = match existing {
        Some(text) => {
            if text.contains(&format!("pub enum {pascal} ")) {
                return Err(format!("`{pascal}` already declared in src/errors.rs"));
            }
            format!("{}{}", text.trim_end_matches('\n'), new_enum)
        }
        None => format!(
            "//! Program error definitions.\n\nuse hopper::prelude::*;\n{new_enum}"
        ),
    };

    fs::write(&path, body).map_err(|err| format!("write {}: {err}", path.display()))?;
    println!(
        "  {} {} ({})",
        style::success(action),
        style::dim(&display_rel(project_root, &path)),
        pascal,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert `name` to snake_case after rejecting anything that wouldn't
/// be a valid Rust identifier.
fn validate_ident(input: &str, kind: &str) -> Result<String, String> {
    let snake = input.replace('-', "_");
    if snake.is_empty()
        || snake.starts_with(|c: char| c.is_ascii_digit())
        || !snake
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(format!(
            "invalid {kind} name `{input}` — must be a valid Rust identifier (e.g. `transfer`, `create_pool`)"
        ));
    }
    Ok(snake)
}

fn snake_to_pascal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut next_upper = true;
    for ch in s.chars() {
        if ch == '_' {
            next_upper = true;
            continue;
        }
        if next_upper {
            out.extend(ch.to_uppercase());
            next_upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn display_rel(root: &Path, full: &Path) -> String {
    full.strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| full.display().to_string())
}

fn print_add_usage() {
    eprintln!("Usage: hopper add [-i <name>] [-s <name>] [-e <name>]");
    eprintln!();
    eprintln!("Scaffold an instruction, state struct, or error enum into the");
    eprintln!("current Hopper project. Any combination of flags can be used.");
    eprintln!();
    eprintln!("  -i, --instruction <name>   New `src/instructions/<name>.rs` handler");
    eprintln!("  -s, --state <name>         New `pub struct <Name>` in src/state.rs");
    eprintln!("  -e, --error <name>         New `pub enum <Name>` in src/errors.rs");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;

    fn unique_tempdir(label: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("hopper-add-{label}-{pid}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        File::create(dir.join("Cargo.toml"))
            .unwrap()
            .write_all(b"[package]\nname=\"sample\"\nversion=\"0.1.0\"\nedition=\"2021\"\n")
            .unwrap();
        dir
    }

    #[test]
    fn snake_to_pascal_roundtrip() {
        assert_eq!(snake_to_pascal("create_vault"), "CreateVault");
        assert_eq!(snake_to_pascal("transfer"), "Transfer");
        assert_eq!(snake_to_pascal("a_b_c"), "ABC");
    }

    #[test]
    fn validate_ident_rejects_garbage() {
        assert!(validate_ident("123foo", "instruction").is_err());
        assert!(validate_ident("with space", "instruction").is_err());
        assert!(validate_ident("good_one", "instruction").is_ok());
        assert_eq!(
            validate_ident("kebab-name", "instruction").unwrap(),
            "kebab_name"
        );
    }

    #[test]
    fn brace_matcher_handles_nested_blocks() {
        // The body starts AFTER the opening brace of the outer block.
        let body = " inner { ... } trailing }";
        let close = find_matching_close_brace(body).expect("balanced");
        // Position should be at the final closing `}`, which is the
        // last byte of the body (length minus one).
        assert_eq!(close, body.len() - 1);
    }

    #[test]
    fn brace_matcher_skips_braces_in_strings() {
        let body = " let x = \"}\"; }";
        let close = find_matching_close_brace(body).expect("balanced");
        assert_eq!(close, body.len() - 1);
    }

    #[test]
    fn state_disc_increment_picks_next_unused() {
        let s = r#"
#[hopper::state(disc = 1, version = 1)]
pub struct A { pub x: u8 }

#[hopper::state(disc = 7, version = 1)]
pub struct B { pub y: u8 }
"#;
        assert_eq!(find_max_state_disc(s), Some(7));
    }

    #[test]
    fn run_state_appends_with_next_disc() {
        let dir = unique_tempdir("state");
        // First state — creates the file.
        run_state(&dir, "first").unwrap();
        let body = fs::read_to_string(dir.join("src/state.rs")).unwrap();
        assert!(body.contains("pub struct First"));
        assert!(body.contains("disc = 1"));

        // Second state — appends with disc=2.
        run_state(&dir, "second").unwrap();
        let body = fs::read_to_string(dir.join("src/state.rs")).unwrap();
        assert!(body.contains("pub struct Second"));
        assert!(body.contains("disc = 2"));

        // Third call with the same name — must error, not corrupt the file.
        let err = run_state(&dir, "second").unwrap_err();
        assert!(err.contains("already declared"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_error_creates_then_appends() {
        let dir = unique_tempdir("err");
        run_error(&dir, "vault").unwrap();
        let body = fs::read_to_string(dir.join("src/errors.rs")).unwrap();
        assert!(body.contains("pub enum Vault"));
        assert!(body.contains("ProgramError::Custom"));

        run_error(&dir, "access").unwrap();
        let body = fs::read_to_string(dir.join("src/errors.rs")).unwrap();
        assert!(body.contains("pub enum Vault"));
        assert!(body.contains("pub enum Access"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn instruction_injects_into_hopper_program_block() {
        let dir = unique_tempdir("ix");
        let lib_rs = dir.join("src/lib.rs");
        fs::write(
            &lib_rs,
            r#"use hopper::prelude::*;

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(_p: &Address, _a: &[AccountView], _d: &[u8]) -> ProgramResult { Ok(()) }

#[hopper::program]
mod app {
    use super::*;

    #[instruction(0)]
    pub fn initialize(ctx: Context<Initialize>) -> ProgramResult { Ok(()) }
}
"#,
        )
        .unwrap();

        run_instruction(&dir, "transfer").unwrap();

        let lib = fs::read_to_string(&lib_rs).unwrap();
        assert!(lib.contains("mod instructions;"));
        assert!(lib.contains("#[instruction(1)]"));
        assert!(lib.contains("pub fn transfer("));

        let ix = fs::read_to_string(dir.join("src/instructions/transfer.rs")).unwrap();
        assert!(ix.contains("pub struct Transfer"));

        let m = fs::read_to_string(dir.join("src/instructions/mod.rs")).unwrap();
        assert!(m.contains("mod transfer;"));
        assert!(m.contains("pub use transfer::*;"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn instruction_detects_manual_dispatch() {
        let dir = unique_tempdir("manual");
        let lib_rs = dir.join("src/lib.rs");
        fs::write(
            &lib_rs,
            r#"use hopper::prelude::*;

fn process_instruction(_p: &Address, _a: &[AccountView], data: &[u8]) -> ProgramResult {
    let (disc, _) = data.split_first().ok_or(ProgramError::InvalidInstructionData)?;
    match *disc {
        0 => Ok(()),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
"#,
        )
        .unwrap();

        // No panic, no injection — just creates the instruction file
        // and prints a hint. We can verify by reading back the lib.rs:
        // it should still be the manual dispatch.
        run_instruction(&dir, "withdraw").unwrap();
        let lib = fs::read_to_string(&lib_rs).unwrap();
        assert!(lib.contains("match *disc"));
        assert!(!lib.contains("#[instruction(1)]"));

        let _ = fs::remove_dir_all(&dir);
    }
}
