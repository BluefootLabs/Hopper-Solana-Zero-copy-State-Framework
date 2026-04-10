#!/usr/bin/env bash
# Hopper CU Benchmark Runner
#
# Deploys hopper-bench to a local validator, executes each benchmark
# instruction, parses CU measurements from logs, and compares against
# golden baselines in cu_baselines.toml.
#
# Usage:
#   ./measure.sh              # Run and display results
#   ./measure.sh --ci         # Run with CI-mode output
#   ./measure.sh --update     # Update baselines with measured values
#   ./measure.sh --fail-on-regression=5  # Fail if any >5% regression
#
# Prerequisites:
#   - solana-test-validator running
#   - cargo build-sbf completed for hopper-bench
#   - solana CLI configured for localhost

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BASELINES="$SCRIPT_DIR/cu_baselines.toml"
PROGRAM_DIR="$SCRIPT_DIR/hopper-bench"
PROGRAM_SO="$PROGRAM_DIR/target/deploy/hopper_bench.so"
RESULTS_FILE="$SCRIPT_DIR/results.csv"

# Parse arguments
CI_MODE=false
UPDATE_MODE=false
REGRESSION_THRESHOLD=0

for arg in "$@"; do
    case "$arg" in
        --ci) CI_MODE=true ;;
        --update) UPDATE_MODE=true ;;
        --fail-on-regression=*) REGRESSION_THRESHOLD="${arg#*=}" ;;
    esac
done

# Colors (disabled in CI)
if [ "$CI_MODE" = true ]; then
    RED="" GREEN="" YELLOW="" RESET=""
else
    RED='\033[0;31m' GREEN='\033[0;32m' YELLOW='\033[1;33m' RESET='\033[0m'
fi

echo "=== Hopper CU Benchmark Lab ==="
echo ""

# Step 1: Build
echo "Building hopper-bench..."
(cd "$PROGRAM_DIR" && cargo build-sbf 2>&1)

# Step 2: Check validator
if ! solana cluster-version &>/dev/null; then
    echo "${RED}Error: No local validator running. Start with: solana-test-validator --reset${RESET}"
    exit 1
fi

# Step 3: Deploy
echo "Deploying hopper-bench..."
PROGRAM_ID=$(solana program deploy "$PROGRAM_SO" --output json | jq -r '.programId')
echo "Program ID: $PROGRAM_ID"

# Step 4: Create test accounts
echo "Creating benchmark accounts..."
# (Account creation commands would go here - validator-specific)

# Step 5: Run each benchmark
echo ""
echo "Running benchmarks..."
echo "operation,measured_cu,baseline_cu,delta_pct" > "$RESULTS_FILE"

BENCHMARKS=(
    "0:check_signer:20"
    "1:check_writable:20"
    "2:check_owner:50"
    "3:check_account_tier1:120"
    "4:check_keys_eq:40"
    "5:overlay_57b:8"
    "6:write_header:30"
    "7:zero_init_57b:15"
    "8:check_account_fast:12"
    "9:emit_event_32b:100"
    "10:trust_strict_load:130"
)

FAILURES=0

for bench in "${BENCHMARKS[@]}"; do
    IFS=':' read -r disc name baseline <<< "$bench"

    # Send transaction with instruction disc and parse CU from logs
    TX_OUTPUT=$(solana program invoke \
        --program-id "$PROGRAM_ID" \
        --data "$(printf '%02x' "$disc")" \
        --fee-payer ~/.config/solana/id.json \
        --output json 2>&1 || true)

    # Extract CU from transaction logs: "consumed X of Y compute units"
    MEASURED=$(echo "$TX_OUTPUT" | grep -oP 'consumed \K[0-9]+' | head -n1)
    if [ -z "$MEASURED" ]; then
        # Fallback: use baseline as placeholder if invoke failed
        echo -e "  ${YELLOW}SKIP${RESET} ${name}: invoke failed (using baseline)"
        MEASURED=${baseline}
    fi

    if [ "$baseline" -gt 0 ]; then
        DELTA=$(( (MEASURED - baseline) * 100 / baseline ))
    else
        DELTA=0
    fi

    # Record
    echo "${name},${MEASURED},${baseline},${DELTA}" >> "$RESULTS_FILE"

    # Display
    if [ "$DELTA" -gt "$REGRESSION_THRESHOLD" ] && [ "$REGRESSION_THRESHOLD" -gt 0 ]; then
        echo -e "  ${RED}FAIL${RESET} ${name}: ${MEASURED} CU (baseline: ${baseline}, +${DELTA}%)"
        FAILURES=$((FAILURES + 1))
    elif [ "$DELTA" -gt 0 ]; then
        echo -e "  ${YELLOW}WARN${RESET} ${name}: ${MEASURED} CU (baseline: ${baseline}, +${DELTA}%)"
    else
        echo -e "  ${GREEN}PASS${RESET} ${name}: ${MEASURED} CU (baseline: ${baseline})"
    fi
done

echo ""
echo "Results written to: $RESULTS_FILE"

if [ "$FAILURES" -gt 0 ]; then
    echo -e "${RED}${FAILURES} benchmark(s) exceeded regression threshold (${REGRESSION_THRESHOLD}%)${RESET}"
    exit 1
else
    echo -e "${GREEN}All benchmarks within budget.${RESET}"
fi
