//! Is a 32-byte string a point of the ed25519 curve? In `const fn`, with no
//! dependency.
//!
//! A program-derived address is a SHA-256 output that is *not* a curve
//! point, so nobody holds a private key for it. On chain the runtime
//! decides that with the `sol_curve_validate_point` syscall. Off chain
//! there is no syscall, and before this module every derivation in
//! `hopper_native::pda` refused to run on the host. With the same decision
//! available as a `const fn`:
//!
//! - `find_program_address`, `create_program_address`, and the verify
//!   helpers run in plain unit tests and in off-chain tools, and return
//!   what the cluster returns;
//! - [`crate::pda::find_program_address_const`] searches for the canonical
//!   bump at compile time, from any `const` seeds.
//!
//! The decision is the one `curve25519-dalek`'s point decompression makes,
//! which is what the syscall runs: take `y` from the low 255 bits (a value
//! of `p` or more is reduced, not refused), form `u = y^2 - 1` and
//! `v = d*y^2 + 1`, and accept when `u/v` is a square in the field. The
//! sign bit does not take part.
//!
//! Field elements are five 51-bit limbs. The code favours being obviously
//! right over being fast: it runs in tests, tools, and the compiler, never
//! in a deployed program, where the syscall is cheaper.

/// An element of the field of integers modulo `2^255 - 19`.
type Fe = [u64; 5];

const MASK: u64 = (1 << 51) - 1;

/// `2p`, limb by limb, added before a subtraction so no limb goes negative.
const TWO_P: Fe = [
    0x000f_ffff_ffff_ffda,
    0x000f_ffff_ffff_fffe,
    0x000f_ffff_ffff_fffe,
    0x000f_ffff_ffff_fffe,
    0x000f_ffff_ffff_fffe,
];

const ONE: Fe = [1, 0, 0, 0, 0];

/// The curve constant `d = -121665/121666`, computed from its definition
/// at compile time.
const D: Fe = mul(neg(from_u64(121_665)), invert(from_u64(121_666)));

const fn from_u64(value: u64) -> Fe {
    carry([value & MASK, value >> 51, 0, 0, 0])
}

const fn load8(bytes: &[u8; 32], at: usize) -> u64 {
    (bytes[at] as u64)
        | (bytes[at + 1] as u64) << 8
        | (bytes[at + 2] as u64) << 16
        | (bytes[at + 3] as u64) << 24
        | (bytes[at + 4] as u64) << 32
        | (bytes[at + 5] as u64) << 40
        | (bytes[at + 6] as u64) << 48
        | (bytes[at + 7] as u64) << 56
}

/// The low 255 bits of `bytes`, little endian. Bit 255 (the sign of `x`
/// in a compressed point) is dropped.
const fn from_bytes(bytes: &[u8; 32]) -> Fe {
    [
        load8(bytes, 0) & MASK,
        (load8(bytes, 6) >> 3) & MASK,
        (load8(bytes, 12) >> 6) & MASK,
        (load8(bytes, 19) >> 1) & MASK,
        (load8(bytes, 24) >> 12) & MASK,
    ]
}

/// Propagate carries so every limb is below `2^51` plus a small excess.
const fn carry(mut h: Fe) -> Fe {
    let mut i = 0;
    while i < 4 {
        h[i + 1] += h[i] >> 51;
        h[i] &= MASK;
        i += 1;
    }
    h[0] += (h[4] >> 51) * 19;
    h[4] &= MASK;
    h[1] += h[0] >> 51;
    h[0] &= MASK;
    h
}

const fn add(a: Fe, b: Fe) -> Fe {
    carry([
        a[0] + b[0],
        a[1] + b[1],
        a[2] + b[2],
        a[3] + b[3],
        a[4] + b[4],
    ])
}

const fn sub(a: Fe, b: Fe) -> Fe {
    carry([
        a[0] + TWO_P[0] - b[0],
        a[1] + TWO_P[1] - b[1],
        a[2] + TWO_P[2] - b[2],
        a[3] + TWO_P[3] - b[3],
        a[4] + TWO_P[4] - b[4],
    ])
}

const fn neg(a: Fe) -> Fe {
    sub([0; 5], a)
}

