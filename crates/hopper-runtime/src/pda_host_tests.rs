//! Off-chain PDA derivation against the canonical implementation.
//!
//! `hopper_native` decides curve membership with its own `const fn`; the
//! cluster decides it with `curve25519-dalek` behind a syscall. These tests
//! hold the two to the same answer on every input tried: random 32-byte
//! strings, hashes of real seed sets, and the search over bumps.

use crate::address::Address;
use crate::error::ProgramError;
use crate::pda;
use hopper_native::curve25519::is_on_curve;
use solana_pubkey::Pubkey;
use std::vec::Vec;

/// A small deterministic generator: the tests must not depend on a seed
/// that changes between runs.
struct Stream(u64);

impl Stream {
    fn next(&mut self) -> u64 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn bytes(&mut self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for chunk in out.chunks_mut(8) {
            chunk.copy_from_slice(&self.next().to_le_bytes());
        }
        out
    }
}

#[test]
fn curve_membership_matches_the_canonical_decision() {
    let mut stream = Stream(1);
    let (mut on, mut off) = (0, 0);
    for _ in 0..4_000 {
        let bytes = stream.bytes();
        let expected = Pubkey::new_from_array(bytes).is_on_curve();
        assert_eq!(is_on_curve(&bytes), expected, "{bytes:?}");
        if expected {
            on += 1;
        } else {
            off += 1;
        }
    }
    // About half of all strings are points; both answers were exercised.
    assert!(on > 1_500 && off > 1_500, "{on} on, {off} off");
}

#[test]
fn curve_membership_matches_at_the_edges_of_the_field() {
    let mut cases: Vec<[u8; 32]> = Vec::new();
    // Small values of y, and values around p = 2^255 - 19, with and
    // without the sign bit: y >= p is reduced, never refused.
    for low in 0..=40u8 {
        let mut small = [0u8; 32];
        small[0] = low;
        cases.push(small);
        let mut near_p = [0xffu8; 32];
        near_p[0] = 0xed_u8.wrapping_sub(20).wrapping_add(low);
        near_p[31] = 0x7f;
        cases.push(near_p);
    }
    cases.push([0xff; 32]);
    for case in cases.clone() {
        let mut signed = case;
        signed[31] ^= 0x80;
        cases.push(signed);
    }
    for bytes in cases {
        assert_eq!(
            is_on_curve(&bytes),
            Pubkey::new_from_array(bytes).is_on_curve(),
            "{bytes:?}"
        );
    }
}

fn seed_sets(stream: &mut Stream) -> Vec<Vec<Vec<u8>>> {
    let mut sets = std::vec![
        Vec::new(),
        std::vec![b"config".to_vec()],
        std::vec![Vec::new()],
        std::vec![std::vec![0xab; 32]],
    ];
    for count in 1..=15usize {
        let mut set = Vec::new();
        for i in 0..count {
            let len = (stream.next() % 33) as usize;
            let mut seed = stream.bytes()[..len.min(32)].to_vec();
            seed.truncate(len);
            if i == 0 && seed.is_empty() {
                seed.push(1);
            }
            set.push(seed);
        }
        sets.push(set);
    }
    sets
}

