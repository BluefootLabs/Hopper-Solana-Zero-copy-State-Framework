# Designing a Hopper program

Start with the actions users need: deposit, transfer, claim, exchange, approve,
withdraw, or close. Specify the authority and balance changes for each action
before choosing its account layout.

1. Declare typed account roles and instruction inputs.
2. Bind owners, signers, PDAs, mints, destinations, and accepted programs.
3. Validate business rules and arithmetic before mutation.
4. Release incompatible borrows before checked CPI.
5. Use the owning program's instructions for token or wallet debits.
6. Check exact expected account state, balances, refusals, and rollback against
   compiled SBF. Repeat asset lifecycles on the target cluster.

Named fixed-field inputs and `init_<account>_with(values)` keep initialization
readable. Bounded dynamic fields support labels and member lists. Lower-level
account views remain available when the program needs explicit memory control.

Application constraints remain your policy. Ownership alone does not prove a
PDA's canonical bump; a signature alone does not establish configured membership;
a write declaration alone does not authorize an asset transfer.

Read [named initialization](NAMED_INITIALIZATION.md),
[program architecture](ARCHITECTURE.md), and
[governance examples](GOVERNANCE_PROGRAMS.md).
