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
//! ## Instruction wire format (verified against the on-chain processor)
//!
//! The metadata program dispatches on the first instruction-data byte
//! (`entrypoint.rs`): `0 = Write`, `1 = Initialize`, `3 = SetData`,
//! `7 = Allocate`. The remaining bytes are the processor payload.
//!
//! * **Initialize** (`initialize.rs`): `[seed:16][encoding:1][compression:1]
//!   [format:1][data_source:1][payload…]`. When the target PDA is empty the
//!   program CPI-creates it (signed by the PDA seeds) with
//!   `space = 96 + payload.len()`, copies `payload` to offset 96, and writes
//!   the 96-byte header itself. The account must be **pre-funded** to rent
//!   exemption first (`CreateAccountAllowPrefund { funding: None }`). When the
//!   PDA is instead a pre-allocated `Buffer` (discriminator 1), no inline
//!   payload is allowed and the program finalizes the already-written bytes
//!   in place.
//! * **Allocate** (`allocate.rs`): `[seed:16]`. Creates a 96-byte `Buffer`
//!   header at the canonical PDA (must be pre-funded).
//! * **Write** (`write.rs`): `[offset:u32-le][chunk…]`. Copies `chunk` to
//!   `offset + 96` in the buffer, resizing as needed (so the account must be
//!   funded for its final size before the first write).
//! * **SetData** (`set_data.rs`): `[encoding:1][compression:1][format:1]
//!   [data_source:1][payload…]`. Rewrites the data section of an existing
//!   mutable metadata account (used by `--overwrite`).
//!
//! Account orderings are taken from the same source files and encoded in the
//! `*_accounts` helpers below. Canonical publishing requires the target
//! `program` account (executable) plus its BPF-Loader-v3 `ProgramData` PDA so
//! the program can verify the signer is the program's upgrade authority.
//!
//! ## Scope
//!
//! Shipped: the correctness-critical instruction encoders + PDA/header core
//! (unit-tested byte-for-byte), a `--dry-run` preview, and a **real signed
//! on-chain send** — fresh canonical publish via a single inline `Initialize`
//! for small IDLs, or `Allocate` + chunked `Write` + in-place `Initialize`
//! for IDLs that exceed one transaction's data budget, plus an inline
//! `--overwrite` (`SetData`) rewrite path. Rent pre-funding, "already
//! initialized", insufficient-balance, and RPC failures are surfaced
//! explicitly; nothing is half-written. The one bounded gap is *overwriting*
//! an IDL large enough to need chunking (a fresh large publish works): that
//! requires a separate source buffer + `SetData`-from-buffer and is reported
//! as a clear error pointing at close-then-republish.

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

// Instruction discriminators (first byte of the instruction data). Values
// pinned to the on-chain `ProgramMetadataInstruction` enum ordering.
/// `Write` instruction discriminator.
pub const INSTR_WRITE: u8 = 0;
/// `Initialize` instruction discriminator.
pub const INSTR_INITIALIZE: u8 = 1;
/// `SetData` instruction discriminator.
pub const INSTR_SET_DATA: u8 = 3;
/// `Allocate` instruction discriminator.
pub const INSTR_ALLOCATE: u8 = 7;

/// BPF Upgradeable Loader program id (base58); owns v3 programs and their
/// `ProgramData` accounts, from which the upgrade authority is read.
pub const BPF_LOADER_UPGRADEABLE_ID_B58: &str = "BPFLoaderUpgradeab1e11111111111111111111111";

/// Maximum compressed payload carried inline in a single `Initialize` /
/// `SetData` transaction. A Solana transaction serializes to at most ~1232
/// bytes; the inline `Initialize` data is `1 + 16 + 4 + payload`, and the tx
/// also carries a system transfer, five account metas, a signature and a
/// blockhash. 800 bytes leaves comfortable headroom; larger payloads use the
/// `Allocate` + chunked `Write` path.
pub const MAX_INLINE_PAYLOAD: usize = 800;

/// Maximum bytes per `Write` chunk when buffering a large payload. Each write
/// tx carries `1 + 4 + chunk` instruction bytes plus three account metas and a
/// signature, well under the transaction size limit at 900 bytes.
pub const WRITE_CHUNK_LEN: usize = 900;

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

/// Decode the BPF Upgradeable Loader program id into its 32 raw bytes.
pub fn bpf_loader_upgradeable_id() -> [u8; 32] {
    let v = bs58::decode(BPF_LOADER_UPGRADEABLE_ID_B58)
        .into_vec()
        .expect("bpf loader upgradeable id is valid base58");
    let mut out = [0u8; 32];
    out.copy_from_slice(&v);
    out
}