#[test]
fn find_and_create_match_the_canonical_derivation() {
    let mut stream = Stream(2);
    for round in 0..8 {
        let program = stream.bytes();
        let program_id = Address::new_from_array(program);
        let canonical_program = Pubkey::new_from_array(program);
        for set in seed_sets(&mut stream) {
            let seeds: Vec<&[u8]> = set.iter().map(|s| s.as_slice()).collect();
            let (expected, expected_bump) =
                Pubkey::find_program_address(&seeds, &canonical_program);
            let (address, bump) = pda::find_program_address(&seeds, &program_id);
            assert_eq!(
                (address.as_array(), bump),
                (&expected.to_bytes(), expected_bump),
                "round {round}, {} seeds",
                seeds.len()
            );
            assert_eq!(
                pda::find_program_address_const(&seeds, &program_id),
                (address, bump)
            );

            // Every bump: created exactly when the canonical one is.
            for candidate in [255u8, 254, 253, bump, bump.wrapping_sub(1), 0] {
                let bump_seed = [candidate];
                let mut with_bump = seeds.clone();
                with_bump.push(&bump_seed);
                let canonical = Pubkey::create_program_address(&with_bump, &canonical_program);
                let ours = pda::create_program_address(&with_bump, &program_id);
                match (canonical, ours) {
                    (Ok(c), Ok(o)) => assert_eq!(o.as_array(), &c.to_bytes()),
                    (Err(_), Err(e)) => assert_eq!(e, ProgramError::InvalidSeeds),
                    (c, o) => panic!("bump {candidate}: canonical {c:?}, ours {o:?}"),
                }
            }

            // The verify helpers accept the address and refuse another.
            let bump_seed = [bump];
            let mut with_bump = seeds.clone();
            with_bump.push(&bump_seed);
            assert_eq!(
                pda::verify_pda_address(&with_bump, &program_id, &address),
                Ok(())
            );
            assert_eq!(
                pda::verify_pda_address_checked(&with_bump, &program_id, &address),
                Ok(())
            );
            assert_eq!(
                pda::find_bump_for_address(&seeds, &program_id, &address),
                Ok(bump)
            );
            assert_eq!(
                pda::find_canonical_bump_checked(&seeds, &program_id, &address),
                Ok(bump)
            );
            let other = Address::new_from_array(stream.bytes());
            assert_eq!(
                pda::verify_pda_address(&with_bump, &program_id, &other),
                Err(ProgramError::InvalidSeeds)
            );
            assert_eq!(
                pda::find_bump_for_address(&seeds, &program_id, &other),
                Err(ProgramError::InvalidSeeds)
            );
        }
    }
}

#[test]
fn a_bump_the_runtime_skips_is_skipped_here() {
    // Find seed sets whose canonical bump is not 255: the search had to
    // pass over at least one bump whose hash is a curve point.
    let mut stream = Stream(3);
    let program_id = Address::new_from_array(stream.bytes());
    let mut skipped = 0;
    for i in 0..400u32 {
        let seed = i.to_le_bytes();
        let (address, bump) = pda::find_program_address(&[&seed], &program_id);
        if bump < 255 {
            skipped += 1;
            let refused = pda::create_program_address(&[&seed, &[255]], &program_id);
            assert_eq!(refused, Err(ProgramError::InvalidSeeds));
            // The unchecked hash still exists; it is the curve check that
            // refuses it.
            let unchecked = pda::const_program_address(&program_id, &[&seed], 255);
            assert!(is_on_curve(unchecked.as_array()));
            assert!(!is_on_curve(address.as_array()));
        }
    }
    assert!(skipped > 100, "{skipped} of 400 seed sets skipped a bump");
}

#[test]
fn seed_limits_are_the_runtime_limits() {
    let program_id = Address::new_from_array([9; 32]);
    let long = [0u8; 33];
    assert_eq!(
        pda::create_program_address(&[&long], &program_id),
        Err(ProgramError::InvalidSeeds)
    );
    let one = [1u8];
    let sixteen: Vec<&[u8]> = (0..16).map(|_| &one[..]).collect();
    assert!(
        pda::create_program_address(&sixteen, &program_id).is_ok()
            || pda::create_program_address(&sixteen, &program_id)
                == Err(ProgramError::InvalidSeeds)
    );
    let seventeen: Vec<&[u8]> = (0..17).map(|_| &one[..]).collect();
    assert_eq!(
        pda::create_program_address(&seventeen, &program_id),
        Err(ProgramError::InvalidSeeds)
    );
    assert!(Pubkey::create_program_address(&seventeen, &Pubkey::new_from_array([9; 32])).is_err());
}

const PROGRAM: Address = Address::new_from_array([0x42; 32]);
const CONFIG: (Address, u8) = pda::find_program_address_const(&[b"config"], &PROGRAM);
const VAULT: (Address, u8) =
    pda::find_program_address_const(&[b"vault", PROGRAM.as_array()], &PROGRAM);

#[test]
fn a_const_pda_is_the_address_the_cluster_derives() {
    let program = Pubkey::new_from_array([0x42; 32]);
    let (config, bump) = Pubkey::find_program_address(&[b"config"], &program);
    assert_eq!((CONFIG.0.as_array(), CONFIG.1), (&config.to_bytes(), bump));
    let (vault, bump) = Pubkey::find_program_address(&[b"vault", program.as_ref()], &program);
    assert_eq!((VAULT.0.as_array(), VAULT.1), (&vault.to_bytes(), bump));
}
