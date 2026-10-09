#![cfg(feature = "proc-macros")]

use hopper::hopper_runtime::{BoundedString, BoundedVec};
use hopper::prelude::*;
use hopper_svm::{AccountFixture, HopperSvm};

#[hopper::unit_enum]
pub enum Side {
    Bid = 1,
    Ask = 7,
}

#[hopper::pod]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Order {
    pub side: OptionByte<EnumByte<Side>>,
    pub amount: WireU64,
}

type Orders<'a> = BoundedSlice<'a, Order, 4>;
type Label = BoundedString<16>;
type Quantities = BoundedVec<u16, 4>;
type Nonce = u16;
type Salt = [u8; 3];

#[derive(Accounts)]
pub struct Authorized<'info> {
    pub authority: Signer<'info>,
}

#[hopper::program(entrypoint = false)]
mod batches {
    use super::*;

    #[instruction(0)]
    pub fn submit(_ctx: Ctx<Authorized>, orders: Orders<'_>, nonce: u16) -> ProgramResult {
        if orders.len() == 2 && orders.as_slice()[1].amount.get() == 99 && nonce == 513 {
            Ok(())
        } else {
            Err(ProgramError::Custom(7001))
        }
    }

    #[instruction(1)]
    pub fn bytes(
        _ctx: Ctx<Authorized>,
        bytes: BoundedSlice<'_, u8, 8>,
        note: BoundedString<16>,
    ) -> ProgramResult {
        if bytes.as_slice() == [42, 99] && note.as_str()? == "ok" {
            Ok(())
        } else {
            Err(ProgramError::InvalidInstructionData)
        }
    }

    #[instruction(2)]
    pub fn aliases(
        _ctx: Ctx<Authorized>,
        quantities: Quantities,
        label: Label,
        nonce: Nonce,
        salt: Salt,
    ) -> ProgramResult {
        if quantities.as_slice() == [7, 11]
            && label.as_str()? == "é"
            && nonce == 513
            && salt == [1, 2, 3]
        {
            Ok(())
        } else {
            Err(ProgramError::InvalidInstructionData)
        }
    }
}

hopper::program_manifest! { program = batches }

fn payload() -> std::vec::Vec<u8> {
    let mut input = vec![0, 2, 0, 1, 1];
    input.extend_from_slice(&42u64.to_le_bytes());
    input.extend_from_slice(&[1, 7]);
    input.extend_from_slice(&99u64.to_le_bytes());
    input.extend_from_slice(&513u16.to_le_bytes());
    input
}

#[test]
fn borrowed_sequences_compose_and_validate_before_account_binding() {
    let program = Address::new([9; 32]);
    let signer =
        AccountFixture::new(Address::new([8; 32]), Address::new([0; 32]), 1_000_000, 0).signer();
    fn drive<'info>(
        id: &'info Address,
        accounts: &'info [AccountView<'info>],
        data: &'info [u8],
    ) -> ProgramResult {
        batches::process_instruction(&mut Context::new(id, accounts, data))
    }
    let run = |data: &[u8], account: AccountFixture| {
        HopperSvm::new()
            .process_instruction(program, data, &[account], drive)
            .program_result
    };
    let valid = payload();
    assert_eq!(run(&valid, signer.clone()), Ok(()));
    for end in 0..valid.len() {
        assert_eq!(
            run(&valid[..end], signer.clone()),
            Err(ProgramError::InvalidInstructionData)
        );
    }
    let mut extra = valid.clone();
    extra.push(0);
    assert_eq!(
        run(&extra, signer.clone()),
        Err(ProgramError::InvalidInstructionData)
    );
    for tag in 0..=255 {
        let mut malformed = valid.clone();
        malformed[14] = tag;
        let expected = if matches!(tag, 1 | 7) {
            Ok(())
        } else {
            Err(ProgramError::InvalidInstructionData)
        };
        assert_eq!(run(&malformed, signer.clone()), expected);
    }
    let mut unsigned = signer;
    unsigned.is_signer = false;
    assert_eq!(
        run(&valid, unsigned.clone()),
        Err(ProgramError::MissingRequiredSignature)
    );
    let mut malformed = valid;
    malformed[13] = 2;
    assert_eq!(
        run(&malformed, unsigned),
        Err(ProgramError::InvalidInstructionData)
    );
}

