#[cfg(test)]
mod distribute_tests {
    use hopper_distribute::*;

    #[test]
    fn even_split() {
        let mut out = [0u64; 3];
        proportional_split(900, &[1, 1, 1], &mut out).unwrap();
        assert_eq!(out, [300, 300, 300]);
    }

    #[test]
    fn weighted_split() {
        let mut out = [0u64; 3];
        proportional_split(1_000, &[50, 30, 20], &mut out).unwrap();
        assert_eq!(out[0] + out[1] + out[2], 1_000);
        assert_eq!(out[0], 500);
        assert_eq!(out[1], 300);
        assert_eq!(out[2], 200);
    }

    #[test]
    fn dust_is_distributed() {
        // 1_000_003 / 3 leaves remainder of 1 (or similar)
        let mut out = [0u64; 3];
        proportional_split(1_000_003, &[1, 1, 1], &mut out).unwrap();
        assert_eq!(out[0] + out[1] + out[2], 1_000_003);
    }

    #[test]
    fn largest_fractional_remainders_receive_dust() {
        // Exact quotas are 2/7, 6/7, and 6/7. An index-first round-robin
        // implementation would incorrectly produce [1, 1, 0].
        let mut out = [0u64; 3];
        proportional_split(2, &[1, 3, 3], &mut out).unwrap();
        assert_eq!(out, [0, 1, 1]);
    }

    #[test]
    fn equal_remainders_use_input_order() {
        let mut out = [0u64; 3];
        proportional_split(2, &[1, 1, 1], &mut out).unwrap();
        assert_eq!(out, [1, 1, 0]);
    }

    #[test]
    fn single_recipient() {
        let mut out = [0u64; 1];
        proportional_split(999, &[100], &mut out).unwrap();
        assert_eq!(out[0], 999);
    }

    #[test]
    fn prime_total_no_dust_lost() {
        let total = 1_000_003u64;
        let shares = [50u64, 30, 20];
        let mut out = [0u64; 3];
        proportional_split(total, &shares, &mut out).unwrap();
        assert_eq!(out[0] + out[1] + out[2], total);
    }

    #[test]
    fn zero_total() {
        let mut out = [0u64; 2];
        proportional_split(0, &[1, 1], &mut out).unwrap();
        assert_eq!(out, [0, 0]);
    }

    #[test]
    fn rejects_empty_shares() {
        let mut out = [0u64; 0];
        assert!(proportional_split(100, &[], &mut out).is_err());
    }

    #[test]
    fn rejects_all_zero_shares() {
        let mut out = [0u64; 2];
        assert!(proportional_split(100, &[0, 0], &mut out).is_err());
    }

    /// Straightforward largest-remainder reference: sort by remainder
    /// descending, ties by input order, award one unit to the first
    /// `total - sum(floor)` recipients.
    fn reference_split(total: u64, shares: &[u64]) -> Vec<u64> {
        let sum: u128 = shares.iter().map(|&s| s as u128).sum();
        let mut out: Vec<u64> = shares
            .iter()
            .map(|&s| (s as u128 * total as u128 / sum) as u64)
            .collect();
        let left = total - out.iter().sum::<u64>();
        let mut order: Vec<usize> = (0..shares.len()).collect();
        order.sort_by_key(|&i| {
            (
                std::cmp::Reverse(shares[i] as u128 * total as u128 % sum),
                i,
            )
        });
        for &i in order.iter().take(left as usize) {
            out[i] += 1;
        }
        out
    }

    #[test]
    fn matches_the_sorting_reference_including_extremes() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut cases: Vec<(u64, Vec<u64>)> = vec![
            (u64::MAX, vec![u64::MAX, u64::MAX, 1]),
            (u64::MAX, vec![u64::MAX; 7]),
            (u64::MAX - 1, vec![3, 3, 3, 1]),
            (7, vec![1; 40]),
        ];
        for _ in 0..300 {
            let n = (next() % 24 + 1) as usize;
            let big = next() % 2 == 0;
            let shares: Vec<u64> = (0..n)
                .map(|_| if big { next() } else { next() % 1_000 })
                .collect();
            let total = if next() % 2 == 0 {
                next()
            } else {
                next() % 10_000
            };
            if shares.iter().any(|&s| s != 0) {
                cases.push((total, shares));
            }
        }
        for (total, shares) in cases {
            let mut out = vec![0u64; shares.len()];
            proportional_split(total, &shares, &mut out).unwrap();
            assert_eq!(out, reference_split(total, &shares), "{total} {shares:?}");
            assert_eq!(out.iter().map(|&v| v as u128).sum::<u128>(), total as u128);
        }
    }

    #[test]
    fn rejects_length_mismatch() {
        let mut out = [0u64; 2];
        assert!(proportional_split(100, &[1, 1, 1], &mut out).is_err());
    }

    #[test]
    fn extract_fee_basic() {
        let (net, fee) = extract_fee(1_000_000, 30, 0).unwrap();
        assert_eq!(net + fee, 1_000_000);
        // ceil(1_000_000 * 30 / 10_000) = ceil(3000) = 3000
        assert_eq!(fee, 3_000);
    }

    #[test]
    fn extract_fee_with_flat() {
        let (net, fee) = extract_fee(1_000_000, 30, 1_000).unwrap();
        assert_eq!(net + fee, 1_000_000);
        assert_eq!(fee, 4_000); // 3000 bps + 1000 flat
    }

    #[test]
    fn extract_fee_ceil_rounding() {
        // 7 * 100 / 10_000 = 0.07 -> ceil = 1
        let (net, fee) = extract_fee(7, 100, 0).unwrap();
        assert_eq!(net + fee, 7);
        assert_eq!(fee, 1);
    }

    #[test]
    fn extract_fee_zero_bps() {
        let (net, fee) = extract_fee(1_000, 0, 0).unwrap();
        assert_eq!(net, 1_000);
        assert_eq!(fee, 0);
    }

    #[test]
    fn extract_fee_rejects_excessive() {
        // fee_bps = 10_000 (100%) + flat = 1 => exceeds amount
        assert!(extract_fee(1_000, 10_000, 1).is_err());
    }

    #[test]
    fn invariant_net_plus_fee_equals_amount() {
        for amount in [1, 7, 100, 999, 1_000_000, u64::MAX / 10_000] {
            for bps in [0, 1, 30, 100, 500, 9_999] {
                if let Ok((net, fee)) = extract_fee(amount, bps, 0) {
                    assert_eq!(net + fee, amount, "amount={amount} bps={bps}");
                }
            }
        }
    }
}
