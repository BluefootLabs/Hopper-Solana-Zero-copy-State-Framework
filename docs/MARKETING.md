# Hopper product language

Lead with what Hopper is: **the zero-copy framework for Solana programs.**
Then why it matters: programs run at the cost of hand-written code, with the
checks a framework writes for you. Then the evidence: dated numbers next to
hand-written Pinocchio, devnet runs, and the self-audit.

Keep the language plain. Say what a program does and what it costs. Name a
mechanism only when it explains the benefit ("accounts are read in place",
"the PDA is checked with one hash from the stored bump"). A reader who has
never written a Solana program should follow the first screen of any page.

Rules that do not change:

- Every capability statement points to shipped code.
- Every performance statement names the workload, the date, and the commit,
  and links the script that reproduces it. The site keeps its figures in one
  file (`lib/measurements.ts` in the website repository) so every page quotes
  the same numbers.
- Compare against hand-written Pinocchio, the floor every Solana framework is
  measured against. The full framework table, with pina's published rows,
  lives in the repository (`bench/framework-comparison/results`).
- Safety statements explain their boundary: what is checked, where, and what
  the unchecked path is.
- Distinguish working examples from building blocks and planned work. Do not
  claim a complete exchange, airdrop, cNFT adapter, or DAO product from a data
  structure alone.

Public guides explain Hopper's APIs and application behavior. Keep research
notes and engineering analysis out of product READMEs, guides, and marketing
pages.
