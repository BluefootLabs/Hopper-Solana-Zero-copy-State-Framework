//! `hopper publish-idl` - native-Rust IDL publishing to the SPL Program
//! Metadata program, with **zero Node dependencies**.
//!
//! Anchor's `anchor idl build && anchor idl init` path shells out to
//! `npx @solana-program/program-metadata` to write a program's IDL to the
//! canonical on-chain metadata account. Hopper already projects a
//! manifest into Anchor-IDL JSON (see
//! [`hopper_schema::anchor_idl::AnchorIdlFromManifest`]); this module adds
//! the last mile - deriving the metadata PDA, building the 96-byte account
//! header, and zlib-packing the payload - entirely in Rust so `hopper
//! publish-idl` needs no `npx`, no `node_modules`, and no second toolchain.
//!
//! ## Protocol (verified against `solana-program/program-metadata`)
//!
//! Program id: `ProgM6JCCvbYkfKqJYHePx4xxSUSqJp7rh8Lyv7nk7S`.
//!
//! On-chain `Header` is `#[repr(C)]`, 1-byte aligned, exactly 96 bytes,
//! with the associated data section starting at offset 96:
//!
//! ```text
//! off  size  field
//!   0    1   discriminator: u8        (Metadata = 2)
//!   1   32   program: [u8; 32]
//!  33   32   authority: ZeroableOption<[u8;32]>  (all-zero = None)
//!  65    1   mutable: u8
//!  66    1   canonical: u8
//!  67   16   seed: [u8; 16]           (b"idl" right-padded with zeros)
//!  83    1   encoding: u8             (Utf8 = 1)
//!  84    1   compression: u8          (Zlib = 2)
//!  85    1   format: u8               (Json = 1)
//!  86    1   data_source: u8          (Direct = 0)
//!  87    4   data_length: [u8; 4]     (LE u32; length of stored payload)
//!  91    5   _padding: [u8; 5]
//! ```
//!
//! `ZeroableOption<Address>` is a zero-overhead newtype (`struct
//! ZeroableOption<T>(T)`), so `None` is 32 zero bytes with no extra flag -
//! that is why the header sums to exactly 96 and is 8-aligned. The
//! enum values (`Encoding`, `Compression`, `Format`, `DataSource`) and the
//! `Metadata = 2` discriminator were read from the program's
//! `program/src/state/mod.rs`.
//!
//! ## PDA derivation
//!
//! * canonical:     `[program_id, seed16]`
//! * non-canonical: `[program_id, authority, seed16]`
//!
//! where `seed16` is the **fixed 16-byte** seed field, i.e. the reference
//! client's `fixEncoderSize(getUtf8Encoder(), 16)` - the seed string is
//! UTF-8 encoded then right-padded (or truncated) to 16 bytes, and that
//! fixed array is the PDA seed. For explorer interop, IDLs live under the
//! canonical `[program, b"idl"]` PDA as Utf8 + Zlib + Json.
//!
//! ## Scope
//!
//! This pass is the correctness-critical, unit-testable core plus a
//! `--dry-run` that derives + previews without touching the network. The
//! actual signed on-chain send and large-IDL buffer chunking are deferred
//! followups (they need devnet and a signer).

use std::process;

use bs58;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Protocol constants
// ---------------------------------------------------------------------------

/// SPL Program Metadata program id (base58).
pub const METADATA_PROGRAM_ID_B58: &str = "ProgM6JCCvbYkfKqJYHePx4xxSUSqJp7rh8Lyv7nk7S";

/// On-chain `Header` length. Payload begins immediately after.
pub const HEADER_LEN: usize = 96;

/// Fixed seed-field width (`SEED_LEN` in the on-chain program).
pub const SEED_LEN: usize = 16;

/// Default metadata seed for a program IDL.
pub const DEFAULT_SEED: &[u8] = b"idl";

/// System program id (base58), shown in the instruction plan.
pub const SYSTEM_PROGRAM_ID_B58: &str = "11111111111111111111111111111111";

/// `AccountDiscriminator::Metadata`.
pub const DISC_METADATA: u8 = 2;

/// `Encoding::Utf8`.
pub const ENCODING_UTF8: u8 = 1;
/// `Compression::Zlib`.
pub const COMPRESSION_ZLIB: u8 = 2;
/// `Format::Json`.
pub const FORMAT_JSON: u8 = 1;
/// `DataSource::Direct` (payload stored inline in the account).
pub const DATA_SOURCE_DIRECT: u8 = 0;