#[test]
fn sequence_metadata_follows_aliases_and_clients_keep_variable_width() {
    use hopper::hopper_schema::{clientgen::TsInstructions, rust_client::RsClientGen, ArgEncoding};
    let argument = &PROGRAM_MANIFEST.instructions[0].args[0];
    assert_eq!(argument.size, 42);
    assert_eq!(
        argument.encoding,
        ArgEncoding::BoundedVec {
            max_len: 4,
            element_size: 10
        }
    );
    assert_eq!(argument.fixed_size(), None);
    let ts = TsInstructions(&PROGRAM_MANIFEST).to_string();
    let rust = RsClientGen(&PROGRAM_MANIFEST).to_string();
    let python =
        hopper::hopper_schema::python_client::PyInstructions(&PROGRAM_MANIFEST).to_string();
    let go = hopper::hopper_schema::go_client::GoClientGen(&PROGRAM_MANIFEST).to_string();
    let c = hopper::hopper_schema::c_client::CClientGen(&PROGRAM_MANIFEST).to_string();
    let kotlin = hopper::hopper_schema::clientgen::KtInstructions(&PROGRAM_MANIFEST).to_string();
    assert!(ts.contains("orders: readonly Uint8Array[]"));
    assert!(ts.contains("value.length !== 10"));
    assert!(ts.contains("bytes: Uint8Array"));
    assert!(rust.contains("orders: Vec<[u8; 10]>"));
    assert!(rust.contains("Result<Instruction, ClientError>"));
    assert!(rust.contains("offset != data.len()"));
    let aliases = PROGRAM_MANIFEST.instructions[2].args;
    assert_eq!(
        aliases
            .iter()
            .map(|arg| arg.size)
            .collect::<std::vec::Vec<_>>(),
        [10, 18, 2, 3]
    );
    assert_eq!(
        aliases[0].encoding,
        ArgEncoding::BoundedVec {
            max_len: 4,
            element_size: 2
        }
    );
    assert_eq!(
        aliases[1].encoding,
        ArgEncoding::BoundedString { max_len: 16 }
    );
    assert!(go.contains("bounded instruction arguments require"));
    assert!(!go.contains("data := make([]byte,"));
    assert!(c.contains("return HOPPER_CLIENT_UNSUPPORTED_ENCODING;"));
    assert!(!c.contains("out[0] = HOPPER_"));
    assert!(kotlin.contains("throw UnsupportedOperationException"));
    assert!(!kotlin.contains("val data = ByteArray("));
    if let Ok(output) = std::env::var("HOPPER_SLICE_CLIENT_OUT") {
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(std::path::Path::new(&output).join("instructions.ts"), ts).unwrap();
        std::fs::write(std::path::Path::new(&output).join("client.rs"), rust).unwrap();
        std::fs::write(
            std::path::Path::new(&output).join("instructions.py"),
            python,
        )
        .unwrap();
        std::fs::write(std::path::Path::new(&output).join("client.go"), go).unwrap();
        std::fs::write(std::path::Path::new(&output).join("client.h"), c).unwrap();
        std::fs::write(
            std::path::Path::new(&output).join("instructions.kt"),
            kotlin,
        )
        .unwrap();
    }
}

#[test]
fn aliased_owned_arguments_follow_the_same_dispatch_contract() {
    let program = Address::new([9; 32]);
    let signer = AccountFixture::new(Address::new([8; 32]), Address::new([0; 32]), 1, 0).signer();
    let input = [2, 2, 0, 7, 0, 11, 0, 2, 0, 0xc3, 0xa9, 1, 2, 1, 2, 3];
    fn drive<'info>(
        id: &'info Address,
        accounts: &'info [AccountView<'info>],
        data: &'info [u8],
    ) -> ProgramResult {
        batches::process_instruction(&mut Context::new(id, accounts, data))
    }
    assert_eq!(
        HopperSvm::new()
            .process_instruction(program, &input, std::slice::from_ref(&signer), drive)
            .program_result,
        Ok(())
    );
    for end in 0..input.len() {
        assert_eq!(
            HopperSvm::new()
                .process_instruction(program, &input[..end], std::slice::from_ref(&signer), drive)
                .program_result,
            Err(ProgramError::InvalidInstructionData)
        );
    }
    let mut invalid_utf8 = input;
    invalid_utf8[9] = 0xff;
    let mut unsigned = signer;
    unsigned.is_signer = false;
    assert_eq!(
        HopperSvm::new()
            .process_instruction(program, &invalid_utf8, &[unsigned], drive)
            .program_result,
        Err(ProgramError::InvalidInstructionData)
    );
}
