//! Crank marker type emitted by the `#[hopper::crank]` attribute.
//!
//! A `CrankMarker` is a compile-time const that sits next to the
//! decorated handler. It records the handler's name and an optional
//! set of seed hints used by `hopper manager crank run` to resolve
//! PDA accounts autonomously.
//!
//! Indexers and off-chain tools walk the program's emitted `const`
//! items, collect every `CrankMarker`, and surface the resulting set
//! as "this program's crankable instruction list". Having the data
//! stamped in the binary (rather than only in the off-chain
//! manifest) keeps the ground truth on chain.

/// Compile-time descriptor for a crank handler.
///
/// `seed_hints` is a slice of `(account_field_name, seed_bytes_list)`
/// tuples. Each entry says: "the account field named X on this
/// crank's context can be derived from these seeds under the
/// program's id". The crank runner calls `find_program_address`
/// with the seeds and fills the account slot automatically.
#[derive(Copy, Clone, Debug)]
pub struct CrankMarker {
    /// The handler function name, same as `stringify!(fn_name)`.
    pub handler_name: &'static str,
    /// Per-account seed hints. Each tuple is
    /// `(field_name, &[seed_0, seed_1, ...])`.
    pub seed_hints: &'static [(&'static str, &'static [&'static [u8]])],
}

impl CrankMarker {
    /// Number of declared seed hints.
    #[inline(always)]
    pub const fn seed_count(&self) -> usize {
        self.seed_hints.len()
    }

    /// Lookup a single field's seed list by name. Returns `None`
    /// when the field has no declared hint (the runner then falls
    /// back to `--account` supplied at the CLI).
    #[inline]
    pub fn seeds_for(&self, field: &str) -> Option<&'static [&'static [u8]]> {
        let mut i = 0;
        while i < self.seed_hints.len() {
            if str_eq(self.seed_hints[i].0, field) {
                return Some(self.seed_hints[i].1);
            }
            i += 1;
        }
        None
    }
}

/// Const-context `&str` equality. The stdlib's `str::eq` is not
/// const-callable on stable, so we inline the byte-wise compare.
#[inline]
const fn str_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut i = 0;
    while i < ab.len() {
        if ab[i] != bb[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: CrankMarker = CrankMarker {
        handler_name: "settle",
        seed_hints: &[],
    };

    const WITH_HINTS: CrankMarker = CrankMarker {
        handler_name: "rotate",
        seed_hints: &[
            ("vault", &[b"vault".as_slice()] as &[&[u8]]),
            (
                "fee_account",
                &[b"fee".as_slice(), b"account".as_slice()] as &[&[u8]],
            ),
        ],
    };

    #[test]
    fn empty_marker_has_zero_hints() {
        assert_eq!(EMPTY.seed_count(), 0);
        assert!(EMPTY.seeds_for("anything").is_none());
    }

    #[test]
    fn lookup_resolves_named_field() {
        let s = WITH_HINTS.seeds_for("vault").unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0], b"vault");

        let s = WITH_HINTS.seeds_for("fee_account").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], b"fee");
        assert_eq!(s[1], b"account");
    }

    #[test]
    fn lookup_miss_returns_none() {
        assert!(WITH_HINTS.seeds_for("not_declared").is_none());
    }
}