/// Marker appended by `create_program_address` before hashing.
const PDA_MARKER: &[u8] = b"ProgramDerivedAddress";

// ---------------------------------------------------------------------------
// Seed handling
// ---------------------------------------------------------------------------

/// Encode a seed string into the fixed 16-byte seed field: UTF-8 bytes,
/// right-padded with zeros (or truncated to 16). Mirrors the reference
/// client's `fixEncoderSize(getUtf8Encoder(), 16)`.
pub fn pad_seed(seed: &[u8]) -> [u8; SEED_LEN] {
    let mut out = [0u8; SEED_LEN];
    let n = seed.len().min(SEED_LEN);
    out[..n].copy_from_slice(&seed[..n]);
    out
}

/// Decode the metadata program id into its 32 raw bytes.
pub fn metadata_program_id() -> [u8; 32] {
    let v = bs58::decode(METADATA_PROGRAM_ID_B58)
        .into_vec()
        .expect("metadata program id is valid base58");
    let mut out = [0u8; 32];
    out.copy_from_slice(&v);
    out
}

// ---------------------------------------------------------------------------
// PDA derivation (standard create/find_program_address over sha256)
// ---------------------------------------------------------------------------

/// Compute a candidate program-derived address for `seeds + [bump]` under
/// `program_id`. Returns `None` when a seed exceeds 32 bytes or the result
/// lands on the ed25519 curve (i.e. is not a valid PDA).
fn create_program_address(seeds: &[&[u8]], bump: u8, program_id: &[u8; 32]) -> Option<[u8; 32]> {
    let mut hasher = Sha256::new();
    for s in seeds {
        if s.len() > 32 {
            return None;
        }
        hasher.update(s);
    }
    hasher.update([bump]);
    hasher.update(program_id);
    hasher.update(PDA_MARKER);
    let digest = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&digest);
    if is_on_curve(&hash) {
        return None;
    }
    Some(hash)
}

/// Find the canonical `(address, bump)` for `seeds` under `program_id`,
/// walking bumps from 255 down to the first off-curve result.
fn find_program_address(seeds: &[&[u8]], program_id: &[u8; 32]) -> ([u8; 32], u8) {
    for bump in (0u8..=255).rev() {
        if let Some(addr) = create_program_address(seeds, bump, program_id) {
            return (addr, bump);
        }
    }
    // Astronomically unlikely: every bump 255..=0 was on-curve.
    panic!("no off-curve PDA exists for these seeds");
}

fn is_on_curve(bytes: &[u8; 32]) -> bool {
    curve25519_dalek::edwards::CompressedEdwardsY(*bytes)
        .decompress()
        .is_some()
}

/// Derive the canonical metadata PDA: `[program_id, seed16]` under the
/// metadata program.
pub fn derive_canonical_pda(program_id: &[u8; 32], seed16: &[u8; SEED_LEN]) -> ([u8; 32], u8) {
    let meta = metadata_program_id();
    find_program_address(&[program_id.as_ref(), seed16.as_ref()], &meta)
}

/// Derive a non-canonical metadata PDA: `[program_id, authority, seed16]`
/// under the metadata program. Part of the correctness-critical core and
/// pinned by tests; the `--non-canonical` CLI path is a followup, so this
/// is not yet wired into the (canonical-only) dry-run command.
#[allow(dead_code)]
pub fn derive_non_canonical_pda(
    program_id: &[u8; 32],
    authority: &[u8; 32],
    seed16: &[u8; SEED_LEN],
) -> ([u8; 32], u8) {
    let meta = metadata_program_id();
    find_program_address(
        &[program_id.as_ref(), authority.as_ref(), seed16.as_ref()],
        &meta,
    )
}

// ---------------------------------------------------------------------------
// 96-byte header encoder
// ---------------------------------------------------------------------------

