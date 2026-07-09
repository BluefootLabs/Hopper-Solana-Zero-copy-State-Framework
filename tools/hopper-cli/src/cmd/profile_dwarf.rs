//! DWARF inline-frame attribution for `hopper profile elf --inlines`.
//!
//! Why this exists: hopper-runtime is `#[inline(always)]` almost everywhere,
//! so a compiled program's entrypoint collapses into one giant `.text`
//! symbol and symbol-granularity size attribution says nothing actionable
//! about *which runtime feature* the bytes came from. DWARF remembers: every
//! `DW_TAG_inlined_subroutine` DIE carries the PC ranges its inlined body
//! occupies. This module walks that tree and builds an
//! address -> inline-frame-stack resolver, then attributes every `.text`
//! instruction's bytes to its deepest (leaf) inline frame. The ranked
//! leaf-frame table is the fix list: "of entrypoint's 10,464 bytes, N are
//! `receipt::commit`, M are `segment_borrow::register`, ...".
//!
//! Scope / supported DWARF (v4 and v5, 32- and 64-bit format):
//! - PC ranges: `DW_AT_low_pc` + `DW_AT_high_pc` (address or offset form)
//!   and `DW_AT_ranges` (`.debug_ranges` v4 / `.debug_rnglists` v5), all via
//!   `gimli::Dwarf::die_ranges`, including `DW_FORM_addrx` entries backed by
//!   `.debug_addr`.
//! - Names: `DW_AT_linkage_name` / `DW_AT_MIPS_linkage_name` (preferred:
//!   demangled they carry the full module path), else `DW_AT_name`, else the
//!   `DW_AT_abstract_origin` / `DW_AT_specification` chain (unit-local
//!   `DW_FORM_ref*` and cross-unit `DW_FORM_ref_addr`), recursion-capped.
//!   String forms may live inline, in `.debug_str`, or via
//!   `.debug_str_offsets`.
//! - Not supported: split DWARF (.dwo/.dwp), supplementary object files,
//!   and `.debug_types` type units (irrelevant to PC attribution). sBPF
//!   toolchains emit monolithic DWARF, so none of these appear in practice.
//!
//! Honesty rule: if the ELF has no usable DWARF we say exactly what is
//! missing and fall back to symbol-granularity attribution. Nothing is ever
//! fabricated.

use std::borrow::Cow;
use std::collections::BTreeMap;

/// Concrete reader type shared by the production loader and the test
/// fixtures: borrowed little/big-endian byte slices.
type Slice<'a> = gimli::EndianSlice<'a, gimli::RunTimeEndian>;

/// Placeholder frame for `.text` bytes covered by neither DWARF nor any
/// sized symbol (alignment padding, linker stubs).
pub(crate) const UNATTRIBUTED: &str = "[unattributed .text]";

/// SBF/eBPF `lddw` opcode: the only wide (16-byte, two-slot) instruction.
pub(crate) const SBF_OP_LDDW: u8 = 0x18;

// ---------------------------------------------------------------------------
// Inline index: address -> inline-frame stack
// ---------------------------------------------------------------------------

/// One PC range contributed by a `DW_TAG_subprogram` (depth 0) or a
/// `DW_TAG_inlined_subroutine` (depth >= 1) DIE.
struct PcRange {
    start: u64,
    /// Exclusive.
    end: u64,
    /// Frame-stack depth: 0 for the containing subprogram, +1 per level of
    /// inlining. "Deepest wins" picks the leaf frame at an address.
    depth: u32,
    /// Index into `InlineIndex::stacks`.
    stack: u32,
}

/// Address -> inline-frame-stack resolver built from one walk over
/// `.debug_info`.
pub(crate) struct InlineIndex {
    /// All PC ranges, sorted by `(start, end)`.
    ranges: Vec<PcRange>,
    /// `prefix_max_end[i] = max(ranges[..=i].end)`; bounds the backward scan
    /// in `resolve` so lookups stay cheap even with thousands of ranges.
    prefix_max_end: Vec<u64>,
    /// Interned frame stacks, root-first (outermost subprogram .. leaf).
    stacks: Vec<Vec<String>>,
    /// PC ranges contributed by `DW_TAG_inlined_subroutine` DIEs. Zero means
    /// the DWARF carries no inline data (e.g. line-tables-only builds).
    inline_range_count: usize,
    /// Compilation units walked.
    unit_count: usize,
}

impl InlineIndex {
    /// Resolve an address to its full inline-frame stack, root-first. The
    /// last element is the deepest (leaf) frame. `None` when no subprogram
    /// covers the address.
    pub(crate) fn resolve(&self, addr: u64) -> Option<&[String]> {
        // First candidate strictly after `addr`, then scan backwards. The
        // prefix-max-end array lets us stop as soon as nothing to the left
        // can still cover `addr`.
        let mut i = self.ranges.partition_point(|r| r.start <= addr);
        let mut best: Option<&PcRange> = None;
        while i > 0 {
            i -= 1;
            if self.prefix_max_end[i] <= addr {
                break;
            }
            let r = &self.ranges[i];
            if r.start <= addr && addr < r.end {
                let better = match best {
                    None => true,
                    // Deepest wins; on (malformed) equal-depth overlap the
                    // tighter range wins for determinism.
                    Some(b) => {
                        r.depth > b.depth
                            || (r.depth == b.depth && (r.end - r.start) < (b.end - b.start))
                    }
                };
                if better {
                    best = Some(r);
                }
            }
        }
        best.map(|r| self.stacks[r.stack as usize].as_slice())
    }

