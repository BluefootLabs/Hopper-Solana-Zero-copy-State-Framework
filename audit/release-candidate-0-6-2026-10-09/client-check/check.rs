
extern crate self as solana_program;
pub mod instruction { pub use solana_instruction::*; }
pub mod pubkey { pub use solana_pubkey::*; }
#[path = "client.rs"] mod client;
#[cfg(test)] mod tests {
    use super::{client::*, pubkey::Pubkey};
    #[test] fn wire_contract_and_bounds() {
        let mut first = [0; 10]; first[0..2].copy_from_slice(&[1, 1]); first[2..].copy_from_slice(&42u64.to_le_bytes());
        let mut second = [0; 10]; second[0..2].copy_from_slice(&[1, 7]); second[2..].copy_from_slice(&99u64.to_le_bytes());
        let args = SubmitArgs { orders: vec![first, second], nonce: 513 };
        let mut expected = vec![0, 2, 0]; expected.extend_from_slice(&first); expected.extend_from_slice(&second); expected.extend_from_slice(&[1, 2]);
        assert_eq!(encode_submit_data(&args).unwrap(), expected);
        let decoded = decode_submit_args(&expected).unwrap(); assert_eq!(decoded.orders, args.orders); assert_eq!(decoded.nonce, 513);
        for end in 0..expected.len() { assert!(decode_submit_args(&expected[..end]).is_err()); }
        let mut extra = expected.clone(); extra.push(0); assert!(decode_submit_args(&extra).is_err());
        let mut excessive = args.clone(); excessive.orders.resize(5, first); assert!(encode_submit_data(&excessive).is_err());
        excessive.orders.clear(); assert_eq!(encode_submit_data(&excessive).unwrap(), [0,0,0,1,2]);
        excessive.orders.resize(4, first); assert_eq!(decode_submit_args(&encode_submit_data(&excessive).unwrap()).unwrap().orders.len(), 4);
        for count in [3u16, 5, 65535] { let mut bad = expected.clone(); bad[1..3].copy_from_slice(&count.to_le_bytes()); assert!(decode_submit_args(&bad).is_err()); }
        let program = Pubkey::new_from_array([9;32]); let authority = Pubkey::new_from_array([8;32]);
        let ix = submit_ix(&program, &SubmitAccounts { authority }, &args).unwrap();
        assert_eq!(ix.data, expected); assert_eq!(ix.program_id, program); assert_eq!(ix.accounts[0].pubkey, authority); assert!(ix.accounts[0].is_signer); assert!(!ix.accounts[0].is_writable);
        let mixed = BytesArgs { bytes: vec![[42], [99]], note: "ok".into() };
        assert_eq!(encode_bytes_data(&mixed).unwrap(), [1,2,0,42,99,2,0,111,107]);
        assert_eq!(decode_bytes_args(&[1,2,0,42,99,2,0,111,107]).unwrap().note, "ok");
        assert!(decode_bytes_args(&[1,0,0,1,0,255]).is_err());
        assert!(encode_bytes_data(&BytesArgs { bytes: vec![], note: "é".repeat(9) }).is_err());
    }
}
