//! `hopper profile` subcommand tree.
//!
//! Two shipping subcommands:
//!
//! - `profile bench`. Runs the primitive-benchmark lab against a live
//!   cluster and emits JSON/CSV regression artifacts. Existing code.
//! - `profile elf`. Parses a compiled SBF ELF, resolves DWARF function
//!   names, and emits flamegraph-compatible folded-stack output plus a
//!   human-readable "top N functions by static size" table. Matches
//!   Quasar's `quasar profile` command ergonomically.
//!
//! The flamegraph output is the standard folded-stack format the
//! Brendan Gregg `FlameGraph.pl` and `inferno-flamegraph` consume:
//! `<stack_frames>;<semicolon_separated> <value>`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process;

use crate::bench;

pub fn cmd_profile(args: &[String]) {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_profile_usage();
        return;
    }

    if args.is_empty() || args[0] == "bench" {
        let bench_args = if args.first().map(String::as_str) == Some("bench") {
            &args[1..]
        } else {
            args
        };

        if let Err(err) = bench::run_primitive_bench(bench_args) {
            eprintln!("hopper profile bench failed: {err}");
            process::exit(1);
        }
        return;
    }

    if args[0] == "elf" {
        if let Err(err) = cmd_profile_elf(&args[1..]) {
            eprintln!("hopper profile elf failed: {err}");
            process::exit(1);
        }
        return;
    }

    eprintln!("Unknown profile subcommand: {}", args[0]);
    print_profile_usage();
    process::exit(1);
}

fn print_profile_usage() {
    eprintln!("Usage: hopper profile <subcommand> [options]");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  bench                         Primitive benchmark lab with JSON/CSV artifacts");
    eprintln!("  elf <path/to/program.so>      Static SBF ELF analysis: symbol sizes, DWARF");
    eprintln!("                                names, flamegraph-compatible folded output");
    eprintln!();
    eprintln!("`profile bench` options:");
    eprintln!("  --rpc <url>                   RPC endpoint (default: SOLANA_RPC_URL or localhost)");
    eprintln!("  --keypair <path>             Fee payer keypair (default: ~/.config/solana/id.json)");
    eprintln!("  --out-dir <dir>              Output directory for JSON/CSV artifacts");
    eprintln!("  --program-id <pubkey>        Reuse an existing deployed hopper-bench program");
    eprintln!("  --no-build                   Reuse the current hopper-bench .so");
    eprintln!("  --no-deploy                  Skip deploy (requires --program-id)");
    eprintln!("  --fail-on-regression <pct>   Override tolerated regression percentage");
    eprintln!();
    eprintln!("`profile elf` options:");
    eprintln!("  --top <N>                    Print the top N symbols by size (default 20)");
    eprintln!("  --folded <out.txt>           Write Brendan-Gregg folded-stack output for flamegraph");
    eprintln!("  --html <out.html>            Write self-contained interactive HTML flamegraph");
    eprintln!("                               (hover for tooltips, click to highlight, search box)");
    eprintln!("  --baseline <folded.txt>      Compare symbol sizes against a saved baseline folded file");
    eprintln!("  --open                       Open the HTML flamegraph in the default browser");
    eprintln!("  --no-demangle                Leave mangled symbol names intact");
}

struct ElfArgs<'a> {
    path: &'a str,
    top: usize,
    folded_out: Option<&'a str>,
    html_out: Option<&'a str>,
    baseline: Option<&'a str>,
    open_html: bool,
    demangle: bool,
}

fn parse_elf_args<'a>(args: &'a [String]) -> Result<ElfArgs<'a>, String> {
    if args.is_empty() {
        return Err("missing path to ELF; usage: hopper profile elf <program.so>".into());
    }
    let mut out = ElfArgs {
        path: &args[0],
        top: 20,
        folded_out: None,
        html_out: None,
        baseline: None,
        open_html: false,
        demangle: true,
    };
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--top" => {
                i += 1;
                out.top = args.get(i)
                    .ok_or("`--top` requires a value")?
                    .parse()
                    .map_err(|e| format!("`--top` must be a usize: {e}"))?;
            }
            "--folded" => {
                i += 1;
                out.folded_out = Some(args.get(i).ok_or("`--folded` requires a path")?.as_str());
            }
            "--html" => {
                i += 1;
                out.html_out = Some(args.get(i).ok_or("`--html` requires a path")?.as_str());
            }
            "--baseline" => {
                i += 1;
                out.baseline = Some(args.get(i).ok_or("`--baseline` requires a path")?.as_str());
            }
            "--open" => out.open_html = true,
            "--no-demangle" => out.demangle = false,
            other => return Err(format!("unknown elf flag: {other}")),
        }
        i += 1;
    }
    if out.open_html && out.html_out.is_none() {
        return Err("`--open` requires `--html <path>`".into());
    }
    Ok(out)
}

