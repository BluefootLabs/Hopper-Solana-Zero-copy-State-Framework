//! `hopper_unsafe_region!` from a program's side: the labelled body runs
//! as `unsafe` and the region evaluates to the body's value.

#[test]
fn a_labelled_unsafe_region_evaluates_to_its_body() {
    let bytes = [1u8, 2, 3, 4];
    // SAFETY: index 2 is inside the four-byte array.
    let third = hopper_runtime::hopper_unsafe_region!("read one byte inside the array", {
        *bytes.as_ptr().add(2)
    });
    assert_eq!(third, 3);
}
