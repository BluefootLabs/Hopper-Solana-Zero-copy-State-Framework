use hopper::__macro_support::DecodeInstructionArg;
use hopper::hopper_runtime::BoundedString;

const INVALID: usize = <BoundedString<65534> as DecodeInstructionArg>::WIRE_SIZE;

fn main() {
    let _ = INVALID;
}
