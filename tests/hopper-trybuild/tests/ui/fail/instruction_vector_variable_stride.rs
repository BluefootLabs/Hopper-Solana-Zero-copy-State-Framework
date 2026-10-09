use hopper::__macro_support::DecodeInstructionArg;
use hopper::hopper_runtime::BoundedVec;

const INVALID: usize = <BoundedVec<Option<u16>, 4> as DecodeInstructionArg>::WIRE_SIZE;

fn main() {
    let _ = INVALID;
}
