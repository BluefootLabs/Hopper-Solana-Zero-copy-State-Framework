//! END-TO-END: drive `examples/hopper-sentinel` through the Hopper host
//! harness, capture the REAL touch map + pre/post bytes, and run the Grillo
//! verifier against the REAL manifest.
//!
//! This is the test that matters. It proves the whole chain on real
//! execution, not synthetic inputs:
//!
//! 1. `honest_pause` runs through its generated `Pause` context (borrow
//!    tracking + strict-write policy live), and the verifier returns PASS
//!    with exactly the `paused` + `revision` bytes acquired-and-changed.
//! 2. Corrupting an admin byte in the POST snapshot — a change the handler
//!    never made — is caught as `UntrackedWrite` at exactly the admin
//!    offset (`changed ⊄ acquired`).
//! 3. A forged touch-map record claiming an admin write is caught as
//!    `UnauthorizedAcquisition` (`acquired ⊄ authorized`).
//! 4. The real mutation-complete `collect_fees` contract catches an
//!    unauthorized lamport drain on `treasury`.
//!
//! The harness binds the generated dispatcher against real `AccountView`
//! memory, exactly as `examples/hopper-sentinel/tests/flagship_host.rs`
//! does; because `HopperSvm` takes a plain `fn` pointer, the emitted touch
//! map is smuggled out of the handler through a thread-local.

use std::cell::RefCell;

use grillo_manifest::{MutationManifest, RangeContract};
use grillo_verifier::{
    decode_touch_map, verify, AccountDelta, TouchMap, TouchRecord, Verdict, Violation,
};

use hopper::layout::write_header;
use hopper::prelude::*;
// `hopper::prelude` re-exports a `Vec` type alias (the no_alloc authoring
// vec); re-shadow it with the std `Vec` this host-side test actually uses.
use std::vec::Vec;

use hopper_sentinel::{Config, Pause};
use hopper_svm::{AccountFixture, HopperSvm};

/// The REAL manifest, generated from source (see
/// `crates/grillo-manifest/tests/fixtures`).
const SENTINEL_MANIFEST: &str =
    include_str!("../../grillo-manifest/tests/fixtures/hopper-sentinel.manifest.json");

fn sentinel_manifest() -> MutationManifest {
    MutationManifest::from_json(SENTINEL_MANIFEST).expect("real sentinel manifest parses")
}

const PROGRAM_ID: Address = Address::new_from_array([7u8; 32]);

fn admin_addr() -> Address {
    Address::new_from_array([1u8; 32])
}
fn config_addr() -> Address {
    Address::new_from_array([2u8; 32])
}
fn system_addr() -> Address {
    Address::new_from_array([0u8; 32])
}

fn signer(addr: Address) -> AccountFixture {
    AccountFixture::new(addr, system_addr(), 1_000_000, 0).signer()
}

/// A program-owned config with a valid Hopper header and `admin` +
/// `withdraw_authority` seeded to `admin` — what `initialize_config` leaves.
fn seeded_config_data(admin: Address) -> Vec<u8> {
    let mut data = vec![0u8; Config::LEN];
    write_header(&mut data, Config::DISC, Config::VERSION, &Config::LAYOUT_ID).unwrap();
    let a = Config::ADMIN_ABS_OFFSET as usize;
    data[a..a + 32].copy_from_slice(&admin.to_bytes());
    let w = Config::WITHDRAW_AUTHORITY_ABS_OFFSET as usize;
    data[w..w + 32].copy_from_slice(&admin.to_bytes());
    data
}

fn config_fixture(data: Vec<u8>) -> AccountFixture {
    AccountFixture::with_data(config_addr(), PROGRAM_ID, 5_000_000, data).writable()
}

// `HopperSvm::process_instruction` takes a plain `fn` pointer, so the touch
// map the handler encodes is carried out through this thread-local. The
// touch log itself is per-thread (thread-local-registry dev-feature), so
// parallel tests never collide.
thread_local! {
    static CAPTURED_MAP: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// The honest pause body (mirrors `sentinel_program::honest_pause`), plus a
/// tail that encodes the touch map and stashes it for the test to read.
fn honest_pause_capturing<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    {
        let mut bound = Pause::bind(&mut ctx)?;
        {
            let mut paused = bound.config_paused_mut()?;
            *paused = 1u8;
        }
        {
            let mut revision = bound.config_revision_mut()?;
            revision.checked_add_assign(1)?;
        }
    }
    let (buf, len) = ctx.encode_touch_map();
    CAPTURED_MAP.with(|c| *c.borrow_mut() = buf[..len].to_vec());
    Ok(())
}

/// Run the honest pause once; return `(pre_config, post_config, touch_map)`.
fn run_honest_pause() -> (Vec<u8>, Vec<u8>, TouchMap) {
    let admin = admin_addr();
    let pre = seeded_config_data(admin);
    let accounts = [signer(admin), config_fixture(pre.clone())];
    CAPTURED_MAP.with(|c| c.borrow_mut().clear());
    let result =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, honest_pause_capturing);
    assert!(
        result.program_result.is_ok(),
        "honest pause must succeed: {:?}",
        result.program_result
    );
    let post = result.resulting_accounts[1].data.clone();
    let raw = CAPTURED_MAP.with(|c| c.borrow().clone());
    let map = decode_touch_map(&raw).expect("captured touch map decodes");
    (pre, post, map)
}

// ── 1. Honest pause verifies as PASS ────────────────────────────────────

