const CONFIG: (hopper::prelude::Address, u8) = hopper::canonical_pda!(
    "8RJxAyfAMnpb5ghwA4comPDJw6KqbDmZ28LDZHcccaVH", [b"config", b"v1"],
);
const EMPTY: (hopper::prelude::Address, u8) = hopper::canonical_pda!(
    "11111111111111111111111111111111", [],
);

fn main() {
    assert_eq!(CONFIG.0.as_array().len(), 32);
    assert_eq!(EMPTY.0.as_array().len(), 32);
}
