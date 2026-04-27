//! `hopper keys` subcommand tree.
//!
//! Ed25519 keypair management for program deploys and PDA derivation.
//! Matches the ergonomics of `solana-keygen` and `quasar keys` without
//! forcing a second toolchain. Every Hopper workflow that used to
//! require dropping to `solana-keygen new -o target/deploy/foo-keypair.json`
//! now has a framework-native path.
//!
//! Subcommands:
//!
//! - `hopper keys new <path>` - generate a keypair, write to path
//! - `hopper keys list [<path>...]` - pretty-print pubkey + path for
//!   each keypair file. No args walks the workspace's
//!   `target/deploy/*.json`.
//! - `hopper keys print <path>` - print the base58 pubkey only
//! - `hopper keys pda <seed>... [--program <id>]` - derive a PDA
//!   from the given seeds under the specified program id. Seeds
//!   support `b"text"`, `hex:0a1b2c`, `base58:Abc...`, or raw ASCII.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use bs58;
use curve25519_dalek::{constants::ED25519_BASEPOINT_TABLE, scalar::Scalar};
use sha2::{Digest, Sha512};

pub fn cmd_keys(args: &[String]) {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h" | "help") {
        print_usage();
        return;
    }
    match args[0].as_str() {
        "new" => cmd_new(&args[1..]),
        "list" | "ls" => cmd_list(&args[1..]),
        "print" | "show" => cmd_print(&args[1..]),
        "pda" => cmd_pda(&args[1..]),
        "sync" => cmd_sync(&args[1..]),
        other => {
            eprintln!("Unknown keys subcommand: {other}");
            print_usage();
            process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("Usage: hopper keys <subcommand>");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  new <path>                        Generate an ed25519 keypair at <path>");
    eprintln!("  list [<path>...]                  List pubkey + path for each keypair");
    eprintln!("  print <path>                      Print just the base58 pubkey");
    eprintln!("  pda <seed>... [--program <id>]    Derive a PDA from seeds");
    eprintln!("  sync <path> [--src <file.rs>]     Rewrite hopper::declare_id! / declare_id! to match keypair");
    eprintln!();
    eprintln!("Seed formats:");
    eprintln!("  b\"text\"       UTF-8 bytes of `text`");
    eprintln!("  hex:0a1b2c     Hex bytes");
    eprintln!("  base58:...     Base58 bytes (for pubkey seeds)");
    eprintln!("  raw            Otherwise treated as raw UTF-8");
}

/// Rewrite the in-source `declare_id!("...")` (or `hopper::declare_id!("...")`)
/// invocation to match the keypair at `path`. Mirrors the
/// `quasar keys sync` verb so a freshly-minted keypair (`hopper keys new
/// target/deploy/my_program-keypair.json`) immediately propagates into
/// the program's source without manual paste. Searches `src/lib.rs` by
/// default; pass `--src <file.rs>` to point at a different file.
fn cmd_sync(args: &[String]) {
    let mut keypair_path: Option<PathBuf> = None;
    let mut src_path: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--src" => {
                if i + 1 >= args.len() {
                    eprintln!("--src requires a path argument");
                    process::exit(1);
                }
                src_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--help" | "-h" => {
                eprintln!(
                    "Usage: hopper keys sync <keypair-path> [--src <file.rs>]\n\
                     Default --src is `src/lib.rs` relative to the working directory."
                );
                return;
            }
            other if !other.starts_with("--") && keypair_path.is_none() => {
                keypair_path = Some(PathBuf::from(other));
                i += 1;
            }
            other => {
                eprintln!("Unknown argument: {other}");
                process::exit(1);
            }
        }
    }
    let kp_path = match keypair_path {
        Some(p) => p,
        None => {
            eprintln!("Usage: hopper keys sync <keypair-path> [--src <file.rs>]");
            process::exit(1);
        }
    };
    let src_path = src_path.unwrap_or_else(|| PathBuf::from("src/lib.rs"));

    // Read the keypair, derive pubkey.
    let bytes = match read_keypair_json(&kp_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("failed to read keypair {}: {e}", kp_path.display());
            process::exit(1);
        }
    };
    if bytes.len() != 64 {
        eprintln!(
            "expected 64-byte keypair JSON array, got {} bytes",
            bytes.len()
        );
        process::exit(1);
    }
    let pubkey_b58 = bs58::encode(&bytes[32..]).into_string();

    // Read source, replace the first `declare_id!("...")` argument.
    let src = match fs::read_to_string(&src_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to read {}: {e}", src_path.display());
            process::exit(1);
        }
    };
    let (rewritten, replaced) = replace_declare_id(&src, &pubkey_b58);
    if !replaced {
        eprintln!(
            "no `declare_id!(\"...\")` invocation found in {}",
            src_path.display()
        );
        process::exit(1);
    }
    if rewritten == src {
        println!(
            "{} already matches keypair {} ({})",
            src_path.display(),
            kp_path.display(),
            pubkey_b58
        );
        return;
    }
    if let Err(e) = fs::write(&src_path, &rewritten) {
        eprintln!("failed to write {}: {e}", src_path.display());
        process::exit(1);
    }
    println!(
        "synced declare_id! in {} to {} (from {})",
        src_path.display(),
        pubkey_b58,
        kp_path.display()
    );
}

