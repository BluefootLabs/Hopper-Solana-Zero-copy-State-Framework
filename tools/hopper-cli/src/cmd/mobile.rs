use std::fs;
use std::path::PathBuf;
use std::process;

use hopper_schema::ProgramManifest;

pub fn cmd_mobile(args: &[String]) {
    if args.first().map(String::as_str) != Some("gen") {
        usage_and_exit();
    }
    let mut program_arg = None;
    let mut out_dir = PathBuf::from("mobile/generated");
    let mut target = "kotlin".to_string();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--program" => {
                i += 1;
                if i >= args.len() {
                    usage_and_exit();
                }
                program_arg = Some(args[i].clone());
            }
            "--out" => {
                i += 1;
                if i >= args.len() {
                    usage_and_exit();
                }
                out_dir = PathBuf::from(&args[i]);
            }
            "--target" => {
                i += 1;
                if i >= args.len() {
                    usage_and_exit();
                }
                target = args[i].clone();
            }
            "--help" | "-h" => usage_and_exit(),
            other => {
                eprintln!("Unknown mobile argument: {other}");
                usage_and_exit();
            }
        }
        i += 1;
    }
    let program_arg = program_arg.unwrap_or_else(|| usage_and_exit());
    let manifest = crate::load_program_manifest(&program_arg);
    fs::create_dir_all(&out_dir).unwrap_or_else(|err| {
        eprintln!("Failed to create {}: {err}", out_dir.display());
        process::exit(1);
    });

    match target.as_str() {
        "kotlin" | "kt" => write_file(out_dir.join("HopperProgram.kt"), kotlin(&manifest)),
        "react-native" | "ts" => {
            write_file(out_dir.join("hopperProgram.ts"), react_native(&manifest))
        }
        other => {
            eprintln!("Unsupported mobile target `{other}`. Supported: kotlin, react-native");
            process::exit(1);
        }
    }
    println!(
        "Generated {} mobile bindings for {} at {}",
        target,
        manifest.name,
        out_dir.display()
    );
}

fn usage_and_exit() -> ! {
    eprintln!("Usage: hopper mobile gen --program <manifest> --target <kotlin|react-native> [--out <dir>]");
    process::exit(1);
}

fn write_file(path: PathBuf, content: String) {
    fs::write(&path, content).unwrap_or_else(|err| {
        eprintln!("Failed to write {}: {err}", path.display());
        process::exit(1);
    });
}

fn kotlin(manifest: &ProgramManifest) -> String {
    let object_name = to_pascal(manifest.name);
    let tags = manifest
        .instructions
        .iter()
        .map(|ix| format!("    const val {}: Int = {}", to_const(ix.name), ix.tag))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "package generated.hopper\n\nobject {object_name}Instructions {{\n{tags}\n\n    fun tagFor(name: String): Int? = when (name) {{\n{cases}\n        else -> null\n    }}\n}}\n",
        cases = manifest
            .instructions
            .iter()
            .map(|ix| format!("        \"{}\" -> {}", kt_escape(ix.name), ix.tag))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn react_native(manifest: &ProgramManifest) -> String {
    let tags = manifest
        .instructions
        .iter()
        .map(|ix| format!("  {}: {},", sanitize_ident(ix.name), ix.tag))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "export const programName = '{name}';\n\nexport const instructionTags = {{\n{tags}\n}} as const;\n\nexport type HopperInstructionName = keyof typeof instructionTags;\n\nexport function encodeInstructionTag(name: HopperInstructionName): Uint8Array {{\n  return new Uint8Array([instructionTags[name]]);\n}}\n",
        name = ts_escape(manifest.name)
    )
}

fn to_pascal(value: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            if upper {
                out.push(ch.to_ascii_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        } else {
            upper = true;
        }
    }
    if out.is_empty() {
        "Program".to_string()
    } else {
        out
    }
}

fn to_const(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_uppercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn sanitize_ident(value: &str) -> String {
    let mut out = String::new();
    for (i, ch) in value.chars().enumerate() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            if i == 0 && ch.is_ascii_digit() {
                out.push('_');
            }
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "instruction".to_string()
    } else {
        out
    }
}

fn kt_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn ts_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}
