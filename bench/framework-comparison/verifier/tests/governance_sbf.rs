//! Execute the actual treasury and bounded multisig ELFs with complete state checks.
use hopper::prelude::*;
use mollusk_svm::{program::loader_keys::LOADER_V3, Mollusk};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use std::vec::Vec;
#[path = "../../../../examples/hopper-bounded-multisig/src/state.rs"]
mod multisig;
#[allow(dead_code)]
#[path = "../../../../examples/hopper-treasury/src/state.rs"]
mod treasury;

struct Harness {
    svm: Mollusk,
    program: Pubkey,
    accounts: Vec<(Pubkey, Account)>,
}
impl Harness {
    fn new(env: &str) -> Self {
        let mut svm = Mollusk::default();
        svm.sysvars.clock.unix_timestamp = 1000;
        let program = Pubkey::new_unique();
        svm.add_program_with_loader_and_elf(
            &program,
            &LOADER_V3,
            &std::fs::read(std::env::var(env).expect("set program ELF path")).unwrap(),
        );
        let mut accounts: Vec<_> = (0..8)
            .map(|_| (Pubkey::new_unique(), Account::default()))
            .collect();
        for (i, (_, a)) in accounts.iter_mut().enumerate() {
            if i != 1 {
                a.lamports = 10_000_000;
            }
        }
        accounts[0].1.lamports = 1_000_000_000;
        accounts[7] = mollusk_svm::program::keyed_account_for_system_program();
        Self {
            svm,
            program,
            accounts,
        }
    }
    fn ix(&self, tag: u8, body: &[u8], roles: &[(usize, bool, bool)]) -> Instruction {
        let mut data = vec![tag];
        data.extend_from_slice(body);
        Instruction::new_with_bytes(
            self.program,
            &data,
            roles
                .iter()
                .map(|&(i, w, s)| {
                    if w {
                        AccountMeta::new(self.accounts[i].0, s)
                    } else {
                        AccountMeta::new_readonly(self.accounts[i].0, s)
                    }
                })
                .collect(),
        )
    }
    fn accept(&mut self, ix: &Instruction, expected: Vec<(Pubkey, Account)>) {
        let result = self.svm.process_instruction(ix, &self.accounts);
        assert_eq!(result.raw_result, Ok(()), "tag {}", ix.data[0]);
        assert_eq!(
            result.resulting_accounts, expected,
            "complete state: tag {}",
            ix.data[0]
        );
        self.accounts = result.resulting_accounts;
    }
    fn refuse(&self, ix: &Instruction) {
        let result = self.svm.process_instruction(ix, &self.accounts);
        assert!(
            result.raw_result.is_err(),
            "accepted invalid request {:?}",
            ix.data
        );
        assert_eq!(
            result.resulting_accounts, self.accounts,
            "refusal changed accounts"
        );
    }
    fn treasury_init(&mut self) {
        let ix = self.ix(
            0,
            &words(&[5000, 3000, 60]),
            &[(0, true, true), (1, true, true), (7, false, false)],
        );
        let mut expected = self.accounts.clone();
        let rent = self
            .svm
            .sysvars
            .rent
            .minimum_balance(treasury::TREASURY_ACCOUNT_SIZE);
        expected[0].1.lamports -= rent;
        expected[1].1 = Account::new(rent, treasury::TREASURY_ACCOUNT_SIZE, &self.program);
        let data = &mut expected[1].1.data;
        treasury::TreasuryCore::write_init_header(&mut data[..treasury::PERM_OFFSET]).unwrap();
        treasury::PermissionSegment::write_init_header(
            &mut data[treasury::PERM_OFFSET..treasury::BUDGET_OFFSET],
        )
        .unwrap();
        treasury::BudgetSegment::write_init_header(&mut data[treasury::BUDGET_OFFSET..]).unwrap();
        data[16..48].copy_from_slice(self.accounts[0].0.as_ref());
        data[treasury::PERM_OFFSET + 16..treasury::PERM_OFFSET + 48]
            .copy_from_slice(self.accounts[0].0.as_ref());
        put(data, treasury::PERM_OFFSET + 49, 3000);
        put(data, treasury::BUDGET_OFFSET + 16, 5000);
        put(data, treasury::BUDGET_OFFSET + 40, 60);
        self.accept(&ix, expected);
    }
    fn treasury_deposit(&mut self, amount: u64) {
        let ix = self.ix(
            1,
            &words(&[amount]),
            &[(0, true, true), (1, true, false), (7, false, false)],
        );
        let mut expected = self.accounts.clone();
        expected[0].1.lamports -= amount;
        expected[1].1.lamports += amount;
        let old = u64::from_le_bytes(expected[1].1.data[48..56].try_into().unwrap());
        put(&mut expected[1].1.data, 48, old + amount);
        self.accept(&ix, expected);
    }
    fn treasury_withdraw(&mut self, amount: u64) {
        let ix = self.ix(
            2,
            &words(&[amount]),
            &[(0, false, true), (1, true, false), (3, true, false)],
        );
        let mut expected = self.accounts.clone();
        expected[1].1.lamports -= amount;
        expected[3].1.lamports += amount;
        let off = treasury::BUDGET_OFFSET + 24;
        let old = u64::from_le_bytes(expected[1].1.data[off..off + 8].try_into().unwrap());
        put(&mut expected[1].1.data, off, old + amount);
        put(
            &mut expected[1].1.data,
            treasury::BUDGET_OFFSET + 48,
            self.svm.sysvars.clock.unix_timestamp as u64,
        );
        self.accept(&ix, expected);
    }
    fn multisig_init(&mut self) {
        let keys: Vec<_> = [4, 5, 6]
            .iter()
            .map(|&i| Address::new(self.accounts[i].0.to_bytes()))
            .collect();
        let mut body = words(&[2]);
        body.extend(string("ops"));
        body.extend(3u16.to_le_bytes());
        for key in &keys {
            body.extend_from_slice(key.as_ref());
        }
        let ix = self.ix(
            2,
            &body,
            &[(0, true, true), (1, true, true), (7, false, false)],
        );
        let mut bad = ix.clone();
        bad.data[1..9].fill(0);
        self.refuse(&bad);
        let mut bad = ix.clone();
        let offset = bad.data.len() - 96;
        bad.data.copy_within(offset..offset + 32, offset + 32);
        self.refuse(&bad);
        for i in [0, 1] {
            let mut bad = ix.clone();
            bad.accounts[i].is_signer = false;
            self.refuse(&bad);
        }
        let mut expected = self.accounts.clone();
        let rent = self
            .svm
            .sysvars
            .rent
            .minimum_balance(multisig::Multisig::ALLOC_SPACE);
        expected[0].1.lamports -= rent;
        expected[1].1 = Account::new(rent, multisig::Multisig::ALLOC_SPACE, &self.program);
        multisig::initialize_multisig_data(&mut expected[1].1.data, 2, "ops", &keys).unwrap();
        self.accept(&ix, expected);
    }
}
fn words(values: &[u64]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn put(data: &mut [u8], offset: usize, value: u64) {
    data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn string(value: &str) -> Vec<u8> {
    let mut bytes = (value.len() as u16).to_le_bytes().to_vec();
    bytes.extend(value.as_bytes());
    bytes
}

#[test]
#[ignore = "requires compiled treasury ELF"]
fn treasury_moves_sol_and_enforces_clock_budget_and_permissions() {
    let mut h = Harness::new("HOPPER_TREASURY_SBF");
    h.treasury_init();
    h.treasury_deposit(10_000);
    let deposit = h.ix(
        1,
        &words(&[1]),
        &[(0, true, true), (1, true, false), (7, false, false)],
    );
    for index in [0, 1] {
        let mut bad = deposit.clone();
        bad.accounts[index].is_writable = false;
        h.refuse(&bad);
    }
    let mut bad = deposit.clone();
    bad.accounts[0].is_signer = false;
    h.refuse(&bad);
    let mut bad = deposit.clone();
    bad.accounts[2].pubkey = h.accounts[6].0;
    h.refuse(&bad);
    let mut bad = deposit.clone();
    bad.data.push(0);
    h.refuse(&bad);
    h.refuse(&h.ix(
        1,
        &words(&[u64::MAX]),
        &[(0, true, true), (1, true, false), (7, false, false)],
    ));
    // Each segment identity is checked before any money moves.
    for offset in [0, treasury::PERM_OFFSET, treasury::BUDGET_OFFSET] {
        h.accounts[1].1.data[offset] ^= 1;
        h.refuse(&deposit);
        h.accounts[1].1.data[offset] ^= 1;
    }
    h.treasury_withdraw(2000);
    let withdraw = h.ix(
        2,
        &words(&[1]),
        &[(0, false, true), (1, true, false), (3, true, false)],
    );
    h.refuse(&withdraw);
    h.svm.sysvars.clock.unix_timestamp = 1059;
    h.refuse(&withdraw);
    h.svm.sysvars.clock.unix_timestamp = 1060;
    h.treasury_withdraw(3000);
    h.svm.sysvars.clock.unix_timestamp = 1120;
    h.refuse(&withdraw); // exhausted budget
    let rotate = h.ix(4, &words(&[1]), &[(0, false, true), (1, true, false)]);
    let mut expected = h.accounts.clone();
    put(&mut expected[1].1.data, treasury::BUDGET_OFFSET + 24, 0);
    put(&mut expected[1].1.data, treasury::BUDGET_OFFSET + 32, 1);
    h.accept(&rotate, expected);
    h.refuse(&rotate); // period must advance
    let mut bad = withdraw.clone();
    bad.accounts[0].pubkey = h.accounts[6].0;
    h.refuse(&bad);
    let mut bad = withdraw.clone();
    bad.accounts[0].is_signer = false;
    h.refuse(&bad);
    let mut bad = withdraw.clone();
    bad.accounts[2].pubkey = h.accounts[1].0;
    h.refuse(&bad);
    let old = h.accounts[3].1.lamports;
    h.accounts[3].1.lamports = u64::MAX;
    h.refuse(&withdraw);
    h.accounts[3].1.lamports = old;
    let freeze = h.ix(3, &[1], &[(0, false, true), (1, true, false)]);
    let mut expected = h.accounts.clone();
    expected[1].1.data[treasury::PERM_OFFSET + 48] = 1;
    h.accept(&freeze, expected);
    h.refuse(&withdraw);
    let mut expected = h.accounts.clone();
    expected[1].1.data[treasury::PERM_OFFSET + 48] = 0;
    h.accept(&freeze, expected);
    h.treasury_withdraw(3000);
    h.svm.sysvars.clock.unix_timestamp = 1180;
    h.treasury_withdraw(2000);
    let rent = h
        .svm
        .sysvars
        .rent
        .minimum_balance(treasury::TREASURY_ACCOUNT_SIZE);
    assert_eq!(h.accounts[1].1.lamports, rent);
    let rotate = h.ix(4, &words(&[2]), &[(0, false, true), (1, true, false)]);
    let mut expected = h.accounts.clone();
    put(&mut expected[1].1.data, treasury::BUDGET_OFFSET + 24, 0);
    put(&mut expected[1].1.data, treasury::BUDGET_OFFSET + 32, 2);
    h.accept(&rotate, expected);
    h.svm.sysvars.clock.unix_timestamp = 1240;
    h.refuse(&withdraw); // cannot spend rent
}

#[test]
#[ignore = "requires compiled multisig ELF"]
fn multisig_authenticates_members_and_moves_sol() {
    let mut h = Harness::new("HOPPER_MULTISIG_SBF");
    h.multisig_init();
    let deposit = h.ix(
        4,
        &words(&[10000]),
        &[(0, true, true), (1, true, false), (7, false, false)],
    );
    let mut expected = h.accounts.clone();
    expected[0].1.lamports -= 10000;
    expected[1].1.lamports += 10000;
    h.accept(&deposit, expected);
    let rename = h.ix(
        0,
        &string("treasury"),
        &[(1, true, false), (4, false, true), (5, false, true)],
    );
    let mut bad = rename.clone();
    bad.accounts.pop();
    h.refuse(&bad);
    let mut bad = rename.clone();
    bad.accounts[1].is_signer = false;
    h.refuse(&bad);
    let mut bad = rename.clone();
    bad.accounts[1].pubkey = h.accounts[0].0;
    h.refuse(&bad);
    let mut bad = rename.clone();
    bad.accounts[2].pubkey = h.accounts[4].0;
    h.refuse(&bad);
    let mut expected = h.accounts.clone();
    multisig::rename_multisig_data(&mut expected[1].1.data, "treasury").unwrap();
    h.accept(&rename, expected);
    let add = h.ix(
        1,
        h.accounts[2].0.as_ref(),
        &[(1, true, false), (4, false, true), (5, false, true)],
    );
    let mut bad = add.clone();
    bad.accounts.pop();
    h.refuse(&bad);
    let mut expected = h.accounts.clone();
    multisig::add_signer_data(
        &mut expected[1].1.data,
        Address::new(h.accounts[2].0.to_bytes()),
    )
    .unwrap();
    put(&mut expected[1].1.data, 24, 1);
    h.accept(&add, expected);
    let withdraw = h.ix(
        3,
        &words(&[4000]),
        &[
            (1, true, false),
            (3, true, false),
            (4, false, true),
            (5, false, true),
        ],
    );
    for index in [0, 1] {
        let mut bad = withdraw.clone();
        bad.accounts[index].is_writable = false;
        h.refuse(&bad);
    }
    let mut bad = withdraw.clone();
    bad.accounts.pop();
    h.refuse(&bad);
    let mut bad = withdraw.clone();
    bad.accounts[2].pubkey = h.accounts[0].0;
    h.refuse(&bad);
    let mut bad = withdraw.clone();
    bad.accounts[3].pubkey = h.accounts[4].0;
    h.refuse(&bad);
    let mut bad = withdraw.clone();
    bad.accounts[1].pubkey = h.accounts[1].0;
    h.refuse(&bad);
    let mut bad = withdraw.clone();
    bad.data.push(0);
    h.refuse(&bad);
    let old = h.accounts[3].1.lamports;
    h.accounts[3].1.lamports = u64::MAX;
    h.refuse(&withdraw);
    h.accounts[3].1.lamports = old;
    let mut expected = h.accounts.clone();
    expected[1].1.lamports -= 4000;
    expected[3].1.lamports += 4000;
    h.accept(&withdraw, expected);
    h.refuse(&h.ix(
        3,
        &words(&[6001]),
        &[
            (1, true, false),
            (3, true, false),
            (4, false, true),
            (5, false, true),
        ],
    ));
    // The destination is an authenticated member; only the other approval is a tail account.
    let withdraw = h.ix(
        3,
        &words(&[6000]),
        &[(1, true, false), (4, true, true), (6, false, true)],
    );
    let mut bad = withdraw.clone();
    bad.accounts[1].is_signer = false;
    h.refuse(&bad);
    let mut expected = h.accounts.clone();
    expected[1].1.lamports -= 6000;
    expected[4].1.lamports += 6000;
    h.accept(&withdraw, expected);
    assert_eq!(
        h.accounts[1].1.lamports,
        h.svm
            .sysvars
            .rent
            .minimum_balance(multisig::Multisig::ALLOC_SPACE)
    );
}

#[test]
fn program_layout_headers() {
    fn hex(data: &[u8]) -> std::string::String {
        data.iter().map(|b| format!("{b:02x}")).collect()
    }
    let mut core = [0; treasury::TreasuryCore::LEN];
    let mut perm = [0; treasury::PermissionSegment::LEN];
    let mut budget = [0; treasury::BudgetSegment::LEN];
    let mut multi = [0; multisig::Multisig::ALLOC_SPACE];
    let mut payout = [0; multisig::Payout::INIT_SPACE];
    treasury::TreasuryCore::write_init_header(&mut core).unwrap();
    treasury::PermissionSegment::write_init_header(&mut perm).unwrap();
    treasury::BudgetSegment::write_init_header(&mut budget).unwrap();
    hopper::systems::init_header::<multisig::Multisig>(&mut multi).unwrap();
    hopper::systems::init_header::<multisig::Payout>(&mut payout).unwrap();
    println!("program-layout: {{\"core\":\"{}\",\"permissions\":\"{}\",\"budget\":\"{}\",\"multisig\":\"{}\",\"payout\":\"{}\",\"multisigSize\":{},\"payoutSize\":{}}}", hex(&core[..16]),hex(&perm[..16]),hex(&budget[..16]),hex(&multi[..16]),hex(&payout[..16]),multi.len(),payout.len());
}

#[test]
#[ignore = "requires compiled multisig ELF"]
fn multisig_payout_is_bounded_single_use_and_invalidated_by_policy_changes() {
    let mut h = Harness::new("HOPPER_MULTISIG_SBF");
    h.multisig_init();
    h.accounts[1].1.lamports += 10_000;
    h.accounts[2].1 = Account::default();
    let approve = h.ix(
        5,
        &words(&[4_000, 1000, 1060]),
        &[
            (1, true, false),
            (2, true, true),
            (0, true, true),
            (3, false, false),
            (7, false, false),
            (4, false, true),
            (5, false, true),
        ],
    );
    let mut bad = approve.clone();
    bad.accounts.pop();
    h.refuse(&bad);
    let mut bad = approve.clone();
    bad.accounts[6].pubkey = h.accounts[4].0;
    h.refuse(&bad);
    let mut bad = approve.clone();
    bad.accounts[5].pubkey = h.accounts[6].0;
    bad.accounts[6].pubkey = h.accounts[0].0;
    h.refuse(&bad);
    let mut bad = approve.clone();
    bad.data[1..9].fill(0);
    h.refuse(&bad);
    let mut bad = approve.clone();
    bad.data[17..25].copy_from_slice(&999u64.to_le_bytes());
    h.refuse(&bad);
    let mut expected = h.accounts.clone();
    let rent = h
        .svm
        .sysvars
        .rent
        .minimum_balance(multisig::Payout::INIT_SPACE);
    expected[0].1.lamports -= rent;
    expected[2].1 = Account::new(rent, multisig::Payout::INIT_SPACE, &h.program);
    let data = &mut expected[2].1.data;
    hopper::systems::init_header::<multisig::Payout>(data).unwrap();
    data[16..48].copy_from_slice(h.accounts[1].0.as_ref());
    data[48..80].copy_from_slice(h.accounts[3].0.as_ref());
    for (offset, value) in [(80, 0), (88, 4000), (96, 1000), (104, 1060)] {
        put(data, offset, value);
    }
    h.accept(&approve, expected);
    h.refuse(&approve);
    let ready = h.accounts.clone();
    let execute = h.ix(
        6,
        &[],
        &[(1, true, false), (2, true, false), (3, true, false)],
    );
    h.svm.sysvars.clock.unix_timestamp = 999;
    h.refuse(&execute);
    h.svm.sysvars.clock.unix_timestamp = 1061;
    h.refuse(&execute);
    h.svm.sysvars.clock.unix_timestamp = 1000;
    let mut bad = execute.clone();
    bad.accounts[2].pubkey = h.accounts[6].0;
    h.refuse(&bad);
    let mut bad = execute.clone();
    bad.data.push(0);
    h.refuse(&bad);
    for i in 0..3 {
        let mut bad = execute.clone();
        bad.accounts[i].is_writable = false;
        h.refuse(&bad);
    }
    let old = h.accounts[3].1.lamports;
    h.accounts[3].1.lamports = u64::MAX;
    h.refuse(&execute);
    h.accounts[3].1.lamports = old;
    let old = h.accounts[1].1.lamports;
    h.accounts[1].1.lamports -= 6001;
    h.refuse(&execute);
    h.accounts[1].1.lamports = old;
    let mut expected = h.accounts.clone();
    expected[1].1.lamports -= 4000;
    expected[3].1.lamports += 4000;
    expected[2].1.data[112] = 1;
    h.accept(&execute, expected);
    h.refuse(&execute);
    h.accounts = ready.clone();
    h.svm.sysvars.clock.unix_timestamp = 1060;
    let mut expected = h.accounts.clone();
    expected[1].1.lamports -= 4000;
    expected[3].1.lamports += 4000;
    expected[2].1.data[112] = 1;
    h.accept(&execute, expected);
    h.accounts = ready.clone();
    h.svm.sysvars.clock.unix_timestamp = 1000;
    let invalidate = h.ix(
        8,
        &[],
        &[(1, true, false), (4, false, true), (5, false, true)],
    );
    let mut bad = invalidate.clone();
    bad.accounts.pop();
    h.refuse(&bad);
    let mut expected = h.accounts.clone();
    put(&mut expected[1].1.data, 24, 1);
    h.accept(&invalidate, expected);
    h.refuse(&execute);
    h.accounts = ready.clone();
    let add = h.ix(
        1,
        h.accounts[0].0.as_ref(),
        &[(1, true, false), (4, false, true), (5, false, true)],
    );
    let mut expected = h.accounts.clone();
    multisig::add_signer_data(
        &mut expected[1].1.data,
        Address::new(h.accounts[0].0.to_bytes()),
    )
    .unwrap();
    put(&mut expected[1].1.data, 24, 1);
    h.accept(&add, expected);
    h.refuse(&execute);
    h.accounts = ready.clone();
    let remove = h.ix(
        9,
        h.accounts[6].0.as_ref(),
        &[(1, true, false), (4, false, true), (5, false, true)],
    );
    let mut expected = h.accounts.clone();
    multisig::remove_signer_data(
        &mut expected[1].1.data,
        &Address::new(h.accounts[6].0.to_bytes()),
    )
    .unwrap();
    put(&mut expected[1].1.data, 24, 1);
    h.accept(&remove, expected);
    h.refuse(&execute);
    // Removing another member would leave fewer members than the threshold.
    h.refuse(&h.ix(
        9,
        h.accounts[5].0.as_ref(),
        &[(1, true, false), (4, false, true), (5, false, true)],
    ));
    h.accounts = ready.clone();
    h.refuse(&h.ix(
        10,
        &words(&[0]),
        &[(1, true, false), (4, false, true), (5, false, true)],
    ));
    h.refuse(&h.ix(
        10,
        &words(&[4]),
        &[(1, true, false), (4, false, true), (5, false, true)],
    ));
    let change = h.ix(
        10,
        &words(&[3]),
        &[(1, true, false), (4, false, true), (5, false, true)],
    );
    let mut expected = h.accounts.clone();
    put(&mut expected[1].1.data, 16, 3);
    put(&mut expected[1].1.data, 24, 1);
    h.accept(&change, expected);
    h.refuse(&execute);
    h.accounts = ready.clone();
    let revoke = h.ix(
        7,
        &[],
        &[
            (1, true, false),
            (2, true, false),
            (4, false, true),
            (5, false, true),
        ],
    );
    let mut bad = revoke.clone();
    bad.accounts.pop();
    h.refuse(&bad);
    let mut expected = h.accounts.clone();
    expected[1].1.lamports += rent;
    expected[2].1.lamports = 0;
    expected[2].1.data.fill(0);
    h.accept(&revoke, expected);
    h.refuse(&execute);
}