/// Derive a program's `ProgramData` PDA: `find_program_address([program_id])`
/// under the BPF Upgradeable Loader. This account holds the upgrade authority
/// that the metadata program checks for canonical publishing.
pub fn derive_program_data_pda(program_id: &[u8; 32]) -> ([u8; 32], u8) {
    let loader = bpf_loader_upgradeable_id();
    find_program_address(&[program_id.as_ref()], &loader)
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
// Instruction-data encoders (verified against the on-chain processor)
// ---------------------------------------------------------------------------

/// `Initialize` with inline data: `[1][seed:16][enc][comp][fmt][ds][payload…]`.
/// The program builds the 96-byte header from these tags and copies `payload`
/// to offset 96 of the freshly-created account.
pub fn build_initialize_inline_data(
    seed16: &[u8; SEED_LEN],
    encoding: u8,
    compression: u8,
    format: u8,
    data_source: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut d = Vec::with_capacity(1 + SEED_LEN + 4 + payload.len());
    d.push(INSTR_INITIALIZE);
    d.extend_from_slice(seed16);
    d.push(encoding);
    d.push(compression);
    d.push(format);
    d.push(data_source);
    d.extend_from_slice(payload);
    d
}

/// `Initialize` finalizing a pre-written `Buffer` in place (no inline payload):
/// `[1][seed:16][enc][comp][fmt][ds]`.
pub fn build_initialize_from_buffer_data(
    seed16: &[u8; SEED_LEN],
    encoding: u8,
    compression: u8,
    format: u8,
    data_source: u8,
) -> Vec<u8> {
    let mut d = Vec::with_capacity(1 + SEED_LEN + 4);
    d.push(INSTR_INITIALIZE);
    d.extend_from_slice(seed16);
    d.push(encoding);
    d.push(compression);
    d.push(format);
    d.push(data_source);
    d
}

/// `Allocate` a canonical PDA buffer: `[7][seed:16]`.
pub fn build_allocate_data(seed16: &[u8; SEED_LEN]) -> Vec<u8> {
    let mut d = Vec::with_capacity(1 + SEED_LEN);
    d.push(INSTR_ALLOCATE);
    d.extend_from_slice(seed16);
    d
}

/// `Write` a chunk at `offset`: `[0][offset:u32-le][chunk…]`. The program
/// stores `chunk` at `offset + 96` (past the buffer header).
pub fn build_write_data(offset: u32, chunk: &[u8]) -> Vec<u8> {
    let mut d = Vec::with_capacity(1 + 4 + chunk.len());
    d.push(INSTR_WRITE);
    d.extend_from_slice(&offset.to_le_bytes());
    d.extend_from_slice(chunk);
    d
}

/// `SetData` with inline data (the `--overwrite` rewrite path):
/// `[3][enc][comp][fmt][ds][payload…]`.
pub fn build_set_data_inline_data(
    encoding: u8,
    compression: u8,
    format: u8,
    data_source: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut d = Vec::with_capacity(1 + 4 + payload.len());
    d.push(INSTR_SET_DATA);
    d.push(encoding);
    d.push(compression);
    d.push(format);
    d.push(data_source);
    d.extend_from_slice(payload);
    d
}

/// Split `payload` into `(offset, chunk)` writes of at most `chunk_len` bytes.
/// Offsets are relative to the data section (the program adds the 96-byte
/// buffer-header offset itself), so they run `0, chunk_len, 2*chunk_len, …`.
pub fn plan_write_chunks(payload: &[u8], chunk_len: usize) -> Vec<(u32, &[u8])> {
    assert!(chunk_len > 0, "chunk_len must be positive");
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset < payload.len() {
        let end = (offset + chunk_len).min(payload.len());
        out.push((offset as u32, &payload[offset..end]));
        offset = end;
    }
    out
}

/// Reassemble the data section that a sequence of `Write` chunks would produce
/// on chain. Used by the dry-run-vs-send parity test to prove the buffered
/// path stores exactly the same bytes as the inline path.
#[allow(dead_code)] // exercised by the parity tests
pub fn reassemble_write_chunks(payload_len: usize, chunks: &[(u32, &[u8])]) -> Vec<u8> {
    let mut buf = vec![0u8; payload_len];
    for (offset, chunk) in chunks {
        let start = *offset as usize;
        buf[start..start + chunk.len()].copy_from_slice(chunk);
    }
    buf
}

/// The exact bytes the metadata account will hold after a successful publish:
/// the 96-byte canonical header followed by the compressed payload. Both the
/// inline and buffered send paths converge on this image, and it is what the
/// `--dry-run` header preview describes.
#[allow(dead_code)] // exercised by the parity tests
pub fn expected_account_image(
    program_id: &[u8; 32],
    seed16: &[u8; SEED_LEN],
    payload: &PreparedPayload,
) -> Vec<u8> {
    let header = MetadataHeader::canonical_idl(*program_id, *seed16, payload);
    let mut image = Vec::with_capacity(HEADER_LEN + payload.compressed.len());
    image.extend_from_slice(&header.encode());
    image.extend_from_slice(&payload.compressed);
    image
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

    let (program_data, _pd_bump) = derive_program_data_pda(program_id);
    let program_data_b58 = bs58::encode(program_data).into_string();

    let stored = payload.data_len() as usize;
    let inline = stored <= MAX_INLINE_PAYLOAD;

    let _ = writeln!(out, "Instruction plan (NOT sent):");
    let _ = writeln!(out, "  metadata program: {METADATA_PROGRAM_ID_B58}");
    let _ = writeln!(out, "  program data PDA: {program_data_b58}");
    if inline {
        let init = build_initialize_inline_data(
            seed16,
            payload.encoding,
            payload.compression,
            payload.format,
            DATA_SOURCE_DIRECT,
            &payload.compressed,
        );
        let _ = writeln!(out, "  path:     single inline Initialize (fits one tx)");
        let _ = writeln!(
            out,
            "  ix[0]:    SystemProgram::transfer  (rent pre-fund -> PDA)"
        );
        let _ = writeln!(
            out,
            "  ix[1]:    Initialize (disc {INSTR_INITIALIZE})  data {} bytes",
            init.len()
        );
        let _ = writeln!(
            out,
            "            accounts: [metadata(w) {pda_b58}], [authority(s,w)],"
        );
        let _ = writeln!(
            out,
            "                      [program(r) {program_b58}], [programData(r)], [system(r) {SYSTEM_PROGRAM_ID_B58}]"
        );
    } else {
        let chunks = plan_write_chunks(&payload.compressed, WRITE_CHUNK_LEN);
        let _ = writeln!(
            out,
            "  path:     Allocate + {} chunked Write(s) + in-place Initialize",
            chunks.len()
        );
        let _ = writeln!(
            out,
            "  tx[0]:    transfer(rent) + Allocate (disc {INSTR_ALLOCATE}, seed)"
        );
        for (idx, (offset, chunk)) in chunks.iter().enumerate() {
            let _ = writeln!(
                out,
                "  tx[{}]:    Write (disc {INSTR_WRITE}) offset {} len {}",
                idx + 1,
                offset,
                chunk.len()
            );
        }
        let _ = writeln!(
            out,
            "  tx[{}]:    Initialize-from-buffer (disc {INSTR_INITIALIZE}, no inline data)",
            chunks.len() + 1
        );
    }
    let _ = writeln!(
        out,
        "  result:   96-byte header (above) + {stored} zlib bytes at the PDA"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Dry run only. No transaction was created or sent.");

    out
}

// ---------------------------------------------------------------------------
// CLI entry point
// ---------------------------------------------------------------------------

fn print_usage() {
    eprintln!("Usage: hopper publish-idl --manifest <path> --program-id <pubkey>");
    eprintln!("                          [--url <rpc>] [--keypair <path>] [--seed <str>]");
    eprintln!("                          [--overwrite] [--dry-run]");
    eprintln!();
    eprintln!("Publish a program's Anchor-IDL JSON to the SPL Program Metadata");
    eprintln!("program (canonical [program, \"idl\"] PDA), with zero Node dependencies.");
    eprintln!();
    eprintln!("Without --dry-run this signs and submits the on-chain transaction(s):");
    eprintln!("a single inline Initialize for small IDLs, or Allocate + chunked Write");
    eprintln!("+ Initialize for IDLs larger than one transaction. The signer must be");
    eprintln!("the program's upgrade authority (canonical metadata).");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --manifest <path>     Hopper program manifest JSON (also accepts inline/@file)");
    eprintln!("  --program-id <pubkey> Base58 program id the IDL describes");
    eprintln!("  --url <rpc>           RPC endpoint (default: devnet). Also honors SOLANA_RPC_URL");
    eprintln!("  --keypair <path>      Upgrade-authority + fee-payer keypair");
    eprintln!("                        (default: ~/.config/solana/id.json)");
    eprintln!("  --seed <str>          Metadata seed (default \"idl\", padded to 16 bytes)");
    eprintln!("  --overwrite           Rewrite an already-initialized metadata account (SetData)");
    eprintln!(
        "  --dry-run             Derive + preview the header/PDA/instruction plan without sending"
    );
}

/// `hopper publish-idl` entry point.
pub fn cmd_publish_idl(args: &[String]) {
    let mut manifest_arg: Option<String> = None;
    let mut program_id_arg: Option<String> = None;
    let mut seed_arg: Option<String> = None;
    let mut url_arg: Option<String> = None;
    let mut keypair_arg: Option<String> = None;
    let mut overwrite = false;
    let mut dry_run = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--url" | "--rpc" => {
                url_arg = args.get(i + 1).cloned();
                if url_arg.is_none() {
                    eprintln!("--url requires an RPC endpoint argument");
                    process::exit(1);
                }
                i += 2;
            }
            "--keypair" | "--signer" => {
                keypair_arg = args.get(i + 1).cloned();
                if keypair_arg.is_none() {
                    eprintln!("--keypair requires a path argument");
                    process::exit(1);
                }
                i += 2;
            }
            "--overwrite" => {
                overwrite = true;
                i += 1;
            }
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

    // Non-dry-run: build, sign, and submit the real on-chain transaction(s).
    let payload = PreparedPayload::from_idl_json(&idl_json);
    if let Err(e) = run_publish_send(
        &program_id,
        &seed16,
        &payload,
        url_arg.as_deref(),
        keypair_arg.as_deref(),
        overwrite,
        manifest.name,
        manifest.version,
    ) {
        eprintln!("publish-idl: {e}");
        process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Signed on-chain send
// ---------------------------------------------------------------------------

/// Devnet default RPC endpoint for `publish-idl`.
const PUBLISH_DEFAULT_RPC: &str = "https://api.devnet.solana.com";

/// Resolve the RPC endpoint: explicit `--url` wins, then `SOLANA_RPC_URL`,
/// then devnet (publishing an IDL is most often a devnet operation, and we do
/// not want to silently target mainnet).
fn resolve_publish_url(cli: Option<&str>) -> String {
    if let Some(u) = cli {
        return u.to_string();
    }
    if let Ok(u) = std::env::var("SOLANA_RPC_URL") {
        if !u.is_empty() {
            return u;
        }
    }
    PUBLISH_DEFAULT_RPC.to_string()
}

/// Resolve the signer keypair path: explicit `--keypair` wins, else the
/// solana-cli default `~/.config/solana/id.json`.
fn resolve_keypair_path(cli: Option<&str>) -> Result<std::path::PathBuf, String> {
    if let Some(p) = cli {
        return Ok(std::path::PathBuf::from(p));
    }
    crate::workspace::default_solana_keypair_path().ok_or_else(|| {
        "no --keypair supplied and no default keypair at ~/.config/solana/id.json".to_string()
    })
}

/// Build, sign and submit the transaction(s) that write the IDL to the
/// canonical metadata PDA. Chooses the inline path for small IDLs and the
/// `Allocate` + chunked `Write` + `Initialize` path for large ones, and the
/// `SetData` rewrite path under `--overwrite`.
#[allow(clippy::too_many_arguments)]
fn run_publish_send(
    program_id: &[u8; 32],
    seed16: &[u8; SEED_LEN],
    payload: &PreparedPayload,
    url_cli: Option<&str>,
    keypair_cli: Option<&str>,
    overwrite: bool,
    manifest_name: &str,
    manifest_version: &str,
) -> Result<(), String> {
    use solana_client::rpc_client::RpcClient;
    use solana_commitment_config::CommitmentConfig;
    use solana_instruction::{AccountMeta, Instruction};
    use solana_keypair::read_keypair_file;
    use solana_pubkey::Pubkey;
    use solana_signer::Signer;
    use solana_system_interface::instruction as system_instruction;
    use solana_transaction::Transaction;

    let url = resolve_publish_url(url_cli);
    let keypair_path = resolve_keypair_path(keypair_cli)?;
    let authority = read_keypair_file(&keypair_path)
        .map_err(|e| format!("read keypair {}: {e}", keypair_path.display()))?;

    // Derive the canonical metadata PDA and the program's ProgramData PDA.
    let (pda, _bump) = derive_canonical_pda(program_id, seed16);
    let (program_data, _pd_bump) = derive_program_data_pda(program_id);
    let pda_b58 = bs58::encode(pda).into_string();
    let program_b58 = bs58::encode(program_id).into_string();

    let metadata_program = Pubkey::new_from_array(metadata_program_id());
    let system_program = Pubkey::new_from_array([0u8; 32]);
    let program_pk = Pubkey::new_from_array(*program_id);
    let program_data_pk = Pubkey::new_from_array(program_data);
    let pda_pk = Pubkey::new_from_array(pda);
    let authority_pk = authority.pubkey();

    let stored = payload.compressed.len();
    let final_space = HEADER_LEN + stored;

    println!("=== hopper publish-idl ===");
    println!("rpc              : {url}");
    println!("signer/authority : {authority_pk}");
    println!("program          : {program_b58}");
    println!("metadata PDA     : {pda_b58}");
    println!(
        "payload          : {} raw -> {} zlib bytes",
        payload.raw_len, stored
    );
    println!("source           : manifest {manifest_name} v{manifest_version}");

    // Inspect the current state of the PDA (existence / discriminator / owner).
    let existing = crate::rpc::get_account_info(&url, &pda_b58)
        .map_err(|e| format!("getAccountInfo({pda_b58}): {e}"))?;
    let existing_disc = existing.as_ref().and_then(|a| a.data.first().copied());
    let owned_by_metadata = existing
        .as_ref()
        .map(|a| a.owner == METADATA_PROGRAM_ID_B58)
        .unwrap_or(false);
    let existing_lamports = existing.as_ref().map(|a| a.lamports).unwrap_or(0);

    // Guard against clobbering an unrelated account that happens to sit at the
    // PDA (should be impossible for a real PDA, but fail loud rather than send).
    if let Some(acc) = existing.as_ref() {
        if !acc.data.is_empty() && !owned_by_metadata {
            return Err(format!(
                "PDA {pda_b58} exists but is owned by {} (not the metadata program); refusing to write",
                acc.owner
            ));
        }
    }

    let is_initialized = owned_by_metadata && existing_disc == Some(DISC_METADATA);
    let is_leftover_buffer = owned_by_metadata && existing_disc == Some(1 /* Buffer */);

    if is_initialized && !overwrite {
        return Err(format!(
            "metadata account {pda_b58} is already initialized; re-run with --overwrite to rewrite it"
        ));
    }

    let rpc = RpcClient::new_with_commitment(url.clone(), CommitmentConfig::confirmed());

    // Rent the account must hold at its final size, and how much to top up.
    let rent = rpc
        .get_minimum_balance_for_rent_exemption(final_space)
        .map_err(|e| format!("get_minimum_balance_for_rent_exemption({final_space}): {e}"))?;
    let topup = rent.saturating_sub(existing_lamports);

    // Fail early on an under-funded payer rather than mid-sequence.
    //
    // Reserve a base fee for EVERY transaction the chosen path submits —
    // not just one. The buffered path sends Allocate + N Write +
    // Initialize (N+2 txs); reserving a single fee would let a payer pass
    // this guard and then run dry partway through the Write loop, leaving
    // a partially-written Buffer stranded on chain. Over-reserving by a
    // fee on the resume/inline paths is the safe direction.
    const BASE_FEE: u64 = 5_000;
    let planned_txs: u64 = if is_initialized {
        1 // overwrite: single inline SetData (a large payload already errored)
    } else if stored <= MAX_INLINE_PAYLOAD && !is_leftover_buffer {
        1 // fresh inline: single Initialize
    } else {
        // fresh buffered / resume: [Allocate or top-up] + N Write + Initialize
        plan_write_chunks(&payload.compressed, WRITE_CHUNK_LEN).len() as u64 + 2
    };
    let required = topup.saturating_add(planned_txs.saturating_mul(BASE_FEE));

    let payer_balance = rpc
        .get_balance(&authority_pk)
        .map_err(|e| format!("get_balance({authority_pk}): {e}"))?;
    if payer_balance < required {
        return Err(format!(
            "insufficient balance: signer {authority_pk} has {payer_balance} lamports but needs \
             ~{required} (rent top-up {topup} + {planned_txs} tx fee(s)). Fund it (devnet: \
             `solana airdrop 1 {authority_pk} --url {url}`)"
        ));
    }

    // Account-meta templates shared across instructions.
    let init_metas = || {
        vec![
            AccountMeta::new(pda_pk, false),                   // 0 metadata (w)
            AccountMeta::new(authority_pk, true),              // 1 authority (s, w)
            AccountMeta::new_readonly(program_pk, false),      // 2 program (r)
            AccountMeta::new_readonly(program_data_pk, false), // 3 program_data (r)
            AccountMeta::new_readonly(system_program, false),  // 4 system (r)
        ]
    };

    let send = |instructions: &[Instruction], label: &str| -> Result<String, String> {
        let recent = rpc
            .get_latest_blockhash()
            .map_err(|e| format!("get_latest_blockhash ({label}): {e}"))?;
        let tx = Transaction::new_signed_with_payer(
            instructions,
            Some(&authority_pk),
            &[&authority],
            recent,
        );
        rpc.send_and_confirm_transaction(&tx)
            .map(|s| s.to_string())
            .map_err(|e| format!("{label}: {e}"))
    };

    // ---- Overwrite path: SetData on an already-initialized account. ----
    if is_initialized {
        if stored > MAX_INLINE_PAYLOAD {
            return Err(format!(
                "--overwrite of a {stored}-byte payload needs chunking, which the in-place \
                 SetData path does not support. Close the metadata account first, then \
                 re-publish fresh (the fresh large-publish path handles chunking)."
            ));
        }
        let mut ixs = Vec::new();
        if topup > 0 {
            ixs.push(system_instruction::transfer(&authority_pk, &pda_pk, topup));
        }
        let data = build_set_data_inline_data(
            payload.encoding,
            payload.compression,
            payload.format,
            DATA_SOURCE_DIRECT,
            &payload.compressed,
        );
        let metas = vec![
            AccountMeta::new(pda_pk, false),                    // 0 metadata (w)
            AccountMeta::new(authority_pk, true),               // 1 authority (s)
            AccountMeta::new_readonly(metadata_program, false), // 2 buffer placeholder = None
            AccountMeta::new_readonly(program_pk, false),       // 3 program (r)
            AccountMeta::new_readonly(program_data_pk, false),  // 4 program_data (r)
        ];
        ixs.push(Instruction {
            program_id: metadata_program,
            accounts: metas,
            data,
        });
        println!("path             : overwrite (inline SetData)");
        let sig = send(&ixs, "SetData")?;
        println!("signature        : {sig}");
        println!("status           : confirmed (metadata rewritten)");
        return Ok(());
    }

    // ---- Fresh inline path: single Initialize carries the whole payload. ----
    if stored <= MAX_INLINE_PAYLOAD && !is_leftover_buffer {
        let mut ixs = Vec::new();
        if topup > 0 {
            ixs.push(system_instruction::transfer(&authority_pk, &pda_pk, topup));
        }
        let data = build_initialize_inline_data(
            seed16,
            payload.encoding,
            payload.compression,
            payload.format,
            DATA_SOURCE_DIRECT,
            &payload.compressed,
        );
        ixs.push(Instruction {
            program_id: metadata_program,
            accounts: init_metas(),
            data,
        });
        println!("path             : fresh inline Initialize");
        let sig = send(&ixs, "Initialize")?;
        println!("signature        : {sig}");
        println!("status           : confirmed (IDL published)");
        return Ok(());
    }

    // ---- Fresh large path: Allocate + chunked Write + in-place Initialize. ----
    let chunks = plan_write_chunks(&payload.compressed, WRITE_CHUNK_LEN);
    println!(
        "path             : fresh buffered ({} write chunk(s))",
        chunks.len()
    );

    // Step 1: pre-fund to final size + Allocate the buffer header. If a
    // leftover buffer already exists at the PDA (a prior run died mid-write),
    // top up its lamports and skip Allocate — the Writes below overwrite by
    // offset, so resuming is safe and idempotent.
    if is_leftover_buffer {
        if topup > 0 {
            let sig = send(
                &[system_instruction::transfer(&authority_pk, &pda_pk, topup)],
                "top-up existing buffer",
            )?;
            println!("resume buffer    : topped up ({sig})");
        } else {
            println!("resume buffer    : reusing existing buffer at PDA");
        }
    } else {
        let mut ixs = Vec::new();
        if topup > 0 {
            ixs.push(system_instruction::transfer(&authority_pk, &pda_pk, topup));
        }
        ixs.push(Instruction {
            program_id: metadata_program,
            accounts: init_metas(),
            data: build_allocate_data(seed16),
        });
        let sig = send(&ixs, "Allocate")?;
        println!("allocate         : {sig}");
    }

    // Step 2: write each chunk. Write accounts = [buffer(w), authority(s),
    // source(r)] where source = the metadata program id signals "inline data".
    for (idx, (offset, chunk)) in chunks.iter().enumerate() {
        let metas = vec![
            AccountMeta::new(pda_pk, false),      // 0 target buffer (w)
            AccountMeta::new(authority_pk, true), // 1 authority (s)
            AccountMeta::new_readonly(metadata_program, false), // 2 source = None
        ];
        let ix = Instruction {
            program_id: metadata_program,
            accounts: metas,
            data: build_write_data(*offset, chunk),
        };
        let sig = send(&[ix], &format!("Write chunk {}/{}", idx + 1, chunks.len()))?;
        println!(
            "write [{:>2}/{:>2}]    : offset {:>6} len {:>4}  {sig}",
            idx + 1,
            chunks.len(),
            offset,
            chunk.len()
        );
    }

    // Step 3: finalize the buffer in place (no inline data allowed here).
    let data = build_initialize_from_buffer_data(
        seed16,
        payload.encoding,
        payload.compression,
        payload.format,
        DATA_SOURCE_DIRECT,
    );
    let ix = Instruction {
        program_id: metadata_program,
        accounts: init_metas(),
        data,
    };
    let sig = send(&[ix], "Initialize (finalize buffer)")?;
    println!("finalize         : {sig}");
    println!("status           : confirmed (IDL published)");
    Ok(())
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

    // -----------------------------------------------------------------
    // Instruction-encoder + chunking + parity tests
    // -----------------------------------------------------------------

    #[test]
    fn initialize_inline_data_is_disc_seed_tags_payload() {
        let seed = pad_seed(b"idl");
        let payload = [0xAA, 0xBB, 0xCC];
        let d = build_initialize_inline_data(
            &seed,
            ENCODING_UTF8,
            COMPRESSION_ZLIB,
            FORMAT_JSON,
            DATA_SOURCE_DIRECT,
            &payload,
        );
        // [1][seed:16][enc][comp][fmt][ds][payload]
        assert_eq!(d[0], INSTR_INITIALIZE);
        assert_eq!(&d[1..17], &seed);
        assert_eq!(d[17], ENCODING_UTF8);
        assert_eq!(d[18], COMPRESSION_ZLIB);
        assert_eq!(d[19], FORMAT_JSON);
        assert_eq!(d[20], DATA_SOURCE_DIRECT);
        assert_eq!(&d[21..], &payload);
        assert_eq!(d.len(), 1 + SEED_LEN + 4 + payload.len());
    }

    #[test]
    fn initialize_from_buffer_data_omits_payload() {
        let seed = pad_seed(b"idl");
        let d = build_initialize_from_buffer_data(
            &seed,
            ENCODING_UTF8,
            COMPRESSION_ZLIB,
            FORMAT_JSON,
            DATA_SOURCE_DIRECT,
        );
        assert_eq!(d[0], INSTR_INITIALIZE);
        assert_eq!(&d[1..17], &seed);
        assert_eq!(
            &d[17..21],
            &[
                ENCODING_UTF8,
                COMPRESSION_ZLIB,
                FORMAT_JSON,
                DATA_SOURCE_DIRECT
            ]
        );
        assert_eq!(
            d.len(),
            1 + SEED_LEN + 4,
            "no inline payload for the buffer path"
        );
    }

    #[test]
    fn allocate_data_is_disc_and_seed() {
        let seed = pad_seed(b"idl");
        let d = build_allocate_data(&seed);
        assert_eq!(d[0], INSTR_ALLOCATE);
        assert_eq!(&d[1..], &seed);
        assert_eq!(d.len(), 1 + SEED_LEN);
    }

    #[test]
    fn write_data_encodes_offset_le_then_chunk() {
        let chunk = [1u8, 2, 3, 4, 5];
        let d = build_write_data(0x0000_0900, &chunk);
        assert_eq!(d[0], INSTR_WRITE);
        // 0x900 = 2304 -> LE bytes 00 09 00 00
        assert_eq!(&d[1..5], &2304u32.to_le_bytes());
        assert_eq!(&d[5..], &chunk);
    }

    #[test]
    fn set_data_inline_is_disc_tags_payload() {
        let payload = [0xDE, 0xAD];
        let d = build_set_data_inline_data(
            ENCODING_UTF8,
            COMPRESSION_ZLIB,
            FORMAT_JSON,
            DATA_SOURCE_DIRECT,
            &payload,
        );
        assert_eq!(d[0], INSTR_SET_DATA);
        assert_eq!(
            &d[1..5],
            &[
                ENCODING_UTF8,
                COMPRESSION_ZLIB,
                FORMAT_JSON,
                DATA_SOURCE_DIRECT
            ]
        );
        assert_eq!(&d[5..], &payload);
    }

    #[test]
    fn chunking_boundaries_are_contiguous_and_cover_payload() {
        // 2050 bytes at 900/chunk -> offsets 0, 900, 1800 with lengths 900,900,250.
        let payload: Vec<u8> = (0..2050u32).map(|i| (i % 251) as u8).collect();
        let chunks = plan_write_chunks(&payload, WRITE_CHUNK_LEN);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].0, 0);
        assert_eq!(chunks[0].1.len(), 900);
        assert_eq!(chunks[1].0, 900);
        assert_eq!(chunks[1].1.len(), 900);
        assert_eq!(chunks[2].0, 1800);
        assert_eq!(chunks[2].1.len(), 250);
        // Every chunk is at most WRITE_CHUNK_LEN and offsets are contiguous.
        let mut expected_off = 0u32;
        for (off, ch) in &chunks {
            assert_eq!(*off, expected_off);
            assert!(ch.len() <= WRITE_CHUNK_LEN);
            expected_off += ch.len() as u32;
        }
        assert_eq!(expected_off as usize, payload.len());
    }

    #[test]
    fn chunks_reassemble_to_original_payload() {
        let payload: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 256) as u8).collect();
        let chunks = plan_write_chunks(&payload, WRITE_CHUNK_LEN);
        let rebuilt = reassemble_write_chunks(payload.len(), &chunks);
        assert_eq!(
            rebuilt, payload,
            "buffered writes must reproduce the payload byte-for-byte"
        );
    }

    #[test]
    fn program_data_pda_is_deterministic_and_off_curve() {
        let prog = sample_program();
        let a = derive_program_data_pda(&prog);
        let b = derive_program_data_pda(&prog);
        assert_eq!(a, b);
        assert!(!is_on_curve(&a.0));
        // Must differ from the metadata PDA (different program + seeds).
        let (meta_pda, _) = derive_canonical_pda(&prog, &pad_seed(DEFAULT_SEED));
        assert_ne!(a.0, meta_pda);
    }

    #[test]
    fn expected_account_image_is_header_then_payload() {
        let prog = sample_program();
        let seed = pad_seed(DEFAULT_SEED);
        let idl = r#"{"version":"0.1.0","name":"demo","instructions":[]}"#;
        let payload = PreparedPayload::from_idl_json(idl);
        let image = expected_account_image(&prog, &seed, &payload);
        // First 96 bytes are exactly the canonical header.
        let header = MetadataHeader::canonical_idl(prog, seed, &payload);
        assert_eq!(&image[..HEADER_LEN], &header.encode());
        // Remainder is the compressed payload.
        assert_eq!(&image[HEADER_LEN..], &payload.compressed[..]);
        // And the recorded data_len matches the stored region length.
        assert_eq!(image.len() - HEADER_LEN, header.data_len as usize);
    }

    #[test]
    fn dry_run_vs_send_parity_inline() {
        // For a small IDL the inline Initialize carries the payload verbatim,
        // and the resulting on-chain account image equals the dry-run header
        // preview followed by the same compressed bytes.
        let prog = sample_program();
        let seed = pad_seed(DEFAULT_SEED);
        let idl = r#"{"version":"0.1.0","name":"demo","instructions":[]}"#;
        let payload = PreparedPayload::from_idl_json(idl);
        assert!(payload.compressed.len() <= MAX_INLINE_PAYLOAD);

        // The bytes the Initialize instruction carries past its 21-byte prefix
        // are exactly the payload the dry-run reports storing.
        let init = build_initialize_inline_data(
            &seed,
            payload.encoding,
            payload.compression,
            payload.format,
            DATA_SOURCE_DIRECT,
            &payload.compressed,
        );
        let carried = &init[1 + SEED_LEN + 4..];
        assert_eq!(carried, &payload.compressed[..]);

        // The final account image the send produces == dry-run header ++ payload.
        let image = expected_account_image(&prog, &seed, &payload);
        let (pda, _) = derive_canonical_pda(&prog, &seed);
        let report = render_dry_run(&prog, &seed, idl, "demo", "0.1.0");
        assert!(report.contains(&hex_encode(&image[..HEADER_LEN])));
        assert!(report.contains(&bs58::encode(pda).into_string()));
    }

    #[test]
    fn dry_run_vs_send_parity_buffered() {
        // A large IDL: the Allocate + Write + Initialize path must store the
        // exact same account image as the inline path would, byte-for-byte.
        let prog = sample_program();
        let seed = pad_seed(DEFAULT_SEED);
        // Build an IDL whose *compressed* form exceeds the inline threshold by
        // stuffing high-entropy content that zlib cannot shrink much.
        let mut names = String::new();
        for i in 0..400 {
            names.push_str(&format!("\"ix_{i:04x}_{}\",", (i * 2654435761u64) & 0xffff));
        }
        let idl = format!(
            r#"{{"version":"0.1.0","name":"big","docs":[{}]}}"#,
            names.trim_end_matches(',')
        );
        let payload = PreparedPayload::from_idl_json(&idl);
        assert!(
            payload.compressed.len() > MAX_INLINE_PAYLOAD,
            "test fixture must exceed the inline threshold (got {})",
            payload.compressed.len()
        );

        // Reassemble the chunked writes and prepend the header the finalize
        // Initialize builds; it must equal the canonical account image.
        let chunks = plan_write_chunks(&payload.compressed, WRITE_CHUNK_LEN);
        assert!(chunks.len() >= 2, "fixture should need multiple chunks");
        let data_section = reassemble_write_chunks(payload.compressed.len(), &chunks);
        let mut buffered_image = MetadataHeader::canonical_idl(prog, seed, &payload)
            .encode()
            .to_vec();
        buffered_image.extend_from_slice(&data_section);

        let inline_image = expected_account_image(&prog, &seed, &payload);
        assert_eq!(
            buffered_image, inline_image,
            "buffered and inline paths must converge on identical account bytes"
        );

        // The dry-run for this payload should describe the buffered plan.
        let report = render_dry_run(&prog, &seed, &idl, "big", "0.1.0");
        assert!(report.contains("Allocate"));
        assert!(report.contains("Write"));
    }

    #[test]
    fn bpf_loader_id_round_trips() {
        let id = bpf_loader_upgradeable_id();
        assert_eq!(
            bs58::encode(id).into_string(),
            BPF_LOADER_UPGRADEABLE_ID_B58
        );
    }

    #[test]
    fn resolve_publish_url_prefers_cli_then_defaults_devnet() {
        assert_eq!(resolve_publish_url(Some("https://x")), "https://x");
        // With no CLI override and no env var set, defaults to devnet. (Do not
        // mutate process env here to avoid racing other tests.)
        if std::env::var("SOLANA_RPC_URL").is_err() {
            assert_eq!(resolve_publish_url(None), PUBLISH_DEFAULT_RPC);
        }
    }
}
