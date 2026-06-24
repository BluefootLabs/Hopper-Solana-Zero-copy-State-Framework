//! Promotion test: the Instructions-sysvar introspection surface is reachable
//! from `hopper::sysvar` (alongside Clock/Rent/EpochRewards) and from
//! `hopper::systems`, giving parity with Pinocchio `Instructions<T>` / Quasar
//! introspection without descending into `hopper::hopper_core::check`.

/// Minimal Instructions-sysvar account image: `u16 num_ix`, a `u16` offset
/// table, then per instruction `u16 num_metas`, metas (`u8 flags` + 32-byte
/// pubkey), the 32-byte program id, and `u16 data_len`; trailing `u16` is the
/// current instruction index.
fn build_ix_sysvar(instructions: &[(&[u8; 32], usize)], current_idx: u16) -> Vec<u8> {
    let num_ix = instructions.len() as u16;
    let mut buf = Vec::new();
    buf.extend_from_slice(&num_ix.to_le_bytes());

    let offset_table_start = buf.len();
    for _ in 0..num_ix {
        buf.extend_from_slice(&0u16.to_le_bytes());
    }

    let mut offsets = Vec::new();
    for &(program_id, num_metas) in instructions {
        offsets.push(buf.len() as u16);
        buf.extend_from_slice(&(num_metas as u16).to_le_bytes());
        for m in 0..num_metas {
            let flags: u8 = if m == 0 { 0x03 } else { 0x02 };
            buf.push(flags);
            let mut key = [0u8; 32];
            key[0] = m as u8;
            buf.extend_from_slice(&key);
        }
        buf.extend_from_slice(program_id);
        buf.extend_from_slice(&0u16.to_le_bytes());
    }

    for (i, offset) in offsets.iter().enumerate() {
        let pos = offset_table_start + i * 2;
        let bytes = offset.to_le_bytes();
        buf[pos] = bytes[0];
        buf[pos + 1] = bytes[1];
    }

    buf.extend_from_slice(&current_idx.to_le_bytes());
    buf
}

#[test]
fn instructions_sysvar_reader_is_reachable_from_hopper_sysvar() {
    use hopper::sysvar::InstructionsSysvar;

    let program_a = [0x11u8; 32];
    let program_b = [0x22u8; 32];
    let data = build_ix_sysvar(&[(&program_a, 2), (&program_b, 0)], 0);

    let view = InstructionsSysvar::new(&data);
    assert_eq!(view.len().unwrap(), 2);
    assert_eq!(view.current_index().unwrap(), 0);
    assert_eq!(view.program_id_at(1).unwrap().as_array(), &program_b);

    let current = view.current_instruction().unwrap();
    assert_eq!(current.account_count(), 2);
    assert_eq!(current.program_id().as_array(), &program_a);

    let meta0 = current.account(0).unwrap().unwrap();
    assert!(meta0.is_signer());
    assert!(meta0.is_writable());
}

#[test]
fn instructions_sysvar_free_helpers_are_reachable_from_hopper_sysvar() {
    use hopper::sysvar::{current_instruction_index, instruction_count, read_program_id_at};

    let p0 = [10u8; 32];
    let p1 = [20u8; 32];
    let data = build_ix_sysvar(&[(&p0, 1), (&p1, 0)], 1);

    assert_eq!(instruction_count(&data).unwrap(), 2);
    assert_eq!(current_instruction_index(&data).unwrap(), 1);
    assert_eq!(read_program_id_at(&data, 0).unwrap(), p0);
    assert_eq!(read_program_id_at(&data, 1).unwrap(), p1);
}

#[test]
fn instructions_sysvar_reader_is_also_reachable_from_hopper_systems() {
    // The advanced surface re-exports the same types via `prelude_advanced`.
    use hopper::systems::{InstructionAccountMeta, InstructionsSysvar, IntrospectedInstruction};

    let p0 = [7u8; 32];
    let data = build_ix_sysvar(&[(&p0, 1)], 0);
    let view: InstructionsSysvar = InstructionsSysvar::new(&data);
    let ix: IntrospectedInstruction = view.current_instruction().unwrap();
    let meta: InstructionAccountMeta = ix.account(0).unwrap().unwrap();
    assert!(meta.is_writable());
}
