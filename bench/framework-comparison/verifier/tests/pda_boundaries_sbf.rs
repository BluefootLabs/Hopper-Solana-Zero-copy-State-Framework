//! Differential PDA checks against the SDK, including hash-equivalent invalid
//! seed lists. A rejection cannot pass merely because its hash did not match.
use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_instruction_error::InstructionError;
use solana_pubkey::Pubkey;

#[test]
#[ignore = "requires HOPPER_PDA_SBF pointing to the compiled pda-boundaries fixture"]
fn pda_helpers_match_the_signing_seed_domain_in_sbf() {
    let elf = std::fs::read(std::env::var("HOPPER_PDA_SBF").expect("set HOPPER_PDA_SBF"))
        .expect("read PDA fixture ELF");
    let program_id = Pubkey::new_from_array([91; 32]);
    let mut svm = Mollusk::default();
    svm.add_program_with_loader_and_elf(&program_id, &LOADER_V3, &elf);

    let exercise = |label: &str, base: &[&[u8]], equivalent: &[&[u8]], valid: bool| {
        let (expected, bump) = Pubkey::find_program_address(equivalent, &program_id);
        let bump_seed = [bump];
        let mut full = base.to_vec();
        full.push(&bump_seed);
        assert_eq!(
            Pubkey::create_program_address(&full, &program_id).is_ok(),
            valid
        );
        let snapshot = vec![(
            expected,
            Account {
                lamports: 1_000_000,
                data: vec![bump],
                owner: program_id,
                executable: false,
                rent_epoch: 0,
            },
        )];
        for api in 0..12 {
            let seeds = if api < 2 { full.as_slice() } else { base };
            let mut data = vec![api, seeds.len() as u8];
            for seed in seeds {
                data.push(seed.len() as u8);
                data.extend_from_slice(seed);
            }
            data.push(bump);
            let instruction = Instruction::new_with_bytes(
                program_id,
                &data,
                vec![AccountMeta::new_readonly(expected, false)],
            );
            let result = svm.process_instruction(&instruction, &snapshot);
            assert_eq!(
                result.raw_result,
                if valid {
                    Ok(())
                } else {
                    Err(InstructionError::InvalidSeeds)
                },
                "{label}, API {api}"
            );
            assert_eq!(
                result.resulting_accounts, snapshot,
                "{label}, API {api}: mutated input"
            );
        }
        println!("{label}: all 12 public PDA paths matched SDK seed limits");
    };

    exercise("empty base seeds", &[], &[], true);
    exercise("32-byte seed", &[&[7; 32]], &[&[7; 32]], true);
    let mut limit: Vec<&[u8]> = vec![b"pda-boundaries"];
    limit.resize(15, b"");
    exercise("15 base seeds plus bump", &limit, &limit, true);
    let mut excessive = limit.clone();
    excessive.push(b"");
    exercise(
        "16 base seeds plus bump, hash-equivalent",
        &excessive,
        &limit,
        false,
    );
    excessive.push(b"");
    exercise("17 base seeds, truncated suffix", &excessive, &limit, false);
    excessive.push(b"ignored-by-old-loop");
    exercise(
        "nonempty suffix must never be ignored",
        &excessive,
        &limit,
        false,
    );
    let long_seed = [5; 33];
    exercise(
        "33-byte seed, hash-equivalent",
        &[&long_seed],
        &[&long_seed[..16], &long_seed[16..]],
        false,
    );
}
