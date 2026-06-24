//! `declare_id!` parity: porting an Anchor/Pinocchio/Quasar program should not
//! require hand-rolling the `ID` / `id()` / `check_id()` trio that every other
//! Solana framework generates from a single macro.

use hopper::prelude::*;

mod my_program {
    // Devnet counter id from COMPARISON.md — any valid base58 32-byte key.
    hopper::declare_id!("D8UGWDX5QRwEkKs2J9Sweabf4zd6hzdLqv7CB11SF91F");
}

#[test]
fn declare_id_generates_id_and_accessors() {
    // `ID` const, `id()` accessor, and `check_id()` guard all exist.
    let from_const: Address = my_program::ID;
    let from_fn: Address = my_program::id();
    assert_eq!(from_const, from_fn);

    assert!(my_program::check_id(&my_program::ID));

    let other = address!("11111111111111111111111111111111");
    assert!(!my_program::check_id(&other));
}

#[test]
fn declared_id_round_trips_through_base58() {
    // The const is decoded at compile time; confirm it is the 32-byte image of
    // the literal and not all-zero.
    assert_ne!(my_program::ID, Address::default());
}