const fn mul(a: Fe, b: Fe) -> Fe {
    let (a0, a1, a2, a3, a4) = (
        a[0] as u128,
        a[1] as u128,
        a[2] as u128,
        a[3] as u128,
        a[4] as u128,
    );
    let (b0, b1, b2, b3, b4) = (
        b[0] as u128,
        b[1] as u128,
        b[2] as u128,
        b[3] as u128,
        b[4] as u128,
    );
    // 2^255 = 19 (mod p), so a limb product that lands at 2^255 or above
    // folds back multiplied by 19.
    let (b1_19, b2_19, b3_19, b4_19) = (b1 * 19, b2 * 19, b3 * 19, b4 * 19);
    let c0 = a0 * b0 + a1 * b4_19 + a2 * b3_19 + a3 * b2_19 + a4 * b1_19;
    let mut c1 = a0 * b1 + a1 * b0 + a2 * b4_19 + a3 * b3_19 + a4 * b2_19;
    let mut c2 = a0 * b2 + a1 * b1 + a2 * b0 + a3 * b4_19 + a4 * b3_19;
    let mut c3 = a0 * b3 + a1 * b2 + a2 * b1 + a3 * b0 + a4 * b4_19;
    let mut c4 = a0 * b4 + a1 * b3 + a2 * b2 + a3 * b1 + a4 * b0;

    let mask = MASK as u128;
    c1 += c0 >> 51;
    c2 += c1 >> 51;
    c3 += c2 >> 51;
    c4 += c3 >> 51;
    let top = (c4 >> 51) as u64;
    let mut out = [
        (c0 & mask) as u64,
        (c1 & mask) as u64,
        (c2 & mask) as u64,
        (c3 & mask) as u64,
        (c4 & mask) as u64,
    ];
    out[0] += top * 19;
    out[1] += out[0] >> 51;
    out[0] &= MASK;
    out
}

const fn square(a: Fe) -> Fe {
    mul(a, a)
}

/// `a` raised to `exponent`, 255 bits, little-endian words.
const fn pow(a: Fe, exponent: [u64; 4]) -> Fe {
    let mut result = ONE;
    let mut bit = 255;
    while bit > 0 {
        bit -= 1;
        result = square(result);
        if (exponent[bit / 64] >> (bit % 64)) & 1 == 1 {
            result = mul(result, a);
        }
    }
    result
}

/// `p - 2`: by Fermat, `a^(p-2)` is the inverse of a nonzero `a`.
const P_MINUS_2: [u64; 4] = [
    0xffff_ffff_ffff_ffeb,
    0xffff_ffff_ffff_ffff,
    0xffff_ffff_ffff_ffff,
    0x7fff_ffff_ffff_ffff,
];

/// `(p - 5) / 8 = 2^252 - 3`.
const P_MINUS_5_OVER_8: [u64; 4] = [
    0xffff_ffff_ffff_fffd,
    0xffff_ffff_ffff_ffff,
    0xffff_ffff_ffff_ffff,
    0x0fff_ffff_ffff_ffff,
];

const fn invert(a: Fe) -> Fe {
    pow(a, P_MINUS_2)
}

/// The one representative of `h` in `0..p`.
const fn canonical(h: Fe) -> Fe {
    let mut h = carry(h);
    // `q` is 1 exactly when `h >= p`: adding 19 then carries out of bit 255.
    let mut q = (h[0] + 19) >> 51;
    q = (h[1] + q) >> 51;
    q = (h[2] + q) >> 51;
    q = (h[3] + q) >> 51;
    q = (h[4] + q) >> 51;
    h[0] += 19 * q;
    let mut i = 0;
    while i < 4 {
        h[i + 1] += h[i] >> 51;
        h[i] &= MASK;
        i += 1;
    }
    // Dropping the carry out of the top limb subtracts 2^255: together
    // with the 19 added above, that is the subtraction of `p`.
    h[4] &= MASK;
    h
}

const fn equal(a: Fe, b: Fe) -> bool {
    let (a, b) = (canonical(a), canonical(b));
    a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3] && a[4] == b[4]
}

