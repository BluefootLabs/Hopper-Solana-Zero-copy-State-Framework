# Minimal SOL vault fixture

A small systems-level program for measuring validated SOL custody operations.
The instruction set includes deposit, withdrawal, authorization, and counter
access. Deposits invoke the System Program; withdrawals debit a validated
program-owned vault after authority and PDA checks.

This fixture is for execution and measurement work. For normal application
onboarding, start with [the typed SOL vault](../hopper-vault) or
[funded token escrow](../hopper-escrow). See [program measurements](../../BENCHMARKS.md)
for dated results and reproducibility requirements.
