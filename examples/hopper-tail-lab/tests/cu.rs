//! Compute units of the tail lab's update instructions under Mollusk.
//!
//! Points at `target/deploy/hopper_tail_lab.so` unless `HOPPER_TAIL_LAB_ELF`
//! names another artifact stem, so two builds of the program can be
//! measured with the same instructions. Skips without an artifact.

use hopper_test::{fixtures, LiteSvmHarness};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

const NOTE_BODY_MAX: usize = 160;

fn bounded(text: &str) -> Vec<u8> {
    let mut out = (text.len() as u16).to_le_bytes().to_vec();
    out.extend_from_slice(text.as_bytes());
    out
}

#[test]
fn update_instructions_cost() {
    let stem = std::env::var("HOPPER_TAIL_LAB_ELF")
        .unwrap_or_else(|_| "../../target/deploy/hopper_tail_lab".to_string());
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = LiteSvmHarness::load(&program_id, &stem) else {
        eprintln!("SKIPPED: build {stem}.so first");
        return;
    };
    let authority = Pubkey::new_unique();
    let note = Pubkey::new_unique();
    let (system, system_account) = LiteSvmHarness::system_program_account();

    let body = "x".repeat(NOTE_BODY_MAX);
    let mut init = vec![0u8];
    init.extend(bounded("audit"));
    init.extend(bounded(&body));
    let result = svm.process(
        &Instruction::new_with_bytes(
            program_id,
            &init,
            vec![
                AccountMeta::new(authority, true),
                AccountMeta::new(note, true),
                AccountMeta::new_readonly(system, false),
            ],
        ),
        &[
            (authority, fixtures::wallet(1_000_000_000)),
            (note, Account::new(0, 0, &system)),
            (system, system_account),
        ],
    );
    assert!(
        result.succeeded(),
        "init_note failed: {:?}",
        result.raw().program_result
    );
    let note_account = result.account(1).clone();
    let authority_account = result.account(0).clone();

    let reviewer = Pubkey::new_unique();
    let mut add = vec![2u8];
    add.extend_from_slice(reviewer.as_ref());
    svm.capture_logs();
    let result = svm.process(
        &Instruction::new_with_bytes(
            program_id,
            &add,
            vec![
                AccountMeta::new_readonly(authority, true),
                AccountMeta::new(note, false),
            ],
        ),
        &[
            (authority, authority_account.clone()),
            (note, note_account.clone()),
        ],
    );
    assert!(result.succeeded(), "add_reviewer failed: {:?}", svm.logs());
    let add_cu = result.compute_units();
    let after_add = result.account(1).clone();

    let mut rewrite = vec![1u8];
    rewrite.extend(bounded("ops"));
    rewrite.extend(bounded(&"y".repeat(NOTE_BODY_MAX - 40)));
    let result = svm.process(
        &Instruction::new_with_bytes(
            program_id,
            &rewrite,
            vec![
                AccountMeta::new_readonly(authority, true),
                AccountMeta::new(note, false),
            ],
        ),
        &[(authority, authority_account), (note, after_add)],
    );
    assert!(result.succeeded(), "rewrite_note failed: {:?}", svm.logs());
    let rewrite_cu = result.compute_units();
    println!("TAIL_LAB_CU add_reviewer={add_cu} rewrite_note={rewrite_cu} elf={stem}");
}
