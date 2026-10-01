# Marketing Hopper

Lead with what people build: token escrow and settlement, payments, claims,
treasuries, and asset applications. Explain how Hopper helps them read state,
authorize actions, move funds, and test the result. Introduce lower-level
runtime details when the reader needs them.

- Keep source comparisons in research and benchmark artifacts, outside
  product READMEs, guides, and website marketing.
- Attribute performance to a workload, source commit, toolchain, and dated
  evidence. Do not generalize a small fixture into universal cost parity.
- Identify current-branch features separately from published packages.
- Describe which account operations are zero-copy; do not promise that all
  application code avoids copies, decoding, or allocation.
- Name the boundary of each safety check and the application's remaining
  responsibility. Do not imply a whole-framework proof or external audit.
- Distinguish tested examples, reusable APIs, and integrations still needed.
  An order store is not a settlement engine; token helpers are not an airdrop
  product; cNFT support needs a Bubblegum integration.