/// Read a Solana-CLI-style keypair JSON file `[u8, u8, ...]` and
/// return the 64 raw bytes.
fn read_keypair_json(path: &Path) -> Result<Vec<u8>, String> {
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let trimmed = raw.trim().trim_start_matches('[').trim_end_matches(']');
    let mut out = Vec::with_capacity(64);
    for tok in trimmed.split(',') {
        let n = tok.trim();
        if n.is_empty() {
            continue;
        }
        let v: u8 = n
            .parse()
            .map_err(|e: std::num::ParseIntError| format!("byte {n}: {e}"))?;
        out.push(v);
    }
    Ok(out)
}

/// Replace the first `declare_id!("...")` (also matches
/// `hopper::declare_id!`, `crate::declare_id!`, etc.) string literal
/// with `new_id`. Returns `(rewritten_source, was_replaced)`.
fn replace_declare_id(src: &str, new_id: &str) -> (String, bool) {
    // Find a `declare_id!` token followed by `(` then the first
    // string literal up to the matching close-quote. The macro
    // path may carry a `crate::`, `hopper::`, etc. prefix, so we
    // anchor on the literal `declare_id!` token.
    let needle = "declare_id!";
    let Some(start) = src.find(needle) else {
        return (src.to_string(), false);
    };
    let after = start + needle.len();
    let rest = &src[after..];
    // Skip optional whitespace, then `(`.
    let open = rest.bytes().position(|b| !b.is_ascii_whitespace());
    let Some(open_off) = open else {
        return (src.to_string(), false);
    };
    if rest.as_bytes()[open_off] != b'(' {
        return (src.to_string(), false);
    }
    // Skip whitespace inside the parens; require an opening `"`.
    let after_open = open_off + 1;
    let mut q = after_open;
    while q < rest.len() && rest.as_bytes()[q].is_ascii_whitespace() {
        q += 1;
    }
    if q >= rest.len() || rest.as_bytes()[q] != b'"' {
        return (src.to_string(), false);
    }
    let lit_start = q + 1;
    // Find the closing quote (no escape handling needed for a base58
    // string literal).
    let lit_end_rel = match rest[lit_start..].find('"') {
        Some(p) => p,
        None => return (src.to_string(), false),
    };
    let abs_lit_start = after + lit_start;
    let abs_lit_end = after + lit_start + lit_end_rel;
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..abs_lit_start]);
    out.push_str(new_id);
    out.push_str(&src[abs_lit_end..]);
    (out, true)
}

fn cmd_new(args: &[String]) {
    let Some(path) = args.first() else {
        eprintln!("Usage: hopper keys new <path>");
        process::exit(1);
    };
    let path = Path::new(path);
    if path.exists() {
        eprintln!("refusing to overwrite existing file: {}", path.display());
        eprintln!("delete it first or choose a different path");
        process::exit(1);
    }
    let seed = random_seed();
    let keypair_bytes = keypair_from_seed(&seed);
    let json = format!(
        "[{}]",
        keypair_bytes.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",")
    );
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    if let Err(e) = fs::write(path, &json) {
        eprintln!("failed to write {}: {e}", path.display());
        process::exit(1);
    }
    let pubkey = &keypair_bytes[32..];
    println!("wrote keypair to {}", path.display());
    println!("pubkey: {}", bs58::encode(pubkey).into_string());
}

