//! The runtime lab under Mollusk: the allocator against the VM's real heap
//! (with and without a requested frame), the SlotHashes lookup against a
//! sysvar with skipped slots, and the panic handler in both builds.
//!
//! Build first, from `examples/hopper-runtime-lab`:
//!
//! ```text
//! cargo build-sbf
//! cargo build-sbf --features report-panics --sbf-out-dir ../../target/deploy-report-panics
//! ```
//!
//! A test whose artifact is missing prints `SKIPPED` and passes.

use hopper_test::LiteSvmHarness;
use solana_account::Account;
use solana_hash::Hash;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_slot_hashes::SlotHashes;

const DEFAULT_ELF: &str = "../../target/deploy/hopper_runtime_lab";
const REPORTING_ELF: &str = "../../target/deploy-report-panics/hopper_runtime_lab";

const SCRATCH: u64 = 8 + 20 * 1024;
const KIB: u64 = 1024;

struct Lab {
    svm: LiteSvmHarness,
    program_id: Pubkey,
    caller: Pubkey,
}

struct Outcome {
    ok: bool,
    error: String,
    return_data: Vec<u8>,
    units: u64,
    logs: Vec<String>,
}

impl Lab {
    fn load(stem: &str) -> Option<Self> {
        let program_id = Pubkey::new_unique();
        let mut svm = LiteSvmHarness::load(&program_id, stem).or_else(|| {
            eprintln!("SKIPPED: build {stem}.so first");
            None
        })?;
        svm.capture_logs();
        Some(Self {
            svm,
            program_id,
            caller: Pubkey::new_unique(),
        })
    }

    fn heap(&mut self, bytes: u32) {
        self.svm.mollusk_mut().compute_budget.heap_size = bytes;
    }

