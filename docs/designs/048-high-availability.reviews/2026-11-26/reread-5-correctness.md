# Targeted re-read 5 of design 048: correctness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: e15a7ba101fb24d5b901fd3242de6ee92fece4b3
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since 2d63c0bd, not a full review. The reviewer read files only and ran nothing.

## Major

1. Success criterion 048-003's first clause can never fail: an expired lease's open gap is always about the time to live,
   so it is always excused, and it removes its repetition from those that qualify. Fix: judge by the repetition's
   measured outage.
2. Closing the connection on every dropped `WriteTx` turns today's deliberate rollbacks (refused admissions, contended
   locks) into connection churn, possibly past the role's connection limit under load. Fix: close only when a statement
   or the commit was in flight; otherwise a spawned `ROLLBACK`; a test that a refused admission keeps its connection.

## Nit

1. `ReadTx`'s drop rule.
2. `DatabaseHealth` needs `take_ticket()` and the connection's kind.
3. The `42P01` branch is unreachable for a serving-profile command.
4. The two lint rules conflict on `transaction_read_only`, and a bare `END` matches `CASE ... END`; ADR-0130 decision 3
   omits the read-write profile.
5. The negative control must append its row to the workload's ledger file.

Counts: Blocker 0, Major 2, Nit 5

All seven were fixed at the next commit.