fn cmd_list(args: &[String]) {
    let paths: Vec<PathBuf> = if args.is_empty() {
        // Default: scan target/deploy for *.json
        let deploy_dir = Path::new("target/deploy");
        if !deploy_dir.is_dir() {
            eprintln!("no target/deploy/ directory; pass keypair paths explicitly");
            process::exit(0);
        }
        fs::read_dir(deploy_dir)
            .ok()
            .map(|it| {
                it.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        args.iter().map(PathBuf::from).collect()
    };

    if paths.is_empty() {
        println!("no keypair files found");
        return;
    }

    let width = paths.iter().map(|p| p.display().to_string().len()).max().unwrap_or(0);
    for path in paths {
        match load_pubkey(&path) {
            Ok(pk) => println!("{:<width$}  {}", path.display(), pk, width = width),
            Err(e) => println!("{:<width$}  ! {}", path.display(), e, width = width),
        }
    }
}

fn cmd_print(args: &[String]) {
    let Some(path) = args.first() else {
        eprintln!("Usage: hopper keys print <path>");
        process::exit(1);
    };
    match load_pubkey(Path::new(path)) {
        Ok(pk) => println!("{pk}"),
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    }
}

fn cmd_pda(args: &[String]) {
    let mut seeds: Vec<Vec<u8>> = Vec::new();
    let mut program_id: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--program" => {
                i += 1;
                program_id = args.get(i).cloned();
            }
            s => {
                seeds.push(parse_seed(s).unwrap_or_else(|e| {
                    eprintln!("bad seed `{s}`: {e}");
                    process::exit(1);
                }));
            }
        }
        i += 1;
    }
    let program_id = program_id.unwrap_or_else(|| {
        eprintln!("`--program <id>` is required");
        process::exit(1);
    });
    let program_bytes = bs58::decode(&program_id).into_vec().unwrap_or_else(|e| {
        eprintln!("invalid base58 program id: {e}");
        process::exit(1);
    });
    if program_bytes.len() != 32 {
        eprintln!("program id must be 32 bytes, got {}", program_bytes.len());
        process::exit(1);
    }
    // Find the canonical bump by scanning 255..=0.
    let seed_slices: Vec<&[u8]> = seeds.iter().map(|s| s.as_slice()).collect();
    let (pda, bump) = find_program_address(&seed_slices, &program_bytes);
    println!("PDA:    {}", bs58::encode(pda).into_string());
    println!("bump:   {bump}");
    println!("seeds:  {}", describe_seeds(&seeds));
}

// ---- helpers ---------------------------------------------------------------

fn random_seed() -> [u8; 32] {
    // getrandom via /dev/urandom on unix, CryptGenRandom on windows.
    let mut out = [0u8; 32];
    getrandom_bytes(&mut out);
    out
}

fn getrandom_bytes(out: &mut [u8]) {
    // Cross-platform host RNG (Unix: /dev/urandom, Windows: BCryptGenRandom
    // via `getrandom`). Failure is a CLI panic because we are minting key
    // material and a silent fallback would be worse than aborting.
    getrandom::getrandom(out).expect("hopper keys new: host RNG unavailable");
}

fn keypair_from_seed(seed: &[u8; 32]) -> [u8; 64] {
    // Ed25519 expanded keypair layout matching solana-keygen:
    // [0..32]  secret seed
    // [32..64] public key
    let mut hash = Sha512::digest(seed);
    hash[0] &= 248;
    hash[31] &= 127;
    hash[31] |= 64;
    let mut scalar_bytes = [0u8; 32];
    scalar_bytes.copy_from_slice(&hash[..32]);
    let scalar = Scalar::from_bytes_mod_order(scalar_bytes);
    let point = &scalar * ED25519_BASEPOINT_TABLE;
    let compressed = point.compress();
    let mut out = [0u8; 64];
    out[..32].copy_from_slice(seed);
    out[32..].copy_from_slice(compressed.as_bytes());
    out
}

fn load_pubkey(path: &Path) -> Result<String, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read failed: {e}"))?;
    let parsed: Vec<u8> = parse_keypair_json(&text)?;
    if parsed.len() != 64 {
        return Err(format!("expected 64-byte keypair, got {}", parsed.len()));
    }
    Ok(bs58::encode(&parsed[32..]).into_string())
}

fn parse_keypair_json(text: &str) -> Result<Vec<u8>, String> {
    let trimmed = text.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Err("expected JSON byte array like `[1,2,...]`".into());
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let mut out = Vec::with_capacity(64);
    for part in inner.split(',') {
        let n: u8 = part
            .trim()
            .parse()
            .map_err(|e| format!("bad byte `{}`: {e}", part.trim()))?;
        out.push(n);
    }
    Ok(out)
}

fn parse_seed(s: &str) -> Result<Vec<u8>, String> {
    if let Some(rest) = s.strip_prefix("b\"") {
        let inner = rest.strip_suffix('"').ok_or("unterminated b\"\" literal")?;
        return Ok(inner.as_bytes().to_vec());
    }
    if let Some(rest) = s.strip_prefix("hex:") {
        return hex_decode(rest);
    }
    if let Some(rest) = s.strip_prefix("base58:") {
        return bs58::decode(rest).into_vec().map_err(|e| format!("bad base58: {e}"));
    }
    Ok(s.as_bytes().to_vec())
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("hex string must have even length".into());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks_exact(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("not a hex digit: {}", b as char)),
    }
}