    pub(crate) fn inline_range_count(&self) -> usize {
        self.inline_range_count
    }

    pub(crate) fn unit_count(&self) -> usize {
        self.unit_count
    }
}

/// Outcome of looking for DWARF in an ELF.
pub(crate) enum DwarfLoad {
    /// `.debug_info` present; the index may still contain zero inline
    /// ranges (check `inline_range_count`).
    Index(InlineIndex),
    /// No (or empty) `.debug_info` section: the build carried no DWARF at
    /// all, or the artifact was stripped.
    Missing,
}

/// Load DWARF out of raw ELF bytes and build the inline index.
///
/// Returns `Ok(DwarfLoad::Missing)` when the file simply has no
/// `.debug_info`, and `Err` for real parse failures (corrupt DWARF,
/// compressed sections we cannot decompress, not an ELF).
pub(crate) fn load_inline_index(bytes: &[u8], demangle: bool) -> Result<DwarfLoad, String> {
    use object::{Object, ObjectSection};

    let file = object::File::parse(bytes).map_err(|e| format!("not a valid ELF: {e}"))?;
    let has_debug_info = file
        .section_by_name(gimli::SectionId::DebugInfo.name())
        .map(|s| s.size() > 0)
        .unwrap_or(false);
    if !has_debug_info {
        return Ok(DwarfLoad::Missing);
    }

    let endian = if file.is_little_endian() {
        gimli::RunTimeEndian::Little
    } else {
        gimli::RunTimeEndian::Big
    };

    // Pull every section gimli asks for (.debug_info, .debug_abbrev,
    // .debug_str, .debug_line, .debug_ranges, .debug_rnglists,
    // .debug_aranges, .debug_addr, .debug_str_offsets, ...). Missing
    // sections load as empty, which gimli treats as absent.
    let load_section = |id: gimli::SectionId| -> Result<Cow<'_, [u8]>, String> {
        match file.section_by_name(id.name()) {
            Some(section) => section
                .uncompressed_data()
                .map_err(|e| format!("could not read section `{}`: {e}", id.name())),
            None => Ok(Cow::Borrowed(&[][..])),
        }
    };
    let dwarf_sections = gimli::DwarfSections::load(load_section)?;
    let dwarf = dwarf_sections.borrow(|section| gimli::EndianSlice::new(section.as_ref(), endian));

    build_index_from_dwarf(&dwarf, demangle).map(DwarfLoad::Index)
}

/// Walk every compilation unit's DIE tree, collecting PC ranges for
/// `DW_TAG_subprogram` / `DW_TAG_inlined_subroutine` DIEs together with the
/// full frame stack active at each DIE. Shared by production (`.so` bytes)
/// and the in-test synthetic-DWARF fixtures.
fn build_index_from_dwarf(
    dwarf: &gimli::Dwarf<Slice>,
    demangle: bool,
) -> Result<InlineIndex, String> {
    // Pass 1: materialize all units so DW_FORM_ref_addr (cross-unit)
    // references in name chains can be chased.
    let mut units: Vec<gimli::Unit<Slice>> = Vec::new();
    let mut headers = dwarf.units();
    while let Some(header) = headers
        .next()
        .map_err(|e| format!("bad .debug_info unit header: {e}"))?
    {
        let unit = dwarf
            .unit(header)
            .map_err(|e| format!("bad compilation unit: {e}"))?;
        units.push(unit);
    }

    let mut ranges: Vec<PcRange> = Vec::new();
    let mut stacks: Vec<Vec<String>> = Vec::new();
    let mut stack_ids: BTreeMap<Vec<String>, u32> = BTreeMap::new();
    let mut inline_range_count = 0usize;

    for unit in &units {
        let mut cursor = unit.entries();
        let mut depth: isize = 0;
        // Active frames on the DIE path to the cursor: (DIE depth, name).
        let mut frames: Vec<(isize, String)> = Vec::new();
        while let Some((delta, entry)) = cursor
            .next_dfs()
            .map_err(|e| format!("bad DIE tree: {e}"))?
        {
            depth += delta;
            while frames.last().is_some_and(|&(d, _)| d >= depth) {
                frames.pop();
            }
            let tag = entry.tag();
            if tag != gimli::DW_TAG_subprogram && tag != gimli::DW_TAG_inlined_subroutine {
                continue;
            }

            let mut die_ranges: Vec<(u64, u64)> = Vec::new();
            let mut riter = dwarf
                .die_ranges(unit, entry)
                .map_err(|e| format!("bad PC ranges: {e}"))?;
            while let Some(r) = riter
                .next()
                .map_err(|e| format!("bad PC range entry: {e}"))?
            {
                if r.end > r.begin {
                    die_ranges.push((r.begin, r.end));
                }
            }

            let name = resolve_die_name(dwarf, &units, unit, entry, demangle, 0)
                .unwrap_or_else(|| "<anonymous>".to_string());
            // Push the frame even when this DIE has no ranges of its own
            // (abstract instances): descendants still need it in their
            // stacks.
            frames.push((depth, name));

            if !die_ranges.is_empty() {
                let stack: Vec<String> = frames.iter().map(|(_, n)| n.clone()).collect();
                let frame_depth = (stack.len() - 1) as u32;
                let id = *stack_ids.entry(stack.clone()).or_insert_with(|| {
                    stacks.push(stack);
                    (stacks.len() - 1) as u32
                });
                if tag == gimli::DW_TAG_inlined_subroutine {
                    inline_range_count += die_ranges.len();
                }
                for (start, end) in die_ranges {
                    ranges.push(PcRange {
                        start,
                        end,
                        depth: frame_depth,
                        stack: id,
                    });
                }
            }
        }
    }

    ranges.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    let mut prefix_max_end = Vec::with_capacity(ranges.len());
    let mut max_end = 0u64;
    for r in &ranges {
        max_end = max_end.max(r.end);
        prefix_max_end.push(max_end);
    }

    Ok(InlineIndex {
        ranges,
        prefix_max_end,
        stacks,
        inline_range_count,
        unit_count: units.len(),
    })
}

