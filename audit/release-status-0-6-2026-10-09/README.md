# Hopper 0.6 publication status — October 9

The framework candidate was committed and pushed as `9f5cbb05165a96016fe67f9d503070edf17aeb01` by
QuarksBlueFoot. The commit has no co-author trailer. The
[candidate archive](../release-candidate-0-6-2026-10-09/README.md) records local
tests, compiled execution, live devnet transactions and website validation.

## Clean package validation

All 29 packages pass the publication-train metadata, dependency-order,
README/source-include closure, and archive checks from that exact clean commit.
The tree remains clean and unchanged throughout validation. `package-clean.json`
records command arguments, every archive hash, authors and publication metadata;
`packages/` retains the matching `.crate` files.

Archive construction uses `cargo package --locked --no-verify`: it does not
claim registry-only compilation, a successful registry dry-run, or publication.
The 25 changed packages target 0.6.0; four unchanged support packages retain
their existing versions and must not be uploaded again.

## Hosted gate blocker

[Solana SBF gates run 37891302003](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/actions/runs/37891302003)
could not start either required job. GitHub's annotation says:

> The job was not started because your account is locked due to a billing issue.

Both jobs have zero executed steps. The Rust release and unsafe-safety workflows
were blocked for the same reason. `hosted-ci/` retains the API responses, job
records and annotations for the candidate commit. This is an account-level
execution blocker, not a compiler or test failure; local results do not replace
the required hosted attestations.

No crate was uploaded. The registry observation still reports hopper-lang
0.5.0 and no indexed 0.6.0. Publishing remains pending until the account billing
lock is cleared and the required release jobs complete. Registry dry-runs,
ordered uploads, downloaded-archive checks and a registry-only consumer must
then be recorded before announcing 0.6.0 as published. Independent review stays
unstarted and is not claimed by these internal checks.

`staged-evidence.json` records validation of 57 bundles and 6,522 staged blobs
before the candidate commit. One persistent Git reader avoids repeated process
startup on Windows; each returned blob ID is checked, and the existing bundle
verification logic is unchanged. The 44 historical never-archived entries retain
their original explicit disposition.