fn describe_seeds(seeds: &[Vec<u8>]) -> String {
    let mut parts = Vec::with_capacity(seeds.len());
    for s in seeds {
        if s.iter().all(|b| b.is_ascii() && !b.is_ascii_control()) {
            parts.push(format!("b\"{}\"", String::from_utf8_lossy(s)));
        } else if s.len() == 32 {
            parts.push(format!("base58:{}", bs58::encode(s).into_string()));
        } else {
            parts.push(format!("hex:{}", hex_encode(s)));
        }
    }
    parts.join(", ")
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Find the canonical (address, bump) for a set of seeds under a
/// program ID. Walks bumps from 255 down to 0, stopping at the first
/// bump that yields an off-curve point.
fn find_program_address(seeds: &[&[u8]], program_id: &[u8]) -> ([u8; 32], u8) {
    for bump in (0u8..=255).rev() {
        if let Some(addr) = create_program_address(seeds, bump, program_id) {
            return (addr, bump);
        }
    }
    panic!("no valid PDA exists for these seeds; extremely unlikely");
}

fn create_program_address(seeds: &[&[u8]], bump: u8, program_id: &[u8]) -> Option<[u8; 32]> {
    const PDA_MARKER: &[u8] = b"ProgramDerivedAddress";
    let mut hasher = Sha256Hasher::new();
    for s in seeds {
        if s.len() > 32 {
            return None;
        }
        hasher.update(s);
    }
    hasher.update(&[bump]);
    hasher.update(program_id);
    hasher.update(PDA_MARKER);
    let hash = hasher.finalize();
    // If the hash is on the ed25519 curve, it is NOT a valid PDA.
    if is_on_curve(&hash) {
        return None;
    }
    Some(hash)
}

fn is_on_curve(bytes: &[u8; 32]) -> bool {
    curve25519_dalek::edwards::CompressedEdwardsY(*bytes)
        .decompress()
        .is_some()
}

/// Tiny wrapper over `sha2::Sha256` to sidestep the incremental-update
/// boilerplate at the call site.
struct Sha256Hasher(sha2::Sha256);
impl Sha256Hasher {
    fn new() -> Self {
        Self(sha2::Sha256::new())
    }
    fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
    fn finalize(self) -> [u8; 32] {
        let result = self.0.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&result);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_declare_id_handles_bare_form() {
        let src = "use hopper::declare_id;\n\
                   declare_id!(\"OLDIDOLDIDOLDIDOLDIDOLDIDOLDIDOLDIDOLDIDOLDID\");\n";
        let (out, replaced) =
            replace_declare_id(src, "NEWIDNEWIDNEWIDNEWIDNEWIDNEWIDNEWIDNEWIDNEW1");
        assert!(replaced);
        assert!(out.contains("declare_id!(\"NEWIDNEWIDNEWIDNEWIDNEWIDNEWIDNEWIDNEWIDNEW1\")"));
        assert!(!out.contains("OLDID"));
    }

    #[test]
    fn replace_declare_id_handles_qualified_form() {
        // The path-qualified form `hopper::declare_id!(...)` is
        // anchored on the `declare_id!` token regardless of prefix.
        let src = "hopper::declare_id!( \"OLDOLDOLDOLDOLDOLDOLDOLDOLDOLDOLDOLDOLDOLD11\" );\n";
        let (out, replaced) =
            replace_declare_id(src, "NEWID11111111111111111111111111111111111112");
        assert!(replaced);
        assert!(
            out.contains("declare_id!( \"NEWID11111111111111111111111111111111111112\" )"),
            "rewrite preserved spacing inside parens: {out:?}"
        );
    }

    #[test]
    fn replace_declare_id_returns_unchanged_when_absent() {
        let src = "fn main() { println!(\"hi\"); }\n";
        let (out, replaced) = replace_declare_id(src, "ANY");
        assert!(!replaced);
        assert_eq!(out, src);
    }

    #[test]
    fn replace_declare_id_only_touches_first_macro() {
        // If for some reason a file has multiple, we rewrite only
        // the first. Solana convention is one declare_id! per
        // program crate, so this matches expectation.
        let src = "declare_id!(\"AAA111111111111111111111111111111111111111A\");\n\
                   declare_id!(\"BBB111111111111111111111111111111111111111B\");\n";
        let (out, replaced) =
            replace_declare_id(src, "ZZZ111111111111111111111111111111111111111Z");
        assert!(replaced);
        assert!(out.contains("declare_id!(\"ZZZ111111111111111111111111111111111111111Z\")"));
        assert!(out.contains("declare_id!(\"BBB111111111111111111111111111111111111111B\")"));
    }
}
