#!/usr/bin/env bash
# bench/run-bench-docker.sh
#
# Run the Hopper primitive benchmark lab against a Docker-managed Solana
# test validator.
#
# Usage:
#   ./bench/run-bench-docker.sh
#   ./bench/run-bench-docker.sh --no-build --out-dir bench/results
#
# Extra arguments are forwarded verbatim to `hopper profile bench`.
#
# Environment overrides:
#   SOLANA_IMAGE   : Docker image for the test validator
#                    (default: solanalabs/solana:v2.1.21)
#   SOLANA_RPC_URL : Override RPC endpoint entirely (validator still started)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
COMPOSE="$SCRIPT_DIR/docker/docker-compose.yml"
FIXTURES_DIR="$SCRIPT_DIR/fixtures"
KEYPAIR_PATH="$FIXTURES_DIR/bench-keypair.json"
RPC_URL="${SOLANA_RPC_URL:-http://127.0.0.1:8899}"
VALIDATOR_READY_TIMEOUT=60   # seconds

# ── Prerequisites ──────────────────────────────────────────────────────────────

if ! command -v docker &>/dev/null; then
    echo "error: docker is not installed or not on PATH." >&2
    exit 1
fi

# ── Bench keypair ──────────────────────────────────────────────────────────────

if [[ ! -f "$KEYPAIR_PATH" ]]; then
    echo "Generating bench keypair at $KEYPAIR_PATH ..."
    mkdir -p "$FIXTURES_DIR"

    if ! command -v solana-keygen &>/dev/null; then
        echo "error: solana-keygen is not on PATH. Install the Solana CLI toolchain." >&2
        exit 1
    fi

    solana-keygen new --no-bip39-passphrase --outfile "$KEYPAIR_PATH"
fi

# ── Start validator ────────────────────────────────────────────────────────────

echo "Starting Solana test validator (Docker Compose) ..."
docker compose -f "$COMPOSE" up -d validator

# Always stop the validator on exit (success, failure, or Ctrl-C).
cleanup() {
    echo "Stopping Solana test validator ..."
    docker compose -f "$COMPOSE" down
}
trap cleanup EXIT

# ── Wait for healthy ───────────────────────────────────────────────────────────

echo "Waiting for validator at $RPC_URL ..."
deadline=$(( $(date +%s) + VALIDATOR_READY_TIMEOUT ))
ready=0

while [[ $(date +%s) -lt $deadline ]]; do
    if curl -sf "$RPC_URL/health" 2>/dev/null | grep -q "ok"; then
        ready=1
        break
    fi
    sleep 0.5
done

if [[ $ready -eq 0 ]]; then
    echo "error: Solana test validator did not become healthy within ${VALIDATOR_READY_TIMEOUT}s." >&2
    exit 1
fi

echo "Validator ready."

# ── Run benchmark ──────────────────────────────────────────────────────────────

cd "$ROOT_DIR"
cargo run -p hopper-cli -- profile bench \
    --rpc     "$RPC_URL"     \
    --keypair "$KEYPAIR_PATH" \
    "$@"
