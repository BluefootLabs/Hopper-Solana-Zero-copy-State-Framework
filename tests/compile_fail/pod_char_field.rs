//! `char` is not Pod: only Unicode scalar values are valid (surrogate
//! pair code points are invalid). Must be rejected at the field-level
//! Pod proof.

use hopper::pod;

#[pod]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct BadChar {
    pub c: char,
}

fn main() {}