/// In-memory view of the on-chain metadata `Header`. `encode` produces the
/// exact 96 bytes that will occupy `[0, 96)` of the account.
#[derive(Debug, Clone)]
pub struct MetadataHeader {
    /// Account discriminator (`DISC_METADATA` for a metadata account).
    pub discriminator: u8,
    /// The program this metadata describes.
    pub program: [u8; 32],
    /// Optional authority; `None` writes 32 zero bytes (the Zeroable None).
    pub authority: Option<[u8; 32]>,
    /// Whether the metadata can be updated later.
    pub mutable: bool,
    /// Whether this is the canonical (upgrade-authority) metadata account.
    pub canonical: bool,
    /// Fixed 16-byte seed field.
    pub seed: [u8; SEED_LEN],
    /// Payload encoding (`ENCODING_UTF8`).
    pub encoding: u8,
    /// Payload compression (`COMPRESSION_ZLIB`).
    pub compression: u8,
    /// Payload format (`FORMAT_JSON`).
    pub format: u8,
    /// Where the payload lives (`DATA_SOURCE_DIRECT`).
    pub data_source: u8,
    /// Length of the stored (post-compression) payload in bytes.
    pub data_len: u32,
}

impl MetadataHeader {
    /// Build the canonical IDL header describing `program`, recording the
    /// prepared `payload`'s encoding/compression/format and stored length.
    /// The payload tags are the single source of truth for these fields.
    pub fn canonical_idl(
        program: [u8; 32],
        seed16: [u8; SEED_LEN],
        payload: &PreparedPayload,
    ) -> Self {
        MetadataHeader {
            discriminator: DISC_METADATA,
            program,
            authority: None,
            mutable: true,
            canonical: true,
            seed: seed16,
            encoding: payload.encoding,
            compression: payload.compression,
            format: payload.format,
            data_source: DATA_SOURCE_DIRECT,
            data_len: payload.data_len(),
        }
    }

    /// Serialize to the exact 96 on-chain header bytes.
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0] = self.discriminator;
        b[1..33].copy_from_slice(&self.program);
        // authority: Some -> the 32 bytes; None -> left as zeros (Zeroable None).
        if let Some(auth) = &self.authority {
            b[33..65].copy_from_slice(auth);
        }
        b[65] = self.mutable as u8;
        b[66] = self.canonical as u8;
        b[67..83].copy_from_slice(&self.seed);
        b[83] = self.encoding;
        b[84] = self.compression;
        b[85] = self.format;
        b[86] = self.data_source;
        b[87..91].copy_from_slice(&self.data_len.to_le_bytes());
        // b[91..96] padding stays zero.
        b
    }
}

// ---------------------------------------------------------------------------
// Payload prep (zlib)
// ---------------------------------------------------------------------------

/// Zlib-compress `data` using the same backend as the manifest encoder.
pub fn zlib_compress(data: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).expect("zlib write");
    encoder.finish().expect("zlib finish")
}

/// Zlib-decompress `data` (round-trip inverse of [`zlib_compress`]).
/// Used by the compression round-trip tests and by the signed-send
/// followup that will read back and verify a published account.
#[allow(dead_code)]
pub fn zlib_decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;
    let mut out = Vec::new();
    ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| format!("zlib decompression failed: {e}"))?;
    Ok(out)
}

/// Prepared payload: the raw IDL bytes, the compressed bytes, and the
/// header field triple to record.
pub struct PreparedPayload {
    /// Raw JSON length before compression.
    pub raw_len: usize,
    /// Zlib-compressed payload bytes (what actually gets stored).
    pub compressed: Vec<u8>,
    /// Encoding tag to record in the header.
    pub encoding: u8,
    /// Compression tag to record in the header.
    pub compression: u8,
    /// Format tag to record in the header.
    pub format: u8,
}

impl PreparedPayload {
    /// Prepare an Anchor-IDL JSON string for on-chain storage as
    /// Utf8 + Zlib + Json.
    pub fn from_idl_json(idl_json: &str) -> Self {
        let compressed = zlib_compress(idl_json.as_bytes());
        PreparedPayload {
            raw_len: idl_json.len(),
            compressed,
            encoding: ENCODING_UTF8,
            compression: COMPRESSION_ZLIB,
            format: FORMAT_JSON,
        }
    }

    /// Stored payload length (post-compression), the header's `data_len`.
    pub fn data_len(&self) -> u32 {
        self.compressed.len() as u32
    }
}

// ---------------------------------------------------------------------------
// Hex helper
// ---------------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

// ---------------------------------------------------------------------------
// Dry-run rendering
// ---------------------------------------------------------------------------

