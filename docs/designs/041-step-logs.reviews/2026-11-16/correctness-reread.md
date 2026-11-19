# Correctness re-read of design 041 on the owner's approval path

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 46c42bbbfc2f2c1cc918970086990fb590020ce4
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a fresh `correctness-reviewer`, asked for a short targeted re-read of the changes of resolution.md section 12 (OD-2026-11-16-7), given its own `correctness-approval-3.md`; no shell. Recorded condensed.

Third-run Majors: 1 (a filtered read ends early) and 2 (end-rule tests on the shared server) partly fixed (new Majors 1 and 4). The lock order L1 to L4, the marker's end rule, the statement count and the counting rule checked and sound.

## Blocker

None.

## Major

1. `contracts.md:276` tests completion by "fewer than 5,000 rows scanned", which a full page also meets: an unfiltered read at `limit=50` of a finished 300-line run answers `complete` after 50 lines, and a tail sends `end` after 200.
2. An empty page with `complete` false is read again at once, so a client that has caught up loops with no pause.
3. A failure of the marker's own transaction is neither stated nor counted (the runner's retry is a repeat; the marker is lost without a crash).
4. The 12,000-line guards of the end rule run on the shared server, where another test's held transaction can mask the old rule.

## Nit

1. "The run's last row" should be "the last row of the index range the request scans".
2. data-model.md says only `record_execution` writes the marker stream.
3. data-model.md says "the three tables".
4. The marker's expiry should come from the stored `script_executions.expires_at` (the pinned retention can be shorter).
5. T041-13's terminal-transition test has no hook and no way to end the run.
6. The 12,000-line run exceeds the default attempt ceiling.

Counts: Blocker 0, Major 4, Nit 6
