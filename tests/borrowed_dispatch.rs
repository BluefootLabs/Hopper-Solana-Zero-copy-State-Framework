#![cfg(feature = "proc-macros")]

use hopper::prelude::*;
use hopper_svm::{AccountFixture, HopperSvm};

#[hopper::unit_enum]
pub enum Side {
    Bid = 1,
    Ask = 7,
}

type OptionalSide = OptionByte<EnumByte<Side>>;

#[hopper::args]
#[repr(C)]
pub struct OrderArgs {
    pub sides: [OptionalSide; 2],
    pub amount: WireU64,
}

type Order = OrderArgs;

#[derive(Accounts)]
pub struct Authorized<'info> {
    pub authority: Signer<'info>,
}

#[hopper::program(entrypoint = false)]
mod raw {
    use super::*;

    #[instruction(0)]
    pub fn exact(ctx: &mut Context<'_>, order: &Order) -> ProgramResult {
        assert!(core::ptr::eq(
            order as *const Order,
            ctx.instruction_data()[1..].as_ptr().cast(),
        ));
        Ok(())
    }

    #[instruction(discriminator = [1, 2])]
    pub fn compose(
        ctx: &mut Context<'_>,
        first: &Order,
        nonce: u16,
        second: &OrderArgs,
        tail: &[u8],
    ) -> ProgramResult {
        let data = ctx.instruction_data();
        assert!(core::ptr::eq(
            first as *const Order,
            data[2..].as_ptr().cast()
        ));
        assert!(core::ptr::eq(
            second as *const OrderArgs,
            data[2 + Order::PACKED_SIZE + 2..].as_ptr().cast(),
        ));
        assert!(core::ptr::eq(
            tail.as_ptr(),
            data[2 + Order::PACKED_SIZE * 2 + 2..].as_ptr(),
        ));
        assert_eq!(nonce, 513);
        assert_eq!(first.amount.get(), 42);
        assert_eq!(second.amount.get(), 99);
        if tail.len() > 32 || core::str::from_utf8(tail).is_err() {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

#[hopper::program(entrypoint = false)]
mod typed {
    use super::*;

    #[instruction(0)]
    pub fn submit(_ctx: Ctx<Authorized>, order: &Order, nonce: u16) -> ProgramResult {
        if order.amount.get() == 42 && nonce == 513 {
            Ok(())
        } else {
            Err(ProgramError::Custom(7001))
        }
    }

    #[instruction(1)]
    pub fn empty(_ctx: Ctx<Authorized>, empty: &[u8; 0], nonce: u16) -> ProgramResult {
        assert!(empty.is_empty());
        if nonce != 513 {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

hopper::program_manifest! { program = typed }

fn bytes(amount: u64) -> std::vec::Vec<u8> {
    let mut data = vec![1, 1, 0, 255];
    data.extend_from_slice(&amount.to_le_bytes());
    data
}

fn dispatch(data: &[u8]) -> ProgramResult {
    raw::process_instruction(&mut Context::new(&Address::new([9; 32]), &[], data))
}

#[test]
fn generated_dispatch_checks_nested_values_before_entering_the_handler() {
    for byte in 0..=255 {
        for (position, accepted) in [(0, byte <= 1), (1, matches!(byte, 1 | 7)), (2, byte <= 1)] {
            let mut data = vec![0];
            let mut order = bytes(42);
            order[position] = byte;
            if position == 2 {
                order[3] = 7;
            }
            data.extend(order);
            assert_eq!(
                dispatch(&data).is_ok(),
                accepted,
                "position {position}, byte {byte}"
            );
        }
        let mut data = vec![0];
        let mut order = bytes(42);
        order[3] = byte;
        data.extend(order);
        assert_eq!(dispatch(&data), Ok(()), "absent payload {byte}");
    }
}

#[test]
fn exact_dispatch_rejects_truncation_and_unconsumed_bytes() {
    let mut data = vec![0];
    data.extend(bytes(42));
    assert_eq!(dispatch(&data), Ok(()));
    for length in 0..data.len() {
        assert_eq!(
            dispatch(&data[..length]),
            Err(ProgramError::InvalidInstructionData)
        );
    }
    data.push(0);
    assert_eq!(dispatch(&data), Err(ProgramError::InvalidInstructionData));
}

#[test]
fn borrowed_layouts_compose_with_scalars_other_layouts_and_a_tail() {
    let mut data = vec![1, 2];
    data.extend(bytes(42));
    data.extend(513u16.to_le_bytes());
    data.extend(bytes(99));
    let prefix = data.len();
    for tail in [&b""[..], &b"memo"[..], &[b'x'; 32][..]] {
        data.truncate(prefix);
        data.extend(tail);
        assert_eq!(dispatch(&data), Ok(()));
    }
    for length in 2..prefix {
        assert_eq!(
            dispatch(&data[..length]),
            Err(ProgramError::InvalidInstructionData)
        );
    }
    for tail in [&[255][..], &[b'x'; 33][..]] {
        data.truncate(prefix);
        data.extend(tail);
        assert_eq!(dispatch(&data), Err(ProgramError::InvalidInstructionData));
    }
}

#[test]
fn typed_account_admission_and_business_rules_still_run() {
    fn drive<'a>(
        id: &'a Address,
        accounts: &'a [AccountView<'a>],
        data: &'a [u8],
    ) -> ProgramResult {
        typed::process_instruction(&mut Context::new(id, accounts, data))
    }
    let id = Address::new([9; 32]);
    let signer = AccountFixture::new(Address::new([8; 32]), Address::new([0; 32]), 1, 0).signer();
    let mut data = vec![0];
    data.extend(bytes(42));
    let run = |data: &[u8], account: AccountFixture| {
        HopperSvm::new()
            .process_instruction(id, data, &[account], drive)
            .program_result
    };
    data.extend(513u16.to_le_bytes());
    assert_eq!(run(&data, signer.clone()), Ok(()));
    let mut unsigned = signer.clone();
    unsigned.is_signer = false;
    assert_eq!(
        run(&data, unsigned.clone()),
        Err(ProgramError::MissingRequiredSignature)
    );
    data[1] = 2;
    assert_eq!(
        run(&data, unsigned),
        Err(ProgramError::InvalidInstructionData)
    );
    data = vec![0];
    data.extend(bytes(99));
    data.extend(513u16.to_le_bytes());
    assert_eq!(run(&data, signer.clone()), Err(ProgramError::Custom(7001)));
    assert_eq!(run(&[1, 1, 2], signer.clone()), Ok(()));
    assert_eq!(
        run(&[1, 42, 1, 2], signer),
        Err(ProgramError::InvalidInstructionData)
    );
}

#[test]
fn manifest_preserves_the_borrowed_alias_wire_size() {
    let argument = &PROGRAM_MANIFEST.instructions[0].args[0];
    assert_eq!(argument.name, "order");
    assert_eq!(argument.size as usize, Order::PACKED_SIZE);
    assert_eq!(argument.size, 12);
    assert_eq!(argument.encoding, hopper::hopper_schema::ArgEncoding::Fixed);
    let empty = &PROGRAM_MANIFEST.instructions[1].args[0];
    assert_eq!(empty.size, 0);
    assert_eq!(empty.fixed_size(), Some(0));
}

#[test]
fn generated_clients_use_the_declared_fixed_byte_contract() {
    let ts = hopper::hopper_schema::clientgen::TsInstructions(&PROGRAM_MANIFEST).to_string();
    let rust = hopper::hopper_schema::rust_client::RsClientGen(&PROGRAM_MANIFEST).to_string();
    assert!(ts.contains("order: Uint8Array"));
    assert!(ts.contains("args.order.length !== 12"));
    assert!(rust.contains("order: [u8; 12]"));
    assert!(ts.contains("args.empty.length !== 0"));
    assert!(rust.contains("empty: [u8; 0]"));
    if let Ok(output) = std::env::var("HOPPER_BORROWED_CLIENT_OUT") {
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(std::path::Path::new(&output).join("instructions.ts"), ts).unwrap();
        std::fs::write(std::path::Path::new(&output).join("client.rs"), rust).unwrap();
    }
}