/// Resolve a subprogram/inlined-subroutine DIE to a display name.
///
/// Preference order: `DW_AT_linkage_name` (demangled it carries the full
/// `crate::module::fn` path - exactly what a fix list needs), then
/// `DW_AT_name`, then the `DW_AT_abstract_origin` / `DW_AT_specification`
/// reference chain. Recursion is capped so malformed reference cycles
/// cannot hang the walk.
fn resolve_die_name(
    dwarf: &gimli::Dwarf<Slice>,
    units: &[gimli::Unit<Slice>],
    unit: &gimli::Unit<Slice>,
    entry: &gimli::DebuggingInformationEntry<Slice>,
    demangle: bool,
    depth: u8,
) -> Option<String> {
    if depth > 8 {
        return None;
    }
    for at in [gimli::DW_AT_linkage_name, gimli::DW_AT_MIPS_linkage_name] {
        if let Ok(Some(value)) = entry.attr_value(at) {
            if let Ok(s) = dwarf.attr_string(unit, value) {
                let raw = String::from_utf8_lossy(s.slice()).into_owned();
                return Some(if demangle {
                    // `{:#}` strips the trailing `::h<hash>` disambiguator:
                    // the fix list reads `hopper_runtime::receipt::commit`,
                    // not `...::commit::hf3a9c2`.
                    format!("{:#}", rustc_demangle::demangle(&raw))
                } else {
                    raw
                });
            }
        }
    }
    if let Ok(Some(value)) = entry.attr_value(gimli::DW_AT_name) {
        if let Ok(s) = dwarf.attr_string(unit, value) {
            return Some(String::from_utf8_lossy(s.slice()).into_owned());
        }
    }
    for at in [gimli::DW_AT_abstract_origin, gimli::DW_AT_specification] {
        if let Ok(Some(value)) = entry.attr_value(at) {
            if let Some(name) = resolve_ref_name(dwarf, units, unit, value, demangle, depth + 1) {
                return Some(name);
            }
        }
    }
    None
}

