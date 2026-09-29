# Devnet evidence, 2026-09-28: the tail lab, round ten

Public devnet (`https://api.devnet.solana.com`), signer
`4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and
upgrade authority of every lane in this directory). The program is
`examples/hopper-tail-lab`, built with `cargo build-sbf` in a clean worktree at commit
`2ec0930` (local ELF SHA-256
`599c7173cebc31e84df2cc2f254544321c68e1e9c92069adfa7c18854472d4ed`, 24,376
bytes) and deployed fresh as `2dihUAuNMBu23gnsRAx7qtwRgXtKSptEDWGotc3NjFoD`
at slot 505,403,690. The on-chain dump equalled the local ELF before the
first transaction and after the last one (`before-onchain.so`,
`after-onchain.so`).

What this round proves: the in-place tail setters and a `#[check]` value rule on a
live cluster. The runner keeps a model of each account's tail and compares the
account with it byte for byte after every instruction. The reviewer list is
filled to its bound in place, a fifth reviewer is refused, the label and the
body are rewritten shorter, to their longest, and with multi-byte text, and a
rewrite signed by someone else is refused by `has_one`. `TailBlob.tag` carries
`#[check(value <= BLOB_TAG_MAX, error = TagOutOfRange)]`: a write and a creation
with a tag outside the rule are both refused with `Custom(6702)`, the first
with the account unchanged and the second with no account left behind. Every
refusal was asserted with the account's bytes unchanged.

15 transactions, slots 505,403,705 to 505,403,903, each sent with `hopper tx send` (the logs are
the CLI's own output, unedited) and re-fetched at `finalized`.

| Step | Signature | Slot | Result | CU |
| --- | --- | --- | --- | --- |
| deploy | `3tZNgcZgHV1mDyLuPGs1nGsWU3mWQFCm1FuagQSrZMNM2y7krURVjRYZfuUM5RS9MZfyQmRfAtdyBxLWuZajzs96` | 505,403,690 | program `2dihUAuN...`, upgradeable, authority = payer | |
| `init-note` | `HfjSajuzH3Q8Qi63jMbLnMDU7rsY8Bo6LW2qipQDEdhDMro9ExJfzkMsAP1inJge93xxDsdntqUCAGFU7WKMWxT` | 505,403,705 | ok | 2,185 |
| `add-reviewer-0` | `4vQDaic24hewXS2rHNQmgAAK1BgejknmVPKpSV8sGpvz1cN3BHpsGiBDxNtDBFRkmUehb6Wjh9z8a2afJecogbov` | 505,403,755 | ok | 628 |
| `add-reviewer-1` | `4y7km6EvvhAyV197adwjbxstSNNGR29oRVCHTXLWxNCaREvm7oX1VKtNTouVuLerQio2XbmxNXke3WiJaBSKgRHv` | 505,403,762 | ok | 645 |
| `add-reviewer-2` | `3bPbiT3Vod1Q495aLQyDtuntRKSbhdT7a7oy8XB8syDJ9xN2CLCCn1Pt1BvCbHpnYR9APXzEqStVPaERsWWb23zo` | 505,403,771 | ok | 662 |
| `add-reviewer-when-full` | `5ra6A3uT6jzcTtznqmjnD3aWgfACfjnnXqcidfc33VaurqGVYGsCDmNpkNMyAnDnXE448vFnem6veMsyykCgvEyg` | 505,403,778 | refused: `AccountDataTooSmall` | 574 |
| `rewrite-note-shorter` | `5VDmBA1tnYcGECj84Z6cpYbisXehbx8X8Mg7Sb1hNDidTLse4MG7EiY1WyYTx4ibDfcXrtj3Gi44zJJFb4ySNNGG` | 505,403,791 | ok | 1,016 |
| `rewrite-note-longest` | `5oxK2Ap3aqB5eb6ZjEaMTfRDHE7h8NxivHyPuWQhrf9RR6RmhBaJetCxYWd8m3zaVtF9ZPqVo6FSU7aYde4uZQZj` | 505,403,818 | ok | 1,331 |
| `rewrite-note-utf8` | `5HJhbjmEr8TszhQxxEE5GeM1LCXnDSTVR7iN2s8VHCYFDsXNDue1wnAcsGrFSuDcBt5rkFhQajrxvKbzg9wu7Fag` | 505,403,827 | ok | 2,169 |
| `rewrite-note-empty-body` | `3XMcvRer9waowMFawYshfVmoN8TvqtZPm7E5N8vftSaoDEtauGNVKW3exBovatDvzVpP9M79wE9j7vXNFdBx1MM1` | 505,403,834 | refused: `Custom(6700)` | 352 |
| `rewrite-note-by-stranger` | `2EFNgfd9jK7HefoXUHgQW439p9aTSX8rqFyKuuAjeiNe8medEipVHsQKsY8tboaCQYNkkKkEdNFn9Kjvh3sXMaRm` | 505,403,846 | refused: `InvalidAccountData` | 338 |
| `init-blob` | `2ACD77GUyGFeLV4buViuqPQZdasq918Gr5J5DwqJwXP3iDhEV2zFwt4UkzYExofVt9iDhEfuPpNUQYkM5ynxbCW6` | 505,403,858 | ok | 1,778 |
| `write-blob-longest` | `2HskSFAdCM8PE78U6VYoKbFdP784mtFeQdZDEatkRBAemBRou1RpETChwuJ7jo1AFGoGyw3jjxsRnA96GP445Esx` | 505,403,868 | ok | 1,645 |
| `write-blob-tag-out-of-range` | `5o8QknvdV1DyQ8fRrwpDDKRgHFLGRKVAKfBeb47CyrDG9PoyfhtRnZPSktQ6w5SRgSSZUJzapevC7YkjCJGXDADn` | 505,403,878 | refused: `Custom(6702)` | 374 |
| `write-blob-empty` | `2w4W8m6WpU324WCWfQVrN2eLYNqEXvxSFfQjnSfdhX6YxmUTcrZStwQsxrF5LFhyEbS94pRAmFCLfivySXj5zu2F` | 505,403,891 | refused: `Custom(6701)` | 323 |
| `init-blob-tag-out-of-range` | `YeJgKc9StRNB3mW1Go921bSYt9HJ9TVVVfTe2fmby1o7d7rGRdFQ3rv9iL6CnsB7dA3vPvhtV8NsfCaPn6s22Ka` | 505,403,903 | refused: `Custom(6702)` | 1,700 |

`receipt.json` is the runner's own record; `SHA256SUMS` lists every file here.