fn cmd_profile_elf(args: &[String]) -> Result<(), String> {
    let opts = parse_elf_args(args)?;
    let bytes = fs::read(opts.path)
        .map_err(|e| format!("could not read `{}`: {e}", opts.path))?;

    let (symbols, byte_total) = parse_symbols(&bytes, opts.demangle)?;

    // Optional baseline. Loaded from a previously-saved folded-stack
    // file (the same format `--folded` writes). When present, every
    // symbol gets a `delta` column relative to the baseline. Missing
    // symbols on the baseline side count as `+current`; missing on
    // the current side count as `-baseline`.
    let baseline_map: Option<BTreeMap<String, u64>> = match opts.baseline {
        Some(path) => Some(load_baseline_folded(path)?),
        None => None,
    };

    // Rank and print top-N by size.
    let mut ranked: Vec<(&str, u64)> = symbols.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));

    println!("hopper profile elf  -  {}", opts.path);
    println!("total code in .text: {} bytes", byte_total);
    println!("distinct symbols:    {}", ranked.len());
    if let Some(ref base) = baseline_map {
        let base_total: u64 = base.values().sum();
        let delta = byte_total as i64 - base_total as i64;
        let sign = if delta >= 0 { "+" } else { "" };
        println!(
            "baseline {} bytes ({} symbols) — total delta {sign}{} bytes",
            base_total,
            base.len(),
            delta,
        );
    }
    println!();
    if baseline_map.is_some() {
        println!("top {} symbols by static size (Δ vs. baseline):", opts.top);
        println!("{:>10}  {:>6}  {:>10}  symbol", "bytes", "pct", "delta");
    } else {
        println!("top {} symbols by static size:", opts.top);
        println!("{:>10}  {:>6}  symbol", "bytes", "pct");
    }
    let total = byte_total.max(1);
    for (name, sz) in ranked.iter().take(opts.top) {
        let pct = (*sz as f64 / total as f64) * 100.0;
        match baseline_map.as_ref() {
            Some(base) => {
                let prev = base.get(*name).copied().unwrap_or(0);
                let delta = *sz as i64 - prev as i64;
                let sign = if delta > 0 {
                    "+"
                } else if delta < 0 {
                    ""
                } else {
                    " "
                };
                println!("{:>10}  {:>5.2}%  {:>9}{}  {}", sz, pct, sign, delta, name);
            }
            None => {
                println!("{:>10}  {:>5.2}%  {}", sz, pct, name);
            }
        }
    }

    if let Some(out_path) = opts.folded_out {
        let folded = render_folded(&ranked);
        fs::write(out_path, folded)
            .map_err(|e| format!("could not write `{}`: {e}", out_path))?;
        println!();
        println!("wrote folded-stack flamegraph input to {}", out_path);
        println!("pipe it to a flamegraph renderer:");
        println!("  cat {} | inferno-flamegraph > profile.svg", out_path);
    }

    if let Some(out_path) = opts.html_out {
        let html = render_html_flamegraph(opts.path, &ranked, byte_total, baseline_map.as_ref());
        fs::write(out_path, html)
            .map_err(|e| format!("could not write `{}`: {e}", out_path))?;
        println!();
        println!("wrote interactive HTML flamegraph to {}", out_path);
        if opts.open_html {
            if let Err(err) = open_browser(out_path) {
                eprintln!("warning: could not open browser automatically: {err}");
                eprintln!("open it by hand: {out_path}");
            }
        } else {
            println!("open it in your browser to explore (hover, click, search).");
        }
    }
    Ok(())
}