/// Render the full `--dry-run` report as a string: derived PDA, prepared
/// payload sizes, the 96-byte header (hex + decoded fields), and the
/// instruction plan that a signed send *would* submit. Pure and
/// side-effect free so it is directly unit-testable.
pub fn render_dry_run(
    program_id: &[u8; 32],
    seed16: &[u8; SEED_LEN],
    idl_json: &str,
    manifest_name: &str,
    manifest_version: &str,
) -> String {
    let program_b58 = bs58::encode(program_id).into_string();
    let (pda, bump) = derive_canonical_pda(program_id, seed16);
    let pda_b58 = bs58::encode(pda).into_string();

    let payload = PreparedPayload::from_idl_json(idl_json);
    let header = MetadataHeader::canonical_idl(*program_id, *seed16, &payload);
    let header_bytes = header.encode();

    // Render the seed field: printable prefix + hex of the fixed 16 bytes.
    let seed_text =
        String::from_utf8_lossy(&seed16[..seed16.iter().position(|&b| b == 0).unwrap_or(SEED_LEN)])
            .into_owned();

    let mut out = String::new();
    use std::fmt::Write as _;

    let _ = writeln!(out, "=== hopper publish-idl (dry run) ===");
    let _ = writeln!(out);
    let _ = writeln!(out, "Program:            {program_b58}");
    let _ = writeln!(out, "Metadata program:   {METADATA_PROGRAM_ID_B58}");
    let _ = writeln!(
        out,
        "Seed:               \"{seed_text}\"  (hex {})",
        hex_encode(seed16)
    );
    let _ = writeln!(out, "Derivation:         canonical [program, seed]");
    let _ = writeln!(out, "Metadata PDA:       {pda_b58}  (bump {bump})");
    let _ = writeln!(out);

    let _ = writeln!(out, "Payload:");
    let _ = writeln!(
        out,
        "  Source:           Anchor IDL JSON (projected from manifest {manifest_name} v{manifest_version})"
    );
    let _ = writeln!(out, "  Raw size:         {} bytes", payload.raw_len);
    let _ = writeln!(
        out,
        "  Stored size:      {} bytes  (encoding=Utf8, compression=Zlib, format=Json)",
        payload.data_len()
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "Account header ({HEADER_LEN} bytes):");
    let _ = writeln!(out, "  {}", hex_encode(&header_bytes));
    let _ = writeln!(
        out,
        "  disc={} mutable={} canonical={} encoding={} compression={} format={} data_source={} data_len={}",
        header.discriminator,
        header.mutable as u8,
        header.canonical as u8,
        header.encoding,
        header.compression,
        header.format,
        header.data_source,
        header.data_len,
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "Instruction plan (NOT sent):");
    let _ = writeln!(out, "  program:  {METADATA_PROGRAM_ID_B58}");
    let _ = writeln!(out, "  accounts:");
    let _ = writeln!(out, "    [0] metadata PDA   {pda_b58}  (writable)");
    let _ = writeln!(out, "    [1] program        {program_b58}  (readonly)");
    let _ = writeln!(
        out,
        "    [2] authority/payer <signer required>  (signer, writable)"
    );
    let _ = writeln!(out, "    [3] system program {SYSTEM_PROGRAM_ID_B58}");
    let _ = writeln!(
        out,
        "  data:     96-byte header above + {} zlib bytes",
        payload.data_len()
    );
    let _ = writeln!(
        out,
        "  note:     concrete instruction-data encoding + signing land in the signed-send followup."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Dry run only. No transaction was created or sent.");

    out
}

// ---------------------------------------------------------------------------
// CLI entry point
// ---------------------------------------------------------------------------

fn print_usage() {
    eprintln!("Usage: hopper publish-idl --manifest <path> --program-id <pubkey> [--dry-run] [--seed <str>]");
    eprintln!();
    eprintln!("Publish a program's Anchor-IDL JSON to the SPL Program Metadata");
    eprintln!("program (canonical [program, \"idl\"] PDA), with zero Node dependencies.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --manifest <path>     Hopper program manifest JSON (also accepts inline/@file)");
    eprintln!("  --program-id <pubkey> Base58 program id the IDL describes");
    eprintln!(
        "  --dry-run             Derive + preview the header/PDA/instruction without sending"
    );
    eprintln!("  --seed <str>          Metadata seed (default \"idl\", padded to 16 bytes)");
}

/// `hopper publish-idl` entry point.
pub fn cmd_publish_idl(args: &[String]) {
    let mut manifest_arg: Option<String> = None;
    let mut program_id_arg: Option<String> = None;
    let mut seed_arg: Option<String> = None;
    let mut dry_run = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" => {
                manifest_arg = args.get(i + 1).cloned();
                if manifest_arg.is_none() {
                    eprintln!("--manifest requires a path argument");
                    process::exit(1);
                }
                i += 2;
            }
            "--program-id" => {
                program_id_arg = args.get(i + 1).cloned();
                if program_id_arg.is_none() {
                    eprintln!("--program-id requires a base58 pubkey argument");
                    process::exit(1);
                }
                i += 2;
            }
            "--seed" => {
                seed_arg = args.get(i + 1).cloned();
                if seed_arg.is_none() {
                    eprintln!("--seed requires a string argument");
                    process::exit(1);
                }
                i += 2;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--help" | "-h" => {
                print_usage();
                return;
            }
            other => {
                eprintln!("Unknown publish-idl argument: {other}");
                print_usage();
                process::exit(1);
            }
        }
    }

    let manifest_arg = manifest_arg.unwrap_or_else(|| {
        eprintln!("--manifest is required");
        print_usage();
        process::exit(1);
    });
    let program_id_arg = program_id_arg.unwrap_or_else(|| {
        eprintln!("--program-id is required");
        print_usage();
        process::exit(1);
    });

    // Decode the program id.
    let program_bytes = bs58::decode(&program_id_arg)
        .into_vec()
        .unwrap_or_else(|e| {
            eprintln!("invalid base58 program id: {e}");
            process::exit(1);
        });
    if program_bytes.len() != 32 {
        eprintln!("program id must be 32 bytes, got {}", program_bytes.len());
        process::exit(1);
    }
    let mut program_id = [0u8; 32];
    program_id.copy_from_slice(&program_bytes);

    // Seed (default "idl"), padded to the fixed 16-byte field.
    let seed_bytes = seed_arg
        .as_deref()
        .map(|s| s.as_bytes().to_vec())
        .unwrap_or_else(|| DEFAULT_SEED.to_vec());
    if seed_bytes.len() > SEED_LEN {
        eprintln!(
            "seed is {} bytes; the metadata seed field is at most {SEED_LEN} bytes",
            seed_bytes.len()
        );
        process::exit(1);
    }
    let seed16 = pad_seed(&seed_bytes);

    // Load the manifest and project it into Anchor-IDL JSON. `--manifest`
    // is a path; `@`-prefix and inline JSON are also accepted via the
    // shared resolver.
    let load_arg = if manifest_arg.starts_with('@') || manifest_arg.trim_start().starts_with('{') {
        manifest_arg.clone()
    } else {
        format!("@{manifest_arg}")
    };
    let manifest = crate::load_program_manifest(&load_arg);
    let idl_json = format!(
        "{}",
        hopper_schema::anchor_idl::AnchorIdlFromManifest(&manifest)
    );

    if dry_run {
        print!(
            "{}",
            render_dry_run(
                &program_id,
                &seed16,
                &idl_json,
                manifest.name,
                manifest.version,
            )
        );
        return;
    }

    // Non-dry-run: the signed on-chain send is a deferred followup. Do not
    // half-send.
    eprintln!("not yet implemented: signed send (followup)");
    eprintln!();
    eprintln!("Re-run with --dry-run to derive the metadata PDA, preview the 96-byte");
    eprintln!("header, and inspect the instruction that a signed send would submit.");
    process::exit(1);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // A deterministic 32-byte program id used across PDA tests.
    fn sample_program() -> [u8; 32] {
        [1u8; 32]
    }

    #[test]
    fn seed_padding_is_right_padded_and_16_bytes() {
        let s = pad_seed(DEFAULT_SEED);
        assert_eq!(s.len(), SEED_LEN);
        // "idl" = 0x69 0x64 0x6c, then 13 zero bytes.
        assert_eq!(&s[..3], b"idl");
        assert!(s[3..].iter().all(|&b| b == 0), "tail must be zero-padded");
    }

    #[test]
    fn seed_padding_truncates_overlong_input() {
        let long = [0xABu8; 20];
        let s = pad_seed(&long);
        assert_eq!(s, [0xABu8; SEED_LEN]);
    }

    #[test]
    fn metadata_program_id_decodes_to_32_bytes() {
        let id = metadata_program_id();
        assert_eq!(id.len(), 32);
        // Round-trips back to the canonical base58 string.
        assert_eq!(bs58::encode(id).into_string(), METADATA_PROGRAM_ID_B58);
    }

    #[test]
    fn canonical_pda_is_deterministic() {
        let prog = sample_program();
        let seed = pad_seed(DEFAULT_SEED);
        let a = derive_canonical_pda(&prog, &seed);
        let b = derive_canonical_pda(&prog, &seed);
        assert_eq!(a, b, "same inputs must yield the same PDA + bump");
        // Result must itself be off-curve, and re-deriving with the found
        // bump must reproduce the same address.
        assert!(!is_on_curve(&a.0));
        let meta = metadata_program_id();
        let redo = create_program_address(&[prog.as_ref(), seed.as_ref()], a.1, &meta);
        assert_eq!(redo, Some(a.0));
    }

    #[test]
    fn canonical_and_non_canonical_differ() {
        let prog = sample_program();
        let authority = [7u8; 32];
        let seed = pad_seed(DEFAULT_SEED);
        let (canon, _) = derive_canonical_pda(&prog, &seed);
        let (non_canon, _) = derive_non_canonical_pda(&prog, &authority, &seed);
        assert_ne!(
            canon, non_canon,
            "adding the authority seed must change the derived address"
        );
    }

    #[test]
    fn different_seed_padding_changes_pda() {
        // The full 16-byte fixed field participates in derivation, so an
        // "idl" seed and an "idl\0...extra" seed with the SAME 3-byte prefix
        // but different padding must derive different addresses. This pins
        // that the padded field (not the trimmed slice) is the PDA seed.
        let prog = sample_program();
        let a = pad_seed(b"idl");
        let mut b = pad_seed(b"idl");
        b[8] = 0xFF; // perturb a padding byte
        let (pa, _) = derive_canonical_pda(&prog, &a);
        let (pb, _) = derive_canonical_pda(&prog, &b);
        assert_ne!(pa, pb);
    }

    #[test]
    fn header_encodes_byte_for_byte_fixture() {
        // Hand-computed fixture from the documented 96-byte layout.
        let header = MetadataHeader {
            discriminator: DISC_METADATA, // 2
            program: [0x01; 32],
            authority: None, // 32 zero bytes
            mutable: true,   // 1
            canonical: true, // 1
            seed: pad_seed(b"idl"),
            encoding: ENCODING_UTF8,         // 1
            compression: COMPRESSION_ZLIB,   // 2
            format: FORMAT_JSON,             // 1
            data_source: DATA_SOURCE_DIRECT, // 0
            data_len: 0x1234_5678,           // LE -> 78 56 34 12
        };

        let mut expected = [0u8; HEADER_LEN];
        expected[0] = 0x02; // disc = Metadata
        for b in expected[1..33].iter_mut() {
            *b = 0x01; // program
        }
        // authority 33..65 stays zero (None)
        expected[65] = 0x01; // mutable
        expected[66] = 0x01; // canonical
        expected[67] = 0x69; // 'i'
        expected[68] = 0x64; // 'd'
        expected[69] = 0x6c; // 'l'
                             // seed padding 70..83 stays zero
        expected[83] = 0x01; // encoding = Utf8
        expected[84] = 0x02; // compression = Zlib
        expected[85] = 0x01; // format = Json
        expected[86] = 0x00; // data_source = Direct
        expected[87] = 0x78; // data_len LE
        expected[88] = 0x56;
        expected[89] = 0x34;
        expected[90] = 0x12;
        // 91..96 padding stays zero

        assert_eq!(header.encode(), expected);
    }

    #[test]
    fn header_encodes_authority_some() {
        // The Some(authority) branch writes the 32 authority bytes at 33..65.
        let header = MetadataHeader {
            discriminator: DISC_METADATA,
            program: [0u8; 32],
            authority: Some([0xAA; 32]),
            mutable: false,
            canonical: false,
            seed: [0u8; SEED_LEN],
            encoding: ENCODING_UTF8,
            compression: COMPRESSION_ZLIB,
            format: FORMAT_JSON,
            data_source: DATA_SOURCE_DIRECT,
            data_len: 0,
        };
        let bytes = header.encode();
        assert!(bytes[33..65].iter().all(|&b| b == 0xAA));
        assert_eq!(bytes[65], 0); // mutable = false
        assert_eq!(bytes[66], 0); // canonical = false
    }

    #[test]
    fn header_len_is_96() {
        let header = MetadataHeader {
            discriminator: DISC_METADATA,
            program: [9u8; 32],
            authority: None,
            mutable: true,
            canonical: true,
            seed: pad_seed(b"idl"),
            encoding: ENCODING_UTF8,
            compression: COMPRESSION_ZLIB,
            format: FORMAT_JSON,
            data_source: DATA_SOURCE_DIRECT,
            data_len: 42,
        };
        assert_eq!(header.encode().len(), HEADER_LEN);
        // data_len lands little-endian at 87..91.
        assert_eq!(&header.encode()[87..91], &42u32.to_le_bytes());
    }

    #[test]
    fn canonical_idl_reads_payload_tags() {
        let payload = PreparedPayload::from_idl_json(r#"{"name":"demo"}"#);
        let header = MetadataHeader::canonical_idl([9u8; 32], pad_seed(b"idl"), &payload);
        assert_eq!(header.encoding, ENCODING_UTF8);
        assert_eq!(header.compression, COMPRESSION_ZLIB);
        assert_eq!(header.format, FORMAT_JSON);
        assert_eq!(header.data_len, payload.data_len());
    }

    #[test]
    fn zlib_round_trips() {
        let sample = br#"{"version":"0.1.0","name":"demo","instructions":[]}"#;
        let compressed = zlib_compress(sample);
        let restored = zlib_decompress(&compressed).expect("decompress");
        assert_eq!(restored, sample);
    }

    #[test]
    fn zlib_round_trips_large_repetitive_payload() {
        // Repetitive JSON-ish data should compress well and still restore.
        let big = "{\"field\":\"value\"},".repeat(4096);
        let compressed = zlib_compress(big.as_bytes());
        assert!(
            compressed.len() < big.len(),
            "repetitive payload should shrink"
        );
        let restored = zlib_decompress(&compressed).expect("decompress");
        assert_eq!(restored, big.as_bytes());
    }

    #[test]
    fn prepared_payload_reports_utf8_zlib_json() {
        let idl = r#"{"version":"0.1.0","name":"demo"}"#;
        let prepared = PreparedPayload::from_idl_json(idl);
        assert_eq!(prepared.raw_len, idl.len());
        assert_eq!(prepared.encoding, ENCODING_UTF8);
        assert_eq!(prepared.compression, COMPRESSION_ZLIB);
        assert_eq!(prepared.format, FORMAT_JSON);
        assert_eq!(prepared.data_len() as usize, prepared.compressed.len());
        // Compressed bytes decompress back to the original IDL.
        let restored = zlib_decompress(&prepared.compressed).expect("decompress");
        assert_eq!(restored, idl.as_bytes());
    }

    #[test]
    fn dry_run_output_contains_pda_and_header() {
        let prog = sample_program();
        let seed = pad_seed(DEFAULT_SEED);
        let idl = r#"{"version":"0.1.0","name":"demo","instructions":[]}"#;
        let report = render_dry_run(&prog, &seed, idl, "demo", "0.1.0");

        // Derived PDA must appear verbatim.
        let (pda, _) = derive_canonical_pda(&prog, &seed);
        let pda_b58 = bs58::encode(pda).into_string();
        assert!(
            report.contains(&pda_b58),
            "dry-run must print the derived metadata PDA"
        );

        // The 96-byte header (hex) must appear verbatim.
        let payload = PreparedPayload::from_idl_json(idl);
        let header = MetadataHeader::canonical_idl(prog, seed, &payload);
        let header_hex = hex_encode(&header.encode());
        assert!(
            report.contains(&header_hex),
            "dry-run must print the 96-byte header hex"
        );

        // Sanity: mentions the metadata program and does not claim to send.
        assert!(report.contains(METADATA_PROGRAM_ID_B58));
        assert!(report.contains("No transaction was created or sent"));
    }
}