    fn send(&mut self, data: Vec<u8>) -> Outcome {
        self.svm.capture_logs();
        let result = self.svm.process(
            &Instruction::new_with_bytes(
                self.program_id,
                &data,
                vec![AccountMeta::new_readonly(self.caller, true)],
            ),
            &[(
                self.caller,
                Account::new(1_000_000_000, 0, &Pubkey::default()),
            )],
        );
        Outcome {
            ok: result.succeeded(),
            error: format!("{:?}", result.raw().program_result),
            return_data: result.raw().return_data.clone(),
            units: result.compute_units(),
            logs: self.svm.logs(),
        }
    }
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn allocate(bytes: u32) -> Vec<u8> {
    let mut data = vec![0u8];
    data.extend_from_slice(&bytes.to_le_bytes());
    data
}

#[test]
fn the_default_heap_holds_what_is_above_the_scratch() {
    let Some(mut lab) = Lab::load(DEFAULT_ELF) else {
        return;
    };
    // 32 KiB of heap, 20 KiB of it Hopper's scratch: 12 KiB less the
    // cursor word.
    let room = 32 * KIB - SCRATCH;
    let outcome = lab.send(allocate(room as u32));
    assert!(outcome.ok, "{}: {:?}", outcome.error, outcome.logs);
    assert_eq!(u64_at(&outcome.return_data, 0), room);

    // One byte more does not fit the memory the VM mapped. The allocator
    // was told the heap is 256 KiB, so it hands the block out, and the VM
    // stops the write that leaves the 32 KiB it granted.
    let outcome = lab.send(allocate(room as u32 + 1));
    assert!(!outcome.ok, "an allocation past the granted heap must fail");
}

#[test]
fn a_requested_heap_frame_is_usable() {
    let Some(mut lab) = Lab::load(DEFAULT_ELF) else {
        return;
    };
    lab.heap(256 * 1024);
    for bytes in [16 * KIB, 64 * KIB, 200 * KIB, 256 * KIB - SCRATCH] {
        let outcome = lab.send(allocate(bytes as u32));
        assert!(
            outcome.ok,
            "{bytes} bytes: {}: {:?}",
            outcome.error, outcome.logs
        );
        assert_eq!(u64_at(&outcome.return_data, 0), bytes);
        println!("RUNTIME_LAB allocate {bytes} bytes: {} CU", outcome.units);
    }
    // Past the declared heap the allocator itself refuses.
    let outcome = lab.send(allocate((256 * KIB - SCRATCH + 1) as u32));
    assert!(!outcome.ok);
    assert!(outcome.error.contains("6800"), "{}", outcome.error);

    // A frame between the two: usable to its own end and no further.
    lab.heap(64 * 1024);
    assert!(lab.send(allocate((64 * KIB - SCRATCH) as u32)).ok);
    assert!(!lab.send(allocate((64 * KIB - SCRATCH + 1) as u32)).ok);
}

#[test]
fn a_vector_that_grows_costs_its_final_size() {
    let Some(mut lab) = Lab::load(DEFAULT_ELF) else {
        return;
    };
    lab.heap(256 * 1024);
    for kib in [1u16, 8, 64, 200] {
        let mut data = vec![1u8];
        data.extend_from_slice(&kib.to_le_bytes());
        let outcome = lab.send(data);
        assert!(
            outcome.ok,
            "{kib} KiB: {}: {:?}",
            outcome.error, outcome.logs
        );
        let (used, capacity) = (
            u64_at(&outcome.return_data, 0),
            u64_at(&outcome.return_data, 8),
        );
        assert_eq!(capacity, kib as u64 * KIB);
        // Grown in place every time: the heap holds the vector once. An
        // allocator that moved it on each growth would have used the sum
        // 1 + 2 + ... + kib KiB.
        assert_eq!(used, capacity, "{kib} KiB");
        println!("RUNTIME_LAB grow to {kib} KiB: {} CU", outcome.units);
    }
}

#[test]
fn a_checkpoint_lets_a_loop_reuse_the_heap() {
    let Some(mut lab) = Lab::load(DEFAULT_ELF) else {
        return;
    };
    // 8 KiB a round for 100 rounds is 800 KiB through a 12 KiB heap.
    let mut data = vec![2u8];
    data.extend_from_slice(&(8 * 1024u32).to_le_bytes());
    data.extend_from_slice(&100u16.to_le_bytes());
    let outcome = lab.send(data);
    assert!(outcome.ok, "{}: {:?}", outcome.error, outcome.logs);
    assert_eq!(
        u64_at(&outcome.return_data, 0),
        0,
        "nothing is held at the end"
    );
    assert_eq!(
        u64_at(&outcome.return_data, 8),
        8 * KIB,
        "the peak is one round"
    );
    println!(
        "RUNTIME_LAB checkpoint loop, 100 rounds of 8 KiB: {} CU",
        outcome.units
    );
}

fn hash_of(slot: u64) -> Hash {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&slot.to_le_bytes());
    bytes[8..16].copy_from_slice(&(!slot).to_le_bytes());
    Hash::new_from_array(bytes)
}

/// 512 entries down from `newest`, skipping the slots `skip` names.
fn slot_hashes(newest: u64, skip: impl Fn(u64) -> bool) -> (SlotHashes, Vec<u64>) {
    let mut slots = Vec::new();
    let mut slot = newest;
    while slots.len() < 512 {
        if slot == newest || !skip(slot) {
            slots.push(slot);
        }
        slot -= 1;
    }
    let entries: Vec<(u64, Hash)> = slots.iter().map(|s| (*s, hash_of(*s))).collect();
    (SlotHashes::new(&entries), slots)
}

