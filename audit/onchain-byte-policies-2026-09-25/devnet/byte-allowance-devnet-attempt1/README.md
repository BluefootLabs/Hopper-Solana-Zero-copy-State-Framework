# Earlier allowance harness funding attempt

The first harness attempt stopped at a CLI readonly-account flag error before
the allowance application cases. Its two successful System Program funding
transfers are preserved here: 2,000,000 lamports of delegate funding plus 10,000
lamports of transaction fees. The transaction payloads, logs, and complete
expected/before/after account snapshots agree. Both captured transactions contain
only a System Program transfer; neither executes the allowance program.

| Transfer | Captured slot | Signature |
| --- | ---: | --- |
| fund-delegate0 | 503874997 | `3e8f1VCjhqj1Z4ofCQK552zvcVmFdKStZ2vanCjZEeaA9Gcd2XssEvjyKV6oRjWemt652UEa1yq2Y64ETovk2Nt8` |
| fund-delegate1 | 503875043 | `4ENzXmuFB6zmJZCQWU3TEfE2jw6kieeN88n3weW8UjiGwvKvcbv2eQmqmRCvRdRL1XEZVMUrMCbkT9CJzfy5BjUw` |

The pre-run deployed ELF matches the final gated allowance artifact. This attempt
has no completed application-test receipt or post-run ELF dump. Its two transfers
are additional spending, excluded from the 62 transactions in the successful
release lanes and publication prerequisite. The later complete capture is in
[`../byte-allowance-devnet/`](../byte-allowance-devnet/). No private keys are included.
