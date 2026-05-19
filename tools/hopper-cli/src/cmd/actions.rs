use std::fs;
use std::path::PathBuf;
use std::process;

use hopper_schema::ProgramManifest;

pub fn cmd_actions(args: &[String]) {
    if args.first().map(String::as_str) != Some("gen") {
        usage_and_exit();
    }
    let mut program_arg = None;
    let mut out_dir = None;
    let mut framework = "next".to_string();
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
                out_dir = Some(PathBuf::from(&args[i]));
            }
            "--framework" => {
                i += 1;
                if i >= args.len() {
                    usage_and_exit();
                }
                framework = args[i].clone();
            }
            "--help" | "-h" => usage_and_exit(),
            other => {
                eprintln!("Unknown actions argument: {other}");
                usage_and_exit();
            }
        }
        i += 1;
    }

    if framework != "next" {
        eprintln!("Unsupported actions framework `{framework}`. Supported: next");
        process::exit(1);
    }

    let program_arg = program_arg.unwrap_or_else(|| usage_and_exit());
    let out_dir = out_dir.unwrap_or_else(|| PathBuf::from("api/actions"));
    let manifest = crate::load_program_manifest(&program_arg);
    write_next_actions(&manifest, &out_dir);
}

fn usage_and_exit() -> ! {
    eprintln!("Usage: hopper actions gen --program <manifest> --out <dir> [--framework next]");
    process::exit(1);
}

fn write_next_actions(manifest: &ProgramManifest, out_dir: &PathBuf) {
    fs::create_dir_all(out_dir).unwrap_or_else(|err| {
        eprintln!("Failed to create {}: {err}", out_dir.display());
        process::exit(1);
    });

    let registry = actions_registry(manifest);
    fs::write(out_dir.join("actions.json"), registry).unwrap_or_else(|err| {
        eprintln!("Failed to write actions.json: {err}");
        process::exit(1);
    });

    let route = next_route(manifest);
    fs::write(out_dir.join("route.ts"), route).unwrap_or_else(|err| {
        eprintln!("Failed to write route.ts: {err}");
        process::exit(1);
    });

    println!(
        "Generated Solana Actions scaffold for {} at {}",
        manifest.name,
        out_dir.display()
    );
}

fn actions_registry(manifest: &ProgramManifest) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!(
        "  \"name\": \"{}\",\n",
        json_escape(manifest.name)
    ));
    out.push_str(&format!(
        "  \"version\": \"{}\",\n",
        json_escape(manifest.version)
    ));
    out.push_str("  \"actions\": [\n");
    for (i, ix) in manifest.instructions.iter().enumerate() {
        let comma = if i + 1 == manifest.instructions.len() {
            ""
        } else {
            ","
        };
        out.push_str(&format!(
            "    {{ \"name\": \"{}\", \"tag\": {}, \"path\": \"/api/actions/{}\" }}{}\n",
            json_escape(ix.name),
            ix.tag,
            json_escape(ix.name),
            comma
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

fn next_route(manifest: &ProgramManifest) -> String {
    let names = manifest
        .instructions
        .iter()
        .map(|ix| format!("  \"{}\": {},", ts_escape(ix.name), ix.tag))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "export const runtime = 'edge';\n\nconst INSTRUCTION_TAGS: Record<string, number> = {{\n{names}\n}};\n\nconst headers = {{\n  'Access-Control-Allow-Origin': '*',\n  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',\n  'Access-Control-Allow-Headers': 'Content-Type, Authorization',\n}};\n\nexport function OPTIONS() {{\n  return new Response(null, {{ headers }});\n}}\n\nexport function GET() {{\n  return Response.json({{\n    label: '{label}',\n    icon: 'https://solana.com/favicon.ico',\n    title: '{label}',\n    description: '{description}',\n    links: {{ actions: Object.keys(INSTRUCTION_TAGS).map((name) => ({{ label: name, href: `/api/actions?instruction=${{name}}` }})) }},\n  }}, {{ headers }});\n}}\n\nexport async function POST(request: Request) {{\n  const url = new URL(request.url);\n  const instruction = url.searchParams.get('instruction');\n  if (!instruction || !(instruction in INSTRUCTION_TAGS)) {{\n    return Response.json({{ error: 'unknown instruction' }}, {{ status: 400, headers }});\n  }}\n  const body = await request.json().catch(() => ({{}}));\n  return Response.json({{\n    type: 'transaction',\n    instruction,\n    tag: INSTRUCTION_TAGS[instruction],\n    body,\n  }}, {{ headers }});\n}}\n",
        label = ts_escape(manifest.name),
        description = ts_escape(manifest.description)
    )
}

fn json_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn ts_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}