#[test]
fn slot_hashes_are_found_with_the_reads_the_design_promises() {
    let Some(mut lab) = Lab::load(DEFAULT_ELF) else {
        return;
    };
    let newest = 300_000_000u64;
    let (sysvar, slots) = slot_hashes(newest, |slot| slot % 23 == 5);
    lab.svm.mollusk_mut().sysvars.slot_hashes = sysvar;
    let oldest = *slots.last().unwrap();

    let mut lookup = |tag: u8, value: u64| {
        let mut data = vec![tag];
        data.extend_from_slice(&value.to_le_bytes());
        let outcome = lab.send(data);
        assert!(outcome.ok, "{}: {:?}", outcome.error, outcome.logs);
        let data = outcome.return_data;
        assert_eq!(u64_at(&data, 0), newest);
        (
            u64_at(&data, 8),
            data[16],
            data[17],
            data[18..50].to_vec(),
            outcome.units,
        )
    };

    // A recent slot: one read for the lookup.
    let (target, status, reads, hash, units) = lookup(4, 3);
    assert_eq!((target, status, reads), (newest - 3, 0, 1));
    assert_eq!(hash, hash_of(newest - 3).to_bytes());
    println!("RUNTIME_LAB slot hash 3 back: {units} CU for the instruction");

    // Older slots: two reads while few slots were skipped in between.
    for back in [20u64, 100, 250] {
        let slot = newest - back;
        let (target, status, reads, hash, units) = lookup(4, back);
        assert_eq!(target, slot);
        if slots.contains(&slot) {
            assert_eq!(status, 0, "{back} back");
            assert_eq!(hash, hash_of(slot).to_bytes());
        } else {
            assert_eq!(status, 1, "{back} back was skipped");
        }
        assert!(reads <= 3, "{back} back took {reads} reads");
        println!(
            "RUNTIME_LAB slot hash {back} back: {reads} reads, {units} CU for the instruction"
        );
    }

    // Every reason is told apart.
    let skipped = (oldest..newest).rev().find(|s| s % 23 == 5).unwrap();
    assert_eq!(lookup(5, skipped).1, 1, "skipped");
    assert_eq!(lookup(5, oldest - 1).1, 2, "too old");
    assert_eq!(lookup(5, newest + 1).1, 3, "ahead");
    let (_, status, _, hash, _) = lookup(5, oldest);
    assert_eq!(status, 0);
    assert_eq!(hash, hash_of(oldest).to_bytes());

    // Every slot the sysvar covers, against the list itself.
    let mut worst = 0;
    for slot in oldest..=newest {
        let (_, status, reads, hash, _) = lookup(5, slot);
        if slots.contains(&slot) {
            assert_eq!(status, 0, "slot {slot}");
            assert_eq!(hash, hash_of(slot).to_bytes(), "slot {slot}");
        } else {
            assert_eq!(status, 1, "slot {slot}");
        }
        worst = worst.max(reads);
    }
    println!(
        "RUNTIME_LAB slot hash worst case over {} slots: {worst} reads",
        newest - oldest + 1
    );
    assert!(worst <= 4, "{worst} reads");
}

#[test]
fn a_panic_is_silent_by_default() {
    let Some(mut lab) = Lab::load(DEFAULT_ELF) else {
        return;
    };
    let outcome = lab.send(vec![3, 7]);
    assert!(!outcome.ok);
    let text = outcome.logs.join("\n");
    // The runtime says the program failed. The program itself says
    // nothing: no log line, no message, no location.
    assert!(!text.contains("Program log:"), "{text}");
    assert!(!text.contains("runtime lab panic"), "{text}");
    assert!(!text.contains("lib.rs"), "{text}");
    println!("RUNTIME_LAB silent panic: {} CU", outcome.units);
}

#[test]
fn a_reporting_build_says_what_and_where() {
    let Some(mut lab) = Lab::load(REPORTING_ELF) else {
        return;
    };
    let outcome = lab.send(vec![3, 7]);
    assert!(!outcome.ok);
    let text = outcome.logs.join("\n");
    assert!(
        text.contains("Program log: runtime lab panic, code 7"),
        "{text}"
    );
    // The runtime's own line: `Panicked in <file> at <line>:<column>`.
    assert!(text.contains("Panicked in"), "{text}");
    assert!(text.contains("lib.rs at "), "{text}");
    println!("RUNTIME_LAB reported panic: {} CU", outcome.units);
    for line in outcome.logs {
        println!("RUNTIME_LAB log: {line}");
    }
    // The reporting build still runs everything else.
    assert!(lab.send(allocate(1024)).ok);
}