/// Parse .text-region function symbols out of the ELF and return a
/// `(symbol_name -> bytes)` map plus the total.
///
/// DWARF-based inline expansion is a future enhancement; the symbol
/// table alone gives a useful first-order map of code footprint,
/// which is the metric the `quasar profile` output leads with. Names
/// are demangled via `rustc-demangle` when the flag is set.
fn parse_symbols(
    bytes: &[u8],
    demangle: bool,
) -> Result<(BTreeMap<String, u64>, u64), String> {
    use object::{Object, ObjectSymbol};

    let file = object::File::parse(bytes).map_err(|e| format!("not a valid ELF: {e}"))?;

    let mut out: BTreeMap<String, u64> = BTreeMap::new();
    let mut total: u64 = 0;
    for sym in file.symbols() {
        let size = sym.size();
        if size == 0 {
            continue;
        }
        if !matches!(sym.kind(), object::SymbolKind::Text) {
            continue;
        }
        let raw_name = sym.name().unwrap_or("?");
        let name = if demangle {
            rustc_demangle::demangle(raw_name).to_string()
        } else {
            raw_name.to_string()
        };
        *out.entry(name).or_insert(0) += size;
        total += size;
    }

    if out.is_empty() {
        return Err(format!(
            "ELF at `{}` has no .text symbols. Was it stripped? Try building with `cargo build-sbf --debug`.",
            "input"
        ));
    }
    Ok((out, total))
}

/// Render a Brendan-Gregg folded-stack flamegraph input from a
/// symbol-to-size table. Each symbol is one stack frame with its
/// name as the only identifier; nested call frames live in the
/// DWARF-enabled follow-up. Even without nesting, the flamegraph
/// still shows symbol sizes as proportional bars, which is the most
/// useful bird's-eye view of a compiled SBF program.
fn render_folded(ranked: &[(&str, u64)]) -> String {
    let mut s = String::new();
    for (name, sz) in ranked {
        // Sanitize the `;` separator inside demangled names (Rust's
        // generics use `<` `>` but not `;`; this is defensive).
        let safe: String = name.replace(';', ":");
        s.push_str(&safe);
        s.push(' ');
        s.push_str(&sz.to_string());
        s.push('\n');
    }
    s
}

/// Load a previously-saved Brendan-Gregg folded-stack file into a
/// `symbol -> bytes` map. The format is one symbol per line, with
/// the symbol name and the count separated by the last space (so
/// names containing spaces — Rust's `<T as Trait>::method` — still
/// parse correctly). Lines starting with `#` are treated as
/// comments and skipped, and blank lines are ignored. Returns a
/// helpful error if the file is malformed at any line so the user
/// can fix the input rather than getting silent zeros for every
/// symbol.
fn load_baseline_folded(path: &str) -> Result<BTreeMap<String, u64>, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("could not read baseline `{path}`: {e}"))?;
    let mut map: BTreeMap<String, u64> = BTreeMap::new();
    for (lineno, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let split_at = match line.rfind(' ') {
            Some(p) => p,
            None => {
                return Err(format!(
                    "baseline `{path}` line {}: missing space-separator before count",
                    lineno + 1
                ))
            }
        };
        let name = line[..split_at].trim();
        let count_str = line[split_at + 1..].trim();
        let count: u64 = count_str.parse().map_err(|e| {
            format!(
                "baseline `{path}` line {}: count `{count_str}` is not a u64: {e}",
                lineno + 1
            )
        })?;
        map.insert(name.to_string(), count);
    }
    Ok(map)
}