/// Chase a DIE reference attribute (`DW_FORM_ref*` unit-local or
/// `DW_FORM_ref_addr` cross-unit) and resolve the referenced DIE's name.
fn resolve_ref_name(
    dwarf: &gimli::Dwarf<Slice>,
    units: &[gimli::Unit<Slice>],
    unit: &gimli::Unit<Slice>,
    value: gimli::AttributeValue<Slice>,
    demangle: bool,
    depth: u8,
) -> Option<String> {
    match value {
        gimli::AttributeValue::UnitRef(offset) => {
            let entry = unit.entry(offset).ok()?;
            resolve_die_name(dwarf, units, unit, &entry, demangle, depth)
        }
        gimli::AttributeValue::DebugInfoRef(offset) => {
            let target = units.iter().find(|u| match u.header.offset() {
                gimli::UnitSectionOffset::DebugInfoOffset(start) => {
                    offset.0 >= start.0 && offset.0 < start.0 + u.header.length_including_self()
                }
                _ => false,
            })?;
            let start = match target.header.offset() {
                gimli::UnitSectionOffset::DebugInfoOffset(start) => start.0,
                _ => return None,
            };
            let entry = target.entry(gimli::UnitOffset(offset.0 - start)).ok()?;
            resolve_die_name(dwarf, units, target, &entry, demangle, depth)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// ELF helpers: executable segments and symbol ranges (fallback attribution)
// ---------------------------------------------------------------------------

/// A sized `.text` symbol with its address range; the attribution fallback
/// when an address has no DWARF coverage.
pub(crate) struct SymbolRange {
    pub(crate) start: u64,
    pub(crate) end: u64,
    pub(crate) name: String,
}

/// Extract executable-section contents as `(vaddr, bytes)` pairs, sorted by
/// address. sBPF links a single `.text`, but we handle the general case.
pub(crate) fn text_segments(bytes: &[u8]) -> Result<Vec<(u64, Vec<u8>)>, String> {
    use object::{Object, ObjectSection};

    let file = object::File::parse(bytes).map_err(|e| format!("not a valid ELF: {e}"))?;
    let mut out = Vec::new();
    for section in file.sections() {
        let name = section.name().unwrap_or("?");
        let executable = matches!(section.kind(), object::SectionKind::Text)
            || name == ".text"
            || name.starts_with(".text.");
        if !executable || section.size() == 0 {
            continue;
        }
        let data = section
            .data()
            .map_err(|e| format!("could not read section `{name}`: {e}"))?;
        out.push((section.address(), data.to_vec()));
    }
    if out.is_empty() {
        return Err("no executable .text section found".into());
    }
    out.sort_by_key(|(addr, _)| *addr);
    Ok(out)
}

/// Sized `.text` symbols with address ranges, sorted by start address.
/// Names are demangled hash-free (`{:#}`) so they line up with DWARF frame
/// names in the `--inlines` outputs.
pub(crate) fn parse_symbol_ranges(
    bytes: &[u8],
    demangle: bool,
) -> Result<Vec<SymbolRange>, String> {
    use object::{Object, ObjectSymbol};

    let file = object::File::parse(bytes).map_err(|e| format!("not a valid ELF: {e}"))?;
    let mut out = Vec::new();
    for sym in file.symbols() {
        if sym.size() == 0 || !matches!(sym.kind(), object::SymbolKind::Text) {
            continue;
        }
        let raw = sym.name().unwrap_or("?");
        let name = if demangle {
            format!("{:#}", rustc_demangle::demangle(raw))
        } else {
            raw.to_string()
        };
        out.push(SymbolRange {
            start: sym.address(),
            end: sym.address() + sym.size(),
            name,
        });
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    Ok(out)
}

fn find_symbol(symbols: &[SymbolRange], addr: u64) -> Option<&SymbolRange> {
    let i = symbols.partition_point(|s| s.start <= addr);
    symbols[..i].iter().rev().find(|s| addr < s.end)
}

// ---------------------------------------------------------------------------
// SBF instruction walking (LDDW-aware)
// ---------------------------------------------------------------------------

/// Iterator over SBF instruction `(offset, byte_len)` pairs.
///
/// SBF/eBPF instructions occupy one 8-byte slot, except `lddw` (opcode
/// 0x18) which occupies two (16 bytes). Walking opcodes replaces the old
/// `bytes / 8` estimate, which over-counted every `lddw` as two
/// instructions (`bytes / 8` remains a valid *upper bound* and is still
/// used where only a byte size is known - see
/// `profile::estimate_sbf_instructions`).
///
/// Truncated trailers (a ragged tail shorter than 8 bytes, or an `lddw`
/// whose second slot is cut off) are yielded once with the remaining
/// length, so every input byte is attributed exactly once and malformed
/// sections cannot cause an overrun.
pub(crate) struct SbfInstructions<'a> {
    code: &'a [u8],
    off: usize,
}

impl<'a> SbfInstructions<'a> {
    pub(crate) fn new(code: &'a [u8]) -> Self {
        Self { code, off: 0 }
    }
}

impl<'a> Iterator for SbfInstructions<'a> {
    /// `(byte offset, instruction byte length)`
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        let remaining = self.code.len() - self.off;
        if remaining == 0 {
            return None;
        }
        let at = self.off;
        let len = if remaining < 8 {
            remaining
        } else if self.code[at] == SBF_OP_LDDW {
            remaining.min(16)
        } else {
            8
        };
        self.off += len;
        Some((at, len))
    }
}

/// Exact SBF instruction count for a code buffer: one per 8-byte slot,
/// except `lddw` which consumes two slots but counts as one instruction.
pub(crate) fn count_sbf_instructions(code: &[u8]) -> u64 {
    SbfInstructions::new(code).count() as u64
}

// ---------------------------------------------------------------------------
// Attribution
// ---------------------------------------------------------------------------

/// One row of the ranked leaf-frame table.
pub(crate) struct LeafRow {
    /// Deepest inline frame the bytes belong to (or the symbol itself in
    /// fallback mode).
    pub(crate) leaf: String,
    /// Top-level symbol/subprogram the frame was inlined into.
    pub(crate) root: String,
    pub(crate) bytes: u64,
}

/// Aggregated attribution over all executable bytes.
pub(crate) struct InlineAttribution {
    /// Ranked leaf-frame rows, descending by bytes.
    pub(crate) leaf_rows: Vec<LeafRow>,
    /// Folded full stacks (`root;mid;leaf` -> bytes), descending by bytes.
    /// Frame names are sanitized (`;` -> `:`) before joining.
    pub(crate) stacks: Vec<(String, u64)>,
    /// Total executable bytes walked.
    pub(crate) text_bytes: u64,
    /// Bytes attributed via DWARF frames (vs. symbol/unattributed fallback).
    pub(crate) dwarf_bytes: u64,
    /// LDDW-aware instruction count over the walked bytes.
    pub(crate) instructions: u64,
}

fn sanitize_frame(name: &str) -> String {
    // `;` is the folded-stack separator. Demangled Rust names never contain
    // it; this is defensive, mirroring `render_folded`.
    name.replace(';', ":")
}

fn fold_stack_key(stack: &[String]) -> String {
    let mut key = String::new();
    for (i, frame) in stack.iter().enumerate() {
        if i > 0 {
            key.push(';');
        }
        key.push_str(&sanitize_frame(frame));
    }
    key
}

/// Attribute every instruction's bytes to its deepest inline frame (full
/// stack retained for folded output). Addresses with no DWARF coverage
/// fall back to the containing sized symbol; addresses with neither go to
/// [`UNATTRIBUTED`]. An instruction is attributed whole at its start
/// address (an `lddw` straddling a frame boundary counts toward the frame
/// it starts in).
pub(crate) fn attribute_text(
    segments: &[(u64, Vec<u8>)],
    index: Option<&InlineIndex>,
    symbols: &[SymbolRange],
) -> InlineAttribution {
    let mut leaf_bytes: BTreeMap<(String, String), u64> = BTreeMap::new();
    let mut stack_bytes: BTreeMap<String, u64> = BTreeMap::new();
    let mut text_bytes = 0u64;
    let mut dwarf_bytes = 0u64;
    let mut instructions = 0u64;

    for (base, code) in segments {
        text_bytes += code.len() as u64;
        instructions += count_sbf_instructions(code);
        for (off, len) in SbfInstructions::new(code) {
            let addr = base + off as u64;
            let weight = len as u64;
            if let Some(stack) = index.and_then(|ix| ix.resolve(addr)) {
                dwarf_bytes += weight;
                let leaf = stack.last().expect("stacks are never empty").clone();
                let root = stack.first().expect("stacks are never empty").clone();
                *leaf_bytes.entry((leaf, root)).or_insert(0) += weight;
                *stack_bytes.entry(fold_stack_key(stack)).or_insert(0) += weight;
            } else if let Some(sym) = find_symbol(symbols, addr) {
                *leaf_bytes
                    .entry((sym.name.clone(), sym.name.clone()))
                    .or_insert(0) += weight;
                *stack_bytes.entry(sanitize_frame(&sym.name)).or_insert(0) += weight;
            } else {
                *leaf_bytes
                    .entry((UNATTRIBUTED.to_string(), UNATTRIBUTED.to_string()))
                    .or_insert(0) += weight;
                *stack_bytes.entry(UNATTRIBUTED.to_string()).or_insert(0) += weight;
            }
        }
    }

    let mut leaf_rows: Vec<LeafRow> = leaf_bytes
        .into_iter()
        .map(|((leaf, root), bytes)| LeafRow { leaf, root, bytes })
        .collect();
    leaf_rows.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.leaf.cmp(&b.leaf)));
    let mut stacks: Vec<(String, u64)> = stack_bytes.into_iter().collect();
    stacks.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    InlineAttribution {
        leaf_rows,
        stacks,
        text_bytes,
        dwarf_bytes,
        instructions,
    }
}

