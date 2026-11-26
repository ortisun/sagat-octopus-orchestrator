# Targeted re-read of design 054 for approval: correctness (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: b83781f7c0dfea6d880837aa2c439d4a384a0e5a
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. The second targeted re-read
(Owner decision 2026-11-25-2, step 4), not a full review. The reviewer read files only and ran nothing.

The history: the full review `../correctness.md` (at `87897d4a`: Blocker 2, Major 4, Nit 4), each Blocker and Major fixed;
re-read 1 `../reread-1/correctness.md` (at `ca46330f`: Nit 6), fixed; this re-read, at `b83781f7`.

Re-read 1 Nits 1 to 6: resolved (2 to 4 with the wording gaps below). No changed section adds a transaction, lock-order or
terminal-state defect: the successor's `run_version` of 3 plus the count is consistent; the settling events before the handoff
event fit `RunState::apply`; the handoff's replay finds its event by type; `EXCLUDE` means "must not be in a prefix", so the
rebuild predicate is sound. The reviewer searched the code for writers of `run.finished`: none outside the rule's reach.

Counts: Blocker 0, Major 0, Nit 3

## Nit

1. With up to three settling events, "empties the pending set on each, checking the counts" reads as the first event emptying it;
   a signal both at its move limit and undeclared has no stated bucket. Fix: subtract each event's count and check the set is empty
   at the handoff event or `run.finished`; `move_limit` takes precedence over `not_declared`.
2. `SIGNAL_PENDING_BYTES_FULL` keyed on "continued under another version" misses a later same-version continuation that still
   holds the moved signals. Fix: apply the 2 MiB check to every code run, with the A to B to C case in Task 054-9's test.
3. "A deferred constraint trigger" without its tables would miss a store-side commit that writes only `steps`. Fix: attach it to
   `steps` (a status changed to `WAITING`) and `run_signal_inbox` (insert), with a fixture that must fail at commit.

Settled in writing (Owner decision 2026-11-25-2, step 5), each carried into the build by follow-up Task 054-19: Owner decisions
2026-11-26-541 to 2026-11-26-543.