#[test]
fn honest_pause_verifies_as_pass_with_paused_and_revision_acquired_and_changed() {
    let (pre, post, map) = run_honest_pause();

    // The REAL emitted map: exactly the two declared writes.
    let writes: Vec<TouchRecord> = map.writes().copied().collect();
    assert_eq!(
        writes,
        vec![
            TouchRecord {
                slot: 1,
                offset: Config::PAUSED_ABS_OFFSET,
                size: 1,
                write: true
            },
            TouchRecord {
                slot: 1,
                offset: Config::REVISION_ABS_OFFSET,
                size: 8,
                write: true
            },
        ],
        "the on-chain touch map describes exactly paused + revision"
    );

    let manifest = sentinel_manifest();
    let pause = manifest.instruction("honest_pause").unwrap();
    let verdict = verify(pause, &[AccountDelta::new(1, &pre, &post)], &map);

    match &verdict {
        Verdict::Pass(ev) => {
            // Only two bytes actually flipped: paused (114) and revision's
            // low byte (115). They coalesce to one (114, 2) changed range —
            // acquired ⊇ changed (the revision lease is 8 bytes; 7 of them
            // were acquired-but-unchanged, which is legal).
            assert_eq!(ev.changed_bytes, 2, "exactly two bytes changed");
            assert_eq!(
                ev.changed,
                vec![RangeContract {
                    account_index: 1,
                    offset: Config::PAUSED_ABS_OFFSET,
                    size: 2
                }]
            );
        }
        other => panic!("expected PASS; got:\n{}", other.render()),
    }
    assert!(verdict.is_pass());
}

// ── 2. Tampered POST snapshot => UntrackedWrite at the admin offset ──────

#[test]
fn corrupting_an_admin_byte_in_post_is_an_untracked_write() {
    let (pre, post, map) = run_honest_pause();

    // Flip a byte in `admin` — a change the handler never made and the touch
    // map never advertised.
    let mut tampered = post.clone();
    let admin_off = Config::ADMIN_ABS_OFFSET as usize;
    tampered[admin_off] ^= 0xFF;

    let manifest = sentinel_manifest();
    let pause = manifest.instruction("honest_pause").unwrap();
    let verdict = verify(pause, &[AccountDelta::new(1, &pre, &tampered)], &map);

    match &verdict {
        Verdict::Violation(v) => assert_eq!(
            v,
            &vec![Violation::UntrackedWrite {
                account_index: 1,
                offset: Config::ADMIN_ABS_OFFSET
            }],
            "the corrupted admin byte is an untracked write at exactly its offset"
        ),
        other => panic!("expected VIOLATION; got:\n{}", other.render()),
    }
}

// ── 3. Forged touch-map record => UnauthorizedAcquisition ────────────────

#[test]
fn a_forged_admin_write_record_is_an_unauthorized_acquisition() {
    let (pre, post, map) = run_honest_pause();

    // Append a forged WRITE record claiming an admin-key rotation the honest
    // body never performed. The POST bytes are the real, unchanged admin.
    let mut forged = map.clone();
    forged.records.push(TouchRecord {
        slot: 1,
        offset: Config::ADMIN_ABS_OFFSET,
        size: 32,
        write: true,
    });

    let manifest = sentinel_manifest();
    let pause = manifest.instruction("honest_pause").unwrap();
    let verdict = verify(pause, &[AccountDelta::new(1, &pre, &post)], &forged);

    match &verdict {
        Verdict::Violation(v) => assert_eq!(
            v,
            &vec![Violation::UnauthorizedAcquisition {
                account_index: 1,
                offset: Config::ADMIN_ABS_OFFSET,
                size: 32
            }],
            "the forged admin write lease is outside the authorized set"
        ),
        other => panic!("expected VIOLATION; got:\n{}", other.render()),
    }
}

// ── 4. Real mutation-complete contract: unauthorized lamport drain ───────

#[test]
fn collect_fees_catches_an_unauthorized_treasury_lamport_drain() {
    let manifest = sentinel_manifest();
    let cf = manifest.instruction("collect_fees").unwrap();
    assert!(
        cf.mutation_complete,
        "collect_fees declared the lamport dimension"
    );
    assert_eq!(cf.lamport_accounts, vec![1], "only fee_sink (index 1)");

    // config(0): revision data range bumped. fee_sink(1): a lamport credit
    // it IS authorized for. treasury(2): a lamport drain it is NOT.
    let pre_cfg = vec![0u8; Config::LEN];
    let mut post_cfg = pre_cfg.clone();
    post_cfg[Config::REVISION_ABS_OFFSET as usize] = 1;
    let empty: &[u8] = &[];

    let map = TouchMap {
        overflowed: false,
        skipped: false,
        records: vec![TouchRecord {
            slot: 0,
            offset: Config::REVISION_ABS_OFFSET,
            size: 8,
            write: true,
        }],
    };
    let deltas = [
        AccountDelta::new(0, &pre_cfg, &post_cfg),
        AccountDelta::new(1, empty, empty).with_lamports(1_000, 2_000), // authorized credit
        AccountDelta::new(2, empty, empty).with_lamports(5_000, 0),     // unauthorized drain
    ];

    match verify(cf, &deltas, &map) {
        Verdict::Violation(v) => assert_eq!(
            v,
            vec![Violation::UnauthorizedLamportDelta {
                account_index: 2,
                pre_lamports: 5_000,
                post_lamports: 0
            }]
        ),
        other => panic!("expected VIOLATION; got:\n{}", other.render()),
    }
}