// ---------------------------------------------------------------------------
// Rendering and honest-fallback messaging
// ---------------------------------------------------------------------------

/// Render the ranked `bytes  %of-.text  leaf frame  (top-level symbol)`
/// table - the fix list. `(self)` marks rows whose leaf *is* the top-level
/// symbol (no inlining at those addresses, or symbol-fallback mode).
pub(crate) fn render_inline_table(rows: &[LeafRow], text_bytes: u64, top: usize) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "top {} leaf frames by attributed bytes:\n",
        top.min(rows.len())
    ));
    out.push_str(&format!(
        "{:>10}  {:>7}  leaf frame  (top-level symbol)\n",
        "bytes", "pct"
    ));
    let denom = text_bytes.max(1) as f64;
    for row in rows.iter().take(top) {
        let pct = row.bytes as f64 / denom * 100.0;
        let origin = if row.leaf == row.root {
            "(self)".to_string()
        } else {
            format!("({})", row.root)
        };
        out.push_str(&format!(
            "{:>10}  {:>6.2}%  {}  {}\n",
            row.bytes, pct, row.leaf, origin
        ));
    }
    out
}

/// Exact guidance when the ELF carries no DWARF at all. Printed instead of
/// fabricating inline attribution.
pub(crate) fn missing_dwarf_note(path: &str) -> String {
    format!(
        "--inlines: no DWARF debug info in `{path}`.\n\
         missing: .debug_info (and friends). Release sBPF builds default to `debug = 0`,\n\
         and `target/deploy/*.so` is stripped by the packager, so it never carries DWARF.\n\
         to get inline attribution:\n\
         \x20 1. rebuild with DWARF kept: add `[profile.release] debug = 2` to Cargo.toml,\n\
         \x20    or one-shot: `CARGO_PROFILE_RELEASE_DEBUG=2 cargo build-sbf`\n\
         \x20 2. re-run against the UNSTRIPPED artifact:\n\
         \x20    target/sbpf-solana-solana/release/deps/<name>.so  (not target/deploy/)\n\
         falling back to symbol-granularity attribution; inline frames are not available\n\
         and are never fabricated."
    )
}

