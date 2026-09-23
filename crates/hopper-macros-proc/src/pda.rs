//! Explicit literal inputs keep expansion independent of filesystem discovery,
//! environment-selected program IDs, and untracked source files.

use curve25519_dalek::edwards::CompressedEdwardsY;
use proc_macro2::TokenStream;
use quote::quote;
use sha2::{Digest, Sha256};
use syn::{bracketed, parse::Parse, parse::ParseStream, LitByteStr, LitStr, Token};

struct LiteralPda {
    program: LitStr,
    seeds: Vec<LitByteStr>,
}

impl Parse for LiteralPda {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let program = input.parse()?;
        input.parse::<Token![,]>()?;
        let content;
        bracketed!(content in input);
        let seeds = content
            .parse_terminated(LitByteStr::parse, Token![,])?
            .into_iter()
            .collect();
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
        Ok(Self { program, seeds })
    }
}

fn derive(input: &LiteralPda) -> syn::Result<([u8; 32], u8)> {
    let invalid_program =
        || syn::Error::new(input.program.span(), "expected a 32-byte base58 program ID");
    let program = bs58::decode(input.program.value())
        .into_vec()
        .map_err(|_| invalid_program())?;
    let program: [u8; 32] = program.try_into().map_err(|_| invalid_program())?;
    if input.seeds.len() >= 16 {
        return Err(syn::Error::new(
            input.seeds[15].span(),
            "canonical PDA accepts at most 15 base seeds; the bump uses the final slot",
        ));
    }
    let mut seeds = Vec::with_capacity(input.seeds.len());
    for seed in &input.seeds {
        let bytes = seed.value();
        if bytes.len() > 32 {
            return Err(syn::Error::new(seed.span(), "PDA seed exceeds 32 bytes"));
        }
        seeds.push(bytes);
    }
    // Solana's signing domain and canonical descending bump search. This
    // host-only path uses the same SHA-256 and Edwards decompression primitives
    // as the SDK; differential tests below guard byte-for-byte compatibility.
    for bump in (0..=255u8).rev() {
        let mut hash = Sha256::new();
        for seed in &seeds {
            hash.update(seed);
        }
        hash.update([bump]);
        hash.update(program);
        hash.update(b"ProgramDerivedAddress");
        let bytes: [u8; 32] = hash.finalize().into();
        if CompressedEdwardsY(bytes).decompress().is_none() {
            return Ok((bytes, bump));
        }
    }
    Err(syn::Error::new(
        input.program.span(),
        "no off-curve PDA bump exists",
    ))
}

pub(crate) fn expand(tokens: TokenStream) -> syn::Result<TokenStream> {
    let input = syn::parse2::<LiteralPda>(tokens)?;
    let (bytes, bump) = derive(&input)?;
    Ok(quote! { (::hopper::prelude::Address::new([#(#bytes),*]), #bump) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_address::Address;
    use std::str::FromStr;

    #[test]
    fn canonical_output_matches_sdk_and_has_no_runtime_calls() {
        let input: LiteralPda = syn::parse_quote!(
            "F4Um7PWsnZfN7y8WFzu1aPYJwqGduJTa4zuCGY9EUqMy",
            [b"config", b"v1"]
        );
        let program = Address::from_str(&input.program.value()).unwrap();
        let (bytes, bump) = derive(&input).unwrap();
        let expected = Address::find_program_address(&[b"config", b"v1"], &program);
        assert_eq!((bytes, bump), (expected.0.to_bytes(), expected.1));
        let output = expand(quote!("11111111111111111111111111111111", [])).unwrap();
        assert!(!output.to_string().contains("find_program_address"));
        assert!(!output.to_string().contains("const_program_address"));
    }

    #[test]
    fn seed_boundaries_include_the_bump() {
        let mut input: LiteralPda = syn::parse_quote!("11111111111111111111111111111111", []);
        let full = LitByteStr::new(&[7; 32], input.program.span());
        input.seeds = vec![full; 15];
        assert!(derive(&input).is_ok());
        input.seeds.push(LitByteStr::new(b"", input.program.span()));
        assert!(derive(&input)
            .unwrap_err()
            .to_string()
            .contains("15 base seeds"));
        input.seeds = vec![LitByteStr::new(&[7; 33], input.program.span())];
        assert!(derive(&input).unwrap_err().to_string().contains("32 bytes"));
    }

    #[test]
    fn canonical_search_matches_sdk_across_keys_and_bumps() {
        for key in 0..64u8 {
            let program = [key; 32];
            let literal = LitStr::new(
                &bs58::encode(program).into_string(),
                proc_macro2::Span::call_site(),
            );
            for seed_len in [0, 1, 16, 32] {
                let seed = vec![key.wrapping_add(1); seed_len];
                let input = LiteralPda {
                    program: literal.clone(),
                    seeds: vec![LitByteStr::new(&seed, literal.span())],
                };
                let expected =
                    Address::find_program_address(&[&seed], &Address::new_from_array(program));
                assert_eq!(derive(&input).unwrap(), (expected.0.to_bytes(), expected.1));
            }
        }
    }

    #[test]
    fn rejects_dynamic_inputs_invalid_ids_and_trailing_tokens() {
        for tokens in [
            quote!(ID, [b"config"]),
            quote!("11111111111111111111111111111111", [payer.as_ref()]),
            quote!("11111111111111111111111111111111", ["text"]),
            quote!("not-a-public-key", [b"config"]),
            quote!("11111111111111111111111111111111", [], 255),
        ] {
            assert!(expand(tokens).is_err());
        }
    }
}