/// Render a self-contained interactive HTML flamegraph. No external
/// resources, no CDN, no JS framework — one file the user can open
/// straight in a browser.
///
/// The chart degrades gracefully when DWARF call-tree data isn't
/// available: each symbol becomes a single horizontal bar sized
/// proportionally to its byte count. Hover shows a tooltip with the
/// full demangled name, byte size, and percentage; click highlights
/// the bar and pins the tooltip; the search box filters by
/// substring. When a baseline is supplied, bars are colour-cued
/// (lime if shrunk, orange if grown, neutral if unchanged) and the
/// tooltip carries the delta.
fn render_html_flamegraph(
    program_path: &str,
    ranked: &[(&str, u64)],
    byte_total: u64,
    baseline: Option<&BTreeMap<String, u64>>,
) -> String {
    // Build the per-bar JSON inline. Symbol names can contain
    // characters that need escaping for both HTML and JSON — we
    // escape both via a small helper rather than depending on a
    // crate just for this.
    let mut data_json = String::from("[");
    for (i, (name, sz)) in ranked.iter().enumerate() {
        if i > 0 {
            data_json.push(',');
        }
        let pct = (*sz as f64 / byte_total.max(1) as f64) * 100.0;
        let delta = baseline
            .and_then(|b| b.get(*name).copied())
            .map(|prev| *sz as i64 - prev as i64);
        data_json.push_str(&format!(
            "{{\"n\":\"{}\",\"b\":{},\"p\":{:.4},\"d\":{}}}",
            json_escape(name),
            sz,
            pct,
            match delta {
                Some(d) => d.to_string(),
                None => "null".to_string(),
            }
        ));
    }
    data_json.push(']');

    let title = html_escape(program_path);
    let total_str = format_human_bytes(byte_total);
    let baseline_note = if baseline.is_some() {
        " (Δ vs. baseline)"
    } else {
        ""
    };

    // Single template string: HTML, CSS, JS. Indentation kept inside
    // the heredoc so the rendered file is readable when a user opens
    // it in a text editor.
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>hopper profile elf — {title}</title>
<style>
  :root {{
    --bg: #0e1117;
    --fg: #d6deeb;
    --dim: #6b7388;
    --accent: #2dd4bf;
    --bar: #38bdf8;
    --bar-grown: #fb923c;
    --bar-shrunk: #84cc16;
    --hover: #a78bfa;
    --row-h: 22px;
  }}
  * {{ box-sizing: border-box; }}
  html, body {{ margin: 0; padding: 0; background: var(--bg); color: var(--fg); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 13px; }}
  header {{ padding: 16px 24px; border-bottom: 1px solid #1f2937; display: flex; align-items: baseline; gap: 24px; flex-wrap: wrap; }}
  header h1 {{ font-size: 16px; margin: 0; color: var(--accent); font-weight: 600; }}
  header .meta {{ color: var(--dim); }}
  header .meta strong {{ color: var(--fg); font-weight: 600; }}
  #search {{ background: #1f2937; color: var(--fg); border: 1px solid #374151; border-radius: 4px; padding: 6px 10px; font: inherit; min-width: 220px; }}
  #search:focus {{ outline: 1px solid var(--accent); border-color: var(--accent); }}
  main {{ padding: 16px 24px 64px 24px; }}
  .bars {{ position: relative; }}
  .bar {{ position: relative; height: var(--row-h); margin-bottom: 1px; cursor: pointer; transition: opacity 0.1s; }}
  .bar.dimmed {{ opacity: 0.18; }}
  .bar .fill {{ position: absolute; left: 0; top: 0; bottom: 0; background: var(--bar); border-radius: 2px; transition: background-color 0.1s; }}
  .bar.grown .fill {{ background: var(--bar-grown); }}
  .bar.shrunk .fill {{ background: var(--bar-shrunk); }}
  .bar:hover .fill, .bar.selected .fill {{ background: var(--hover); }}
  .bar .label {{ position: relative; padding: 0 8px; line-height: var(--row-h); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; mix-blend-mode: difference; color: white; }}
  #tooltip {{ position: fixed; background: #111827; border: 1px solid #374151; border-radius: 4px; padding: 8px 12px; pointer-events: none; font-size: 12px; max-width: 480px; white-space: pre-line; box-shadow: 0 4px 12px rgba(0,0,0,0.5); z-index: 100; display: none; }}
  #tooltip strong {{ color: var(--accent); }}
  footer {{ position: fixed; bottom: 0; left: 0; right: 0; padding: 8px 24px; background: #0a0e14; border-top: 1px solid #1f2937; color: var(--dim); font-size: 11px; }}
  footer kbd {{ font-family: inherit; background: #1f2937; padding: 1px 6px; border-radius: 2px; color: var(--fg); }}
</style>
</head>
<body>
<header>
  <h1>hopper · profile · elf</h1>
  <div class="meta"><strong>{title}</strong></div>
  <div class="meta">total <strong>{total_str}</strong> across <strong id="symcount">…</strong> symbols{baseline_note}</div>
  <input id="search" type="search" placeholder="filter by symbol name…" autocomplete="off">
</header>
<main>
  <div class="bars" id="bars"></div>
</main>
<div id="tooltip" role="tooltip"></div>
<footer>
  <kbd>hover</kbd> for tooltip · <kbd>click</kbd> to pin a bar · <kbd>type</kbd> to filter by substring · <kbd>esc</kbd> to clear
</footer>
<script>
const SYMBOLS = {data_json};
const TOTAL = {byte_total};
document.getElementById("symcount").textContent = SYMBOLS.length;

const bars = document.getElementById("bars");
const tooltip = document.getElementById("tooltip");
const search = document.getElementById("search");

const maxBytes = SYMBOLS.length ? SYMBOLS[0].b : 1;

function fmt(n) {{
  if (Math.abs(n) < 1024) return n + " B";
  if (Math.abs(n) < 1024*1024) return (n/1024).toFixed(2) + " KiB";
  return (n/1024/1024).toFixed(2) + " MiB";
}}

let pinned = null;

SYMBOLS.forEach((sym, i) => {{
  const w = (sym.b / maxBytes) * 100;
  const bar = document.createElement("div");
  bar.className = "bar";
  if (sym.d !== null) {{
    if (sym.d > 0) bar.classList.add("grown");
    else if (sym.d < 0) bar.classList.add("shrunk");
  }}
  bar.dataset.index = i;
  bar.innerHTML = `<div class="fill" style="width:${{w}}%"></div><div class="label">${{escapeHtml(sym.n)}}</div>`;
  bars.appendChild(bar);

  bar.addEventListener("mousemove", (e) => {{
    if (pinned !== null) return;
    showTooltip(sym, e.clientX, e.clientY);
  }});
  bar.addEventListener("mouseleave", () => {{ if (pinned === null) hideTooltip(); }});
  bar.addEventListener("click", (e) => {{
    if (pinned === i) {{
      pinned = null;
      bar.classList.remove("selected");
      hideTooltip();
    }} else {{
      if (pinned !== null) document.querySelectorAll(".bar.selected").forEach(b => b.classList.remove("selected"));
      pinned = i;
      bar.classList.add("selected");
      showTooltip(sym, e.clientX, e.clientY);
    }}
    e.stopPropagation();
  }});
}});

document.addEventListener("click", () => {{
  if (pinned !== null) {{
    document.querySelectorAll(".bar.selected").forEach(b => b.classList.remove("selected"));
    pinned = null;
    hideTooltip();
  }}
}});

document.addEventListener("keydown", (e) => {{
  if (e.key === "Escape") {{
    search.value = "";
    applyFilter("");
    if (pinned !== null) {{
      document.querySelectorAll(".bar.selected").forEach(b => b.classList.remove("selected"));
      pinned = null;
      hideTooltip();
    }}
  }}
}});

search.addEventListener("input", (e) => applyFilter(e.target.value.toLowerCase()));

function applyFilter(needle) {{
  const all = bars.children;
  for (let i = 0; i < all.length; i++) {{
    const sym = SYMBOLS[i];
    const match = !needle || sym.n.toLowerCase().includes(needle);
    all[i].classList.toggle("dimmed", !match);
  }}
}}

function showTooltip(sym, x, y) {{
  let body = `<strong>${{escapeHtml(sym.n)}}</strong>\n${{fmt(sym.b)}} (${{sym.p.toFixed(2)}}%)`;
  if (sym.d !== null) {{
    const sign = sym.d > 0 ? "+" : "";
    body += `\nΔ vs. baseline: ${{sign}}${{fmt(sym.d)}}`;
  }}
  tooltip.innerHTML = body;
  tooltip.style.display = "block";
  // Clamp the tooltip to the viewport so it never escapes off-screen
  // when a user hovers near an edge.
  const rect = tooltip.getBoundingClientRect();
  const px = Math.min(x + 12, window.innerWidth - rect.width - 12);
  const py = Math.min(y + 12, window.innerHeight - rect.height - 12);
  tooltip.style.left = px + "px";
  tooltip.style.top = py + "px";
}}
function hideTooltip() {{ tooltip.style.display = "none"; }}
function escapeHtml(s) {{ return s.replace(/[&<>"']/g, c => ({{"&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;","'":"&#39;"}})[c]); }}
</script>
</body>
</html>
"##
    )
}

/// Format a byte count for human display. Used in the HTML header
/// so authors don't have to mentally divide by 1024.
fn format_human_bytes(b: u64) -> String {
    if b < 1024 {
        format!("{b} B")
    } else if b < 1024 * 1024 {
        format!("{:.2} KiB", b as f64 / 1024.0)
    } else {
        format!("{:.2} MiB", b as f64 / (1024.0 * 1024.0))
    }
}

/// Minimal HTML escaper — sufficient for the metadata strings we
/// embed (program path, symbol names). Avoids pulling in a templating
/// crate.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Minimal JSON-string escaper for the inlined data array. Handles
/// only the subset of cases we can encounter from `rustc-demangle`'s
/// output: ASCII printable, plus quotes, backslashes, and control
/// chars (rare but possible). Callers don't pass arbitrary user
/// input.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            other => out.push(other),
        }
    }
    out
}

/// Open `path` in the user's default browser. Cross-platform
/// best-effort: macOS uses `open`, Linux uses `xdg-open`, Windows
/// uses `cmd /c start`. Returns the underlying error if the launch
/// fails so the caller can fall through to "open it by hand"
/// guidance.
fn open_browser(path: &str) -> Result<(), String> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", path]);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let mut cmd = {
        // Fall back to xdg-open on unrecognised platforms; if it
        // doesn't exist the spawn itself will error.
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.spawn()
        .map_err(|e| format!("failed to launch browser: {e}"))?;
    Ok(())
}

// Compile-time assertion that the module sees its imports. Rustc emits
// an unused-import warning if `Path` is never referenced; pin it here.
#[allow(dead_code)]
fn _ignore(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escape_preserves_normal_names() {
        assert_eq!(json_escape("plain"), "plain");
    }

    #[test]
    fn json_escape_handles_quotes_and_backslashes() {
        assert_eq!(json_escape(r#"a"b\c"#), r#"a\"b\\c"#);
    }

    #[test]
    fn html_escape_handles_lt_gt_amp() {
        assert_eq!(
            html_escape(r#"<T as Trait>::"x" & y"#),
            "&lt;T as Trait&gt;::&quot;x&quot; &amp; y"
        );
    }

    #[test]
    fn baseline_folded_parses_names_with_spaces() {
        // Sandbox-friendly: write to a temp path and parse back.
        let tmp = std::env::temp_dir().join(format!(
            "hopper-baseline-{}.txt",
            std::process::id(),
        ));
        std::fs::write(&tmp, "<T as Trait>::method 1024\nplain 256\n").unwrap();
        let map = load_baseline_folded(tmp.to_str().unwrap()).expect("parse");
        assert_eq!(map.get("<T as Trait>::method"), Some(&1024));
        assert_eq!(map.get("plain"), Some(&256));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn baseline_folded_skips_comments_and_blanks() {
        let tmp = std::env::temp_dir().join(format!(
            "hopper-baseline-comments-{}.txt",
            std::process::id(),
        ));
        std::fs::write(
            &tmp,
            "# generated 2026-04-25\n\nfoo 100\n# comment\nbar 200\n",
        )
        .unwrap();
        let map = load_baseline_folded(tmp.to_str().unwrap()).expect("parse");
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("foo"), Some(&100));
        assert_eq!(map.get("bar"), Some(&200));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn html_flamegraph_embeds_data_inline() {
        let ranked = vec![("foo", 100u64), ("bar", 50u64)];
        let html = render_html_flamegraph("test.so", &ranked, 150, None);
        // Self-contained: no `http://` or `https://` references.
        assert!(!html.contains("https://"), "html must be self-contained");
        // Names appear in the inlined JSON.
        assert!(html.contains("\"n\":\"foo\""));
        assert!(html.contains("\"n\":\"bar\""));
    }
}
