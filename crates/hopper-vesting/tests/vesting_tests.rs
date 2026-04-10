#[cfg(test)]
mod vesting_tests {
    use hopper_vesting::*;

    const TOTAL: u64 = 1_000_000;
    const START: i64 = 1_000;
    const CLIFF: i64 = 2_000;
    const END: i64 = 5_000;

    #[test]
    fn before_cliff_returns_zero() {
        assert_eq!(vested_amount(TOTAL, START, CLIFF, END, 500), 0);
        assert_eq!(vested_amount(TOTAL, START, CLIFF, END, 1_999), 0);
    }

    #[test]
    fn at_cliff_returns_proportional() {
        let v = vested_amount(TOTAL, START, CLIFF, END, CLIFF);
        // elapsed = 1000, duration = 4000 => 25%
        assert_eq!(v, 250_000);
    }

    #[test]
    fn at_end_returns_total() {
        assert_eq!(vested_amount(TOTAL, START, CLIFF, END, END), TOTAL);
        assert_eq!(vested_amount(TOTAL, START, CLIFF, END, END + 1000), TOTAL);
    }

    #[test]
    fn midpoint_linear() {
        let mid = (START + END) / 2; // 3000
        let v = vested_amount(TOTAL, START, CLIFF, END, mid);
        // elapsed = 2000, duration = 4000 => 50%
        assert_eq!(v, 500_000);
    }

    #[test]
    fn zero_duration_returns_total() {
        assert_eq!(vested_amount(TOTAL, START, START, START, START), TOTAL);
    }

    #[test]
    fn cliff_check() {
        assert!(check_cliff_reached(CLIFF, CLIFF).is_ok());
        assert!(check_cliff_reached(CLIFF, CLIFF + 1).is_ok());
        assert!(check_cliff_reached(CLIFF, CLIFF - 1).is_err());
    }

    #[test]
    fn stepped_unlock() {
        // 12 monthly steps
        assert_eq!(unlocked_at_step(1_200, 12, 0), 0);
        assert_eq!(unlocked_at_step(1_200, 12, 6), 600);
        assert_eq!(unlocked_at_step(1_200, 12, 12), 1_200);
        assert_eq!(unlocked_at_step(1_200, 12, 100), 1_200); // past end
    }

    #[test]
    fn stepped_zero_steps_returns_total() {
        assert_eq!(unlocked_at_step(1_000, 0, 5), 1_000);
    }

    #[test]
    fn claimable_subtraction() {
        assert_eq!(claimable(500_000, 200_000), 300_000);
        assert_eq!(claimable(500_000, 500_000), 0);
        assert_eq!(claimable(500_000, 999_999), 0); // saturates
    }

    #[test]
    fn elapsed_steps_computation() {
        let month = 30 * 86_400i64;
        assert_eq!(elapsed_steps(0, 0, month), 0);
        assert_eq!(elapsed_steps(0, month, month), 1);
        assert_eq!(elapsed_steps(0, month * 6, month), 6);
        // Before start
        assert_eq!(elapsed_steps(100, 50, month), 0);
        // Zero step_duration
        assert_eq!(elapsed_steps(0, 100, 0), 0);
    }

    #[test]
    fn full_vesting_cycle() {
        let total = 10_000_000u64;
        let start = 1_700_000_000i64;
        let cliff = start + 365 * 86_400; // 1 year cliff
        let end = start + 4 * 365 * 86_400; // 4 year vesting

        // Before cliff
        let now = start + 100 * 86_400;
        assert_eq!(vested_amount(total, start, cliff, end, now), 0);

        // At cliff (25% elapsed)
        let v = vested_amount(total, start, cliff, end, cliff);
        assert_eq!(v, 2_500_000);

        // 2 years in (50%)
        let v = vested_amount(total, start, cliff, end, start + 2 * 365 * 86_400);
        assert_eq!(v, 5_000_000);

        // Claim half of what's vested
        let claimed = 1_250_000u64;
        assert_eq!(claimable(v, claimed), 3_750_000);
    }
}