/// Exact guidance when DWARF exists but contains no inlined-subroutine PC
/// ranges (typically line-tables-only debug info).
pub(crate) fn no_inline_ranges_note() -> String {
    "--inlines: DWARF is present but contains no inlined-subroutine PC ranges.\n\
     this usually means line-tables-only debug info (`debug = 1` or\n\
     `debug = \"line-tables-only\"`). Rebuild with full debug info:\n\
     `[profile.release] debug = 2` (or `CARGO_PROFILE_RELEASE_DEBUG=2`), then re-run\n\
     against target/sbpf-solana-solana/release/deps/<name>.so.\n\
     falling back to symbol-granularity attribution; nothing is fabricated."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- synthetic DWARF fixture ------------------------------------------
    //
    // Built in-test with `gimli::write` (dev-dependency feature) rather
    // than a checked-in fixture ELF: the fixture is readable and reviewable
    // as code, needs no external toolchain to regenerate, and exercises the
    // exact encodings gimli emits for DWARF v4 (low_pc + offset high_pc,
    // .debug_ranges lists, unit-local origin refs, inline strings).
    //
    // Layout (addresses are exclusive-end):
    //   outer                 [0x100, 0x200)            DW_TAG_subprogram
    //     fix::middle         [0x120, 0x160) u [0x180, 0x190)   inlined, via
    //                         DW_AT_ranges + DW_AT_abstract_origin
    //       fix::leaf         [0x130, 0x150)            inlined, origin ->
    //                         spec chain -> mangled linkage name
    fn synth_dwarf_sections() -> BTreeMap<gimli::SectionId, Vec<u8>> {
        use gimli::write::{
            Address, AttributeValue, DwarfUnit, EndianVec, Range, RangeList, Sections,
        };

        let encoding = gimli::Encoding {
            format: gimli::Format::Dwarf32,
            version: 4,
            address_size: 8,
        };
        let mut dwarf = DwarfUnit::new(encoding);
        let root = dwarf.unit.root();
        dwarf.unit.get_mut(root).set(
            gimli::DW_AT_name,
            AttributeValue::String(b"fixture.rs".to_vec()),
        );

        // Abstract instance for `middle`: mangled linkage name + short name.
        let abs_middle = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
        dwarf.unit.get_mut(abs_middle).set(
            gimli::DW_AT_linkage_name,
            AttributeValue::String(b"_ZN3fix6middle17h0123456789abcdefE".to_vec()),
        );
        dwarf.unit.get_mut(abs_middle).set(
            gimli::DW_AT_name,
            AttributeValue::String(b"middle".to_vec()),
        );
        dwarf.unit.get_mut(abs_middle).set(
            gimli::DW_AT_inline,
            AttributeValue::Inline(gimli::DW_INL_inlined),
        );

        // `leaf` gets its name through a DW_AT_specification hop, so the
        // abstract_origin -> specification chain is exercised.
        let spec_leaf = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
        dwarf.unit.get_mut(spec_leaf).set(
            gimli::DW_AT_linkage_name,
            AttributeValue::String(b"_ZN3fix4leaf17hfedcba9876543210E".to_vec()),
        );
        let abs_leaf = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
        dwarf.unit.get_mut(abs_leaf).set(
            gimli::DW_AT_specification,
            AttributeValue::UnitRef(spec_leaf),
        );
        dwarf.unit.get_mut(abs_leaf).set(
            gimli::DW_AT_inline,
            AttributeValue::Inline(gimli::DW_INL_inlined),
        );

        // Concrete `outer` [0x100, 0x200): low_pc + offset-form high_pc.
        let outer = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
        dwarf
            .unit
            .get_mut(outer)
            .set(gimli::DW_AT_name, AttributeValue::String(b"outer".to_vec()));
        dwarf.unit.get_mut(outer).set(
            gimli::DW_AT_low_pc,
            AttributeValue::Address(Address::Constant(0x100)),
        );
        dwarf
            .unit
            .get_mut(outer)
            .set(gimli::DW_AT_high_pc, AttributeValue::Udata(0x100));

        // `middle` inlined into `outer` over two discontiguous ranges.
        let inl_middle = dwarf.unit.add(outer, gimli::DW_TAG_inlined_subroutine);
        dwarf.unit.get_mut(inl_middle).set(
            gimli::DW_AT_abstract_origin,
            AttributeValue::UnitRef(abs_middle),
        );
        let range_list = dwarf.unit.ranges.add(RangeList(vec![
            Range::StartLength {
                begin: Address::Constant(0x120),
                length: 0x40,
            },
            Range::StartLength {
                begin: Address::Constant(0x180),
                length: 0x10,
            },
        ]));
        dwarf.unit.get_mut(inl_middle).set(
            gimli::DW_AT_ranges,
            AttributeValue::RangeListRef(range_list),
        );

        // `leaf` inlined inside `middle` [0x130, 0x150).
        let inl_leaf = dwarf.unit.add(inl_middle, gimli::DW_TAG_inlined_subroutine);
        dwarf.unit.get_mut(inl_leaf).set(
            gimli::DW_AT_abstract_origin,
            AttributeValue::UnitRef(abs_leaf),
        );
        dwarf.unit.get_mut(inl_leaf).set(
            gimli::DW_AT_low_pc,
            AttributeValue::Address(Address::Constant(0x130)),
        );
        dwarf
            .unit
            .get_mut(inl_leaf)
            .set(gimli::DW_AT_high_pc, AttributeValue::Udata(0x20));

        let mut sections = Sections::new(EndianVec::new(gimli::LittleEndian));
        dwarf.write(&mut sections).expect("write synthetic DWARF");
        let mut out = BTreeMap::new();
        sections
            .for_each(|id, data| {
                out.insert(id, data.slice().to_vec());
                Ok::<(), gimli::Error>(())
            })
            .expect("collect synthetic sections");
        out
    }

    fn index_from_synth(demangle: bool) -> InlineIndex {
        let sections = synth_dwarf_sections();
        let dwarf = gimli::Dwarf::load(|id| -> Result<Slice<'_>, gimli::Error> {
            Ok(gimli::EndianSlice::new(
                sections.get(&id).map(|v| v.as_slice()).unwrap_or(&[]),
                gimli::RunTimeEndian::Little,
            ))
        })
        .expect("load synthetic DWARF");
        build_index_from_dwarf(&dwarf, demangle).expect("build index")
    }

    #[test]
    fn dwarf_walk_resolves_nested_inline_stacks_deepest_wins() {
        let ix = index_from_synth(true);
        assert_eq!(ix.unit_count(), 1);
        // middle contributes 2 ranges + leaf contributes 1.
        assert_eq!(ix.inline_range_count(), 3);

        // Plain outer body: single-frame stack.
        assert_eq!(ix.resolve(0x110).unwrap(), ["outer"]);
        // Inside middle's first range but before leaf.
        assert_eq!(ix.resolve(0x125).unwrap(), ["outer", "fix::middle"]);
        // Deepest frame wins inside leaf.
        assert_eq!(
            ix.resolve(0x140).unwrap(),
            ["outer", "fix::middle", "fix::leaf"]
        );
        // Gap between middle's two ranges belongs to outer alone.
        assert_eq!(ix.resolve(0x170).unwrap(), ["outer"]);
        // middle's second (discontiguous, DW_AT_ranges) range.
        assert_eq!(ix.resolve(0x185).unwrap(), ["outer", "fix::middle"]);
    }

    #[test]
    fn dwarf_walk_range_edges_are_half_open() {
        let ix = index_from_synth(true);
        assert!(ix.resolve(0x0ff).is_none(), "before outer");
        assert_eq!(ix.resolve(0x100).unwrap(), ["outer"], "start inclusive");
        assert_eq!(
            ix.resolve(0x1ff).unwrap(),
            ["outer"],
            "last byte still inside"
        );
        assert!(ix.resolve(0x200).is_none(), "end exclusive");
        // leaf end 0x150 exclusive -> back to middle.
        assert_eq!(ix.resolve(0x14f).unwrap().len(), 3);
        assert_eq!(ix.resolve(0x150).unwrap(), ["outer", "fix::middle"]);
    }

    #[test]
    fn dwarf_walk_demangles_linkage_names_via_origin_and_spec_chains() {
        // `middle`: abstract_origin -> linkage name; `leaf`:
        // abstract_origin -> specification -> linkage name. Demangled
        // hash-free by default...
        let ix = index_from_synth(true);
        let stack = ix.resolve(0x140).unwrap();
        assert_eq!(stack[1], "fix::middle");
        assert_eq!(stack[2], "fix::leaf");

        // ...and left mangled with --no-demangle.
        let raw = index_from_synth(false);
        let stack = raw.resolve(0x140).unwrap();
        assert_eq!(stack[1], "_ZN3fix6middle17h0123456789abcdefE");
        assert_eq!(stack[2], "_ZN3fix4leaf17hfedcba9876543210E");
    }

    #[test]
    fn empty_index_resolves_nothing() {
        let ix = InlineIndex {
            ranges: Vec::new(),
            prefix_max_end: Vec::new(),
            stacks: Vec::new(),
            inline_range_count: 0,
            unit_count: 0,
        };
        assert!(ix.resolve(0x100).is_none());
    }

    // -- LDDW-aware instruction walking ------------------------------------

    #[test]
    fn lddw_counts_as_one_sixteen_byte_instruction() {
        // mov64 (0xb7), lddw (0x18, two slots), exit (0x95).
        let mut code = Vec::new();
        code.extend_from_slice(&[0xb7, 0, 0, 0, 0, 0, 0, 0]);
        code.extend_from_slice(&[0x18, 0, 0, 0, 0x2a, 0, 0, 0]);
        code.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]); // lddw second slot
        code.extend_from_slice(&[0x95, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(count_sbf_instructions(&code), 3);
        let steps: Vec<(usize, usize)> = SbfInstructions::new(&code).collect();
        assert_eq!(steps, vec![(0, 8), (8, 16), (24, 8)]);
        // The old estimate over-counts: 32 bytes / 8 = 4 "instructions".
        assert_eq!(code.len() as u64 / 8, 4);
    }

    #[test]
    fn instruction_walk_clamps_truncated_trailers() {
        // lddw as the final 8 bytes: second slot missing - one instruction,
        // no overrun.
        let code = [0x18, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            SbfInstructions::new(&code).collect::<Vec<_>>(),
            vec![(0, 8)]
        );
        // Ragged 12-byte buffer: 8-byte slot + 4-byte tail.
        let code = [0xb7, 0, 0, 0, 0, 0, 0, 0, 0x95, 0, 0, 0];
        assert_eq!(
            SbfInstructions::new(&code).collect::<Vec<_>>(),
            vec![(0, 8), (8, 4)]
        );
        assert_eq!(count_sbf_instructions(&[]), 0);
    }

    // -- attribution --------------------------------------------------------

    #[test]
    fn attribution_splits_bytes_by_deepest_frame() {
        let ix = index_from_synth(true);
        // 256 zero bytes at 0x100: 32 plain 8-byte instructions covering
        // outer exactly.
        let segments = vec![(0x100u64, vec![0u8; 0x100])];
        let attr = attribute_text(&segments, Some(&ix), &[]);

        assert_eq!(attr.text_bytes, 0x100);
        assert_eq!(attr.dwarf_bytes, 0x100, "everything covered by DWARF");
        assert_eq!(attr.instructions, 32);

        let by_leaf: BTreeMap<(String, String), u64> = attr
            .leaf_rows
            .iter()
            .map(|r| ((r.leaf.clone(), r.root.clone()), r.bytes))
            .collect();
        // outer: [0x100,0x120) + [0x160,0x180) + [0x190,0x200) = 176.
        assert_eq!(by_leaf[&("outer".into(), "outer".into())], 176);
        // middle: [0x120,0x130) + [0x150,0x160) + [0x180,0x190) = 48.
        assert_eq!(by_leaf[&("fix::middle".into(), "outer".into())], 48);
        // leaf: [0x130,0x150) = 32.
        assert_eq!(by_leaf[&("fix::leaf".into(), "outer".into())], 32);

        let stacks: BTreeMap<String, u64> = attr.stacks.iter().cloned().collect();
        assert_eq!(stacks["outer"], 176);
        assert_eq!(stacks["outer;fix::middle"], 48);
        assert_eq!(stacks["outer;fix::middle;fix::leaf"], 32);
    }

    #[test]
    fn attribution_charges_boundary_straddling_lddw_to_its_start_frame() {
        let ix = index_from_synth(true);
        // lddw at 0x118 spans [0x118, 0x128): starts in outer, ends inside
        // middle's range. The whole 16 bytes go to outer (start address
        // rule).
        let mut code = vec![0u8; 0x100];
        code[0x18] = SBF_OP_LDDW;
        let attr = attribute_text(&[(0x100, code)], Some(&ix), &[]);
        let by_leaf: BTreeMap<(String, String), u64> = attr
            .leaf_rows
            .iter()
            .map(|r| ((r.leaf.clone(), r.root.clone()), r.bytes))
            .collect();
        assert_eq!(by_leaf[&("outer".into(), "outer".into())], 184);
        assert_eq!(by_leaf[&("fix::middle".into(), "outer".into())], 40);
        assert_eq!(by_leaf[&("fix::leaf".into(), "outer".into())], 32);
        assert_eq!(attr.instructions, 31, "one lddw fuses two slots");
    }

    #[test]
    fn attribution_falls_back_to_symbols_then_unattributed() {
        let symbols = vec![
            SymbolRange {
                start: 0x100,
                end: 0x180,
                name: "alpha".into(),
            },
            SymbolRange {
                start: 0x180,
                end: 0x1c0,
                name: "beta".into(),
            },
        ];
        // No DWARF index at all: symbol granularity, honest leaf == root.
        let attr = attribute_text(&[(0x100, vec![0u8; 0x100])], None, &symbols);
        assert_eq!(attr.dwarf_bytes, 0);
        let by_leaf: BTreeMap<(String, String), u64> = attr
            .leaf_rows
            .iter()
            .map(|r| ((r.leaf.clone(), r.root.clone()), r.bytes))
            .collect();
        assert_eq!(by_leaf[&("alpha".into(), "alpha".into())], 128);
        assert_eq!(by_leaf[&("beta".into(), "beta".into())], 64);
        // [0x1c0, 0x200) is covered by no symbol.
        assert_eq!(
            by_leaf[&(UNATTRIBUTED.to_string(), UNATTRIBUTED.to_string())],
            64
        );
    }

    #[test]
    fn fold_stack_keys_sanitize_frame_separators() {
        let stack = vec!["outer".to_string(), "weird;name".to_string()];
        assert_eq!(fold_stack_key(&stack), "outer;weird:name");
    }

    // -- rendering & honest fallback ----------------------------------------

    #[test]
    fn inline_table_ranks_and_marks_self_rows() {
        let rows = vec![
            LeafRow {
                leaf: "hopper_runtime::receipt::commit".into(),
                root: "entrypoint".into(),
                bytes: 4096,
            },
            LeafRow {
                leaf: "entrypoint".into(),
                root: "entrypoint".into(),
                bytes: 1024,
            },
            LeafRow {
                leaf: "tiny".into(),
                root: "entrypoint".into(),
                bytes: 8,
            },
        ];
        let table = render_inline_table(&rows, 8192, 2);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines[0], "top 2 leaf frames by attributed bytes:");
        assert!(lines[1].contains("bytes") && lines[1].contains("leaf frame"));
        assert!(
            lines[2].contains("4096")
                && lines[2].contains("50.00%")
                && lines[2].contains("hopper_runtime::receipt::commit")
                && lines[2].contains("(entrypoint)")
        );
        assert!(lines[3].contains("1024") && lines[3].contains("(self)"));
        // --top honored: the 8-byte row is truncated away.
        assert_eq!(lines.len(), 4);
        assert!(!table.contains("tiny"));
    }

    #[test]
    fn missing_dwarf_note_names_the_fix() {
        let note = missing_dwarf_note("target/deploy/hopper_vault.so");
        assert!(note.contains("target/deploy/hopper_vault.so"));
        assert!(note.contains(".debug_info"));
        assert!(note.contains("[profile.release] debug = 2"));
        assert!(note.contains("CARGO_PROFILE_RELEASE_DEBUG=2"));
        assert!(note.contains("target/sbpf-solana-solana/release/deps/"));
        assert!(note.contains("symbol-granularity"));
        assert!(note.contains("never fabricated"));
    }

    #[test]
    fn no_inline_ranges_note_names_the_fix() {
        let note = no_inline_ranges_note();
        assert!(note.contains("no inlined-subroutine PC ranges"));
        assert!(note.contains("debug = 2"));
        assert!(note.contains("CARGO_PROFILE_RELEASE_DEBUG=2"));
        assert!(note.contains("symbol-granularity"));
    }

    #[test]
    fn load_inline_index_reports_missing_dwarf_for_dwarfless_elf() {
        // Minimal valid 64-bit little-endian BPF ELF with zero sections:
        // just the 64-byte header. object parses it; there is no
        // .debug_info, so the loader must report Missing (never Err, never
        // a fabricated index).
        let mut elf = vec![0u8; 64];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2; // ELFCLASS64
        elf[5] = 1; // little-endian
        elf[6] = 1; // EV_CURRENT
        elf[16] = 3; // e_type = ET_DYN
        elf[18] = 247; // e_machine = EM_BPF
        elf[20] = 1; // e_version
        elf[52] = 64; // e_ehsize
        match load_inline_index(&elf, true) {
            Ok(DwarfLoad::Missing) => {}
            Ok(DwarfLoad::Index(_)) => panic!("fabricated an index with no DWARF present"),
            Err(err) => panic!("expected Missing, got error: {err}"),
        }
    }
}