/// Whether `bytes` decompresses to a point of the ed25519 curve: the
/// decision `sol_curve_validate_point` makes for the Edwards curve, and
/// the one that separates a keypair's address from a program-derived one.
pub const fn is_on_curve(bytes: &[u8; 32]) -> bool {
    let y = from_bytes(bytes);
    let yy = square(y);
    let u = sub(yy, ONE);
    let v = add(mul(yy, D), ONE);

    // r = u * v^3 * (u * v^7)^((p-5)/8) is a square root of u/v when one
    // exists, up to a factor of sqrt(-1): then v*r^2 is u or -u.
    let v3 = mul(square(v), v);
    let v7 = mul(square(v3), v);
    let r = mul(mul(u, v3), pow(mul(u, v7), P_MINUS_5_OVER_8));
    let check = mul(v, square(r));
    equal(check, u) || equal(check, neg(u))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    const fn to_bytes(h: Fe) -> [u8; 32] {
        let h = canonical(h);
        let mut out = [0u8; 32];
        let mut bit = 0;
        while bit < 255 {
            let limb = h[bit / 51];
            if (limb >> (bit % 51)) & 1 == 1 {
                out[bit / 8] |= 1 << (bit % 8);
            }
            bit += 1;
        }
        out
    }

    #[test]
    fn d_is_the_curve_constant() {
        // d, little endian, as every ed25519 reference prints it.
        let expected: [u8; 32] = [
            0xa3, 0x78, 0x59, 0x13, 0xca, 0x4d, 0xeb, 0x75, 0xab, 0xd8, 0x41, 0x41, 0x4d, 0x0a,
            0x70, 0x00, 0x98, 0xe8, 0x79, 0x77, 0x79, 0x40, 0xc7, 0x8c, 0x73, 0xfe, 0x6f, 0x2b,
            0xee, 0x6c, 0x03, 0x52,
        ];
        assert_eq!(to_bytes(D), expected);
        // And by its definition: d * 121666 = -121665.
        assert!(equal(mul(D, from_u64(121_666)), neg(from_u64(121_665))));
    }

    #[test]
    fn field_arithmetic_obeys_the_field_laws() {
        let mut seed = [7u8; 32];
        let mut elements = std::vec::Vec::new();
        for round in 0..24u8 {
            for (i, byte) in seed.iter_mut().enumerate() {
                *byte = byte.wrapping_mul(31).wrapping_add(round ^ i as u8);
            }
            elements.push(from_bytes(&seed));
        }
        // The edges: 0, 1, p - 1, and the non-canonical p and 2^255 - 1.
        elements.push([0; 5]);
        elements.push(ONE);
        elements.push(neg(ONE));
        elements.push([MASK - 18, MASK, MASK, MASK, MASK]);
        elements.push([MASK; 5]);

        for a in &elements {
            assert!(equal(add(*a, neg(*a)), [0; 5]));
            assert!(equal(sub(*a, *a), [0; 5]));
            assert!(equal(mul(*a, ONE), *a));
            if !equal(*a, [0; 5]) {
                assert!(equal(mul(*a, invert(*a)), ONE));
            }
            assert_eq!(from_bytes(&to_bytes(*a)), canonical(*a));
            for b in &elements {
                assert!(equal(mul(*a, *b), mul(*b, *a)));
                assert!(equal(sub(add(*a, *b), *b), *a));
                for c in elements.iter().take(4) {
                    assert!(equal(mul(*a, add(*b, *c)), add(mul(*a, *b), mul(*a, *c))));
                }
            }
        }
        // p itself is zero, and 2^255 - 1 is 18.
        assert!(equal([MASK - 18, MASK, MASK, MASK, MASK], [0; 5]));
        assert!(equal([MASK; 5], from_u64(18)));
    }

    #[test]
    fn known_points_and_non_points() {
        // The base point, the identity (y = 1), and y = -1 are points.
        let mut base = [0x66u8; 32];
        base[0] = 0x58;
        assert!(is_on_curve(&base));
        let mut identity = [0u8; 32];
        identity[0] = 1;
        assert!(is_on_curve(&identity));
        assert!(is_on_curve(&to_bytes(neg(ONE))));
        // The sign bit does not take part.
        base[31] |= 0x80;
        assert!(is_on_curve(&base));
        // The System Program's all-zero address is y = 0, a point.
        assert!(is_on_curve(&[0u8; 32]));
        // y = 2 is not: (4 - 1) / (4d + 1) is not a square.
        let mut two = [0u8; 32];
        two[0] = 2;
        assert!(!is_on_curve(&two));
    }

    #[test]
    fn the_decision_is_usable_in_a_const() {
        const BASE_IS_A_POINT: bool = {
            let mut base = [0x66u8; 32];
            base[0] = 0x58;
            is_on_curve(&base)
        };
        // Decided by the compiler: the build fails if the base point is not
        // on the curve.
        const { assert!(BASE_IS_A_POINT) };
    }
}
