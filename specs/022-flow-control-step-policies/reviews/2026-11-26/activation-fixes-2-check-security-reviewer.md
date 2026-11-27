# Security check of the mixed-period fix `59a10d3d3..472a1c9cf` (decisions D022-156 to D022-159), 2026-11-26

The repository's read-only `security-reviewer` checked the fix of its Medium 1 (`activation-fixes-rereview-security-reviewer.md`).

## Verdict

| Earlier finding | Verdict |
|---|---|
| Medium 1, a key of several periods | Fixed: the rule is exact for a recorded lease, judged under the window lock on every path; no limit can be exceeded; nothing crosses tenants |
| Nit 1, the allowance's reason | Fixed |
| Nit 2, the share-lock test | Fixed |

## Low 1 (plausible deadlock). A withdrawal that ends a stopping run takes its locks out of ADR-0050's order

- **Where:** `lock_withdrawn_gates` (`crates/infra-postgres/src/execution/readiness_gate.rs:1757-1761`) takes the step's
  keys (slot 7, then 8b) before the command. The command then cancels the step of a stopping run and settles it
  (`crates/domain/src/execution/command.rs:2239-2240`); if it was the run's last active step, `persist` locks the
  active-run counter row (slot 6) after those keys, and gives back the run's own semaphore after a step's window
  (`crates/infra-postgres/src/execution.rs:5603-5613`).
- **Scenario:** an admission of another run of the namespace holds the counter row and the semaphore and asks for the
  window; the withdrawal holds the window and asks for the counter row. Deadlock.
- **Asked:** in `lock_withdrawn_gates`, when the run is stopping (or always, if cheaper and correct), take
  `run_active_counts_lock_slot` first and add the run's own holder's semaphore to the sorted pass of semaphores. Test, red
  first: hold the counter row and the semaphore in a test transaction, hand back the last step of a cancelling run, then
  have the holder take the window with a `lock_timeout`: it finishes without a deadlock.

## Note 2. D022-157 overstates its exactness

- **Where:** the lease pass writes `leased_at` without the window lock and back in time, as the instant of the journal's
  lease (`crates/infra-postgres/src/maintenance.rs:363`).
- **Scenario:** decisions made before the pass recorded the instant counted the grant as unleased; the rule then judges
  them against the earlier recorded instant and can refuse a withdrawal whose slot every decision counted. It only refuses
  more, never less, so it adds to G022-042's cost.
- **Asked:** say so in D022-157 and G022-042; add the two missing tests: a grant at exactly `leased_at + period`
  (refused), and a hand-back that ends a run being cancelled (the test of Low 1 covers it).
