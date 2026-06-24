//! Const SHA-256 implementation used for Hopper-owned discriminators.

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Compute SHA-256(data) at compile time.
pub const fn sha256(data: &[u8]) -> [u8; 32] {
    sha256_concat(data, &[])
}

/// Compute SHA-256(left || right) at compile time without allocating.
pub const fn sha256_concat(left: &[u8], right: &[u8]) -> [u8; 32] {
    let mut state = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let len = left.len() + right.len();
    let full_blocks = len / 64;
    let mut block_index = 0;
    while block_index < full_blocks {
        let mut block = [0u8; 64];
        let mut i = 0;
        while i < 64 {
            block[i] = concat_byte(left, right, block_index * 64 + i);
            i += 1;
        }
        state = compress(state, block);
        block_index += 1;
    }

    let rem = len % 64;
    let mut block = [0u8; 64];
    let mut i = 0;
    while i < rem {
        block[i] = concat_byte(left, right, full_blocks * 64 + i);
        i += 1;
    }
    block[rem] = 0x80;
    let bit_len = (len as u64).wrapping_mul(8);
    if rem <= 55 {
        write_len(&mut block, bit_len);
        state = compress(state, block);
    } else {
        state = compress(state, block);
        let mut last = [0u8; 64];
        write_len(&mut last, bit_len);
        state = compress(state, last);
    }

    state_to_bytes(state)
}

/// Compute SHA-256(left || right)[0..8].
pub const fn sha256_prefix8(left: &[u8], right: &[u8]) -> [u8; 8] {
    let hash = sha256_concat(left, right);
    [
        hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7],
    ]
}

const fn concat_byte(left: &[u8], right: &[u8], index: usize) -> u8 {
    if index < left.len() {
        left[index]
    } else {
        right[index - left.len()]
    }
}

const fn write_len(block: &mut [u8; 64], bit_len: u64) {
    let bytes = bit_len.to_be_bytes();
    let mut i = 0;
    while i < 8 {
        block[56 + i] = bytes[i];
        i += 1;
    }
}

const fn compress(mut state: [u32; 8], block: [u8; 64]) -> [u32; 8] {
    let mut w = [0u32; 64];
    let mut i = 0;
    while i < 16 {
        let j = i * 4;
        w[i] = u32::from_be_bytes([block[j], block[j + 1], block[j + 2], block[j + 3]]);
        i += 1;
    }
    while i < 64 {
        let s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
        let s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
        i += 1;
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];
    i = 0;
    while i < 64 {
        let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
        i += 1;
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
    state
}

const fn rotr(value: u32, by: u32) -> u32 {
    value.rotate_right(by)
}

const fn state_to_bytes(state: [u32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 8 {
        let bytes = state[i].to_be_bytes();
        out[i * 4] = bytes[0];
        out[i * 4 + 1] = bytes[1];
        out[i * 4 + 2] = bytes[2];
        out[i * 4 + 3] = bytes[3];
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_empty_matches_known_vector() {
        assert_eq!(
            sha256(b""),
            [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
                0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
                0x78, 0x52, 0xb8, 0x55,
            ]
        );
    }

    #[test]
    fn sha256_abc_matches_known_vector() {
        assert_eq!(
            sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }

    #[test]
    fn concat_matches_single_slice() {
        assert_eq!(
            sha256_concat(b"global:", b"initialize"),
            sha256(b"global:initialize")
        );
    }
}
