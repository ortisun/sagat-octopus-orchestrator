# Correctness re-review of 34b8be41 (key instants to the minute)

Reviewer: `correctness-reviewer` (read-only), given the diff of `34b8be41` and the spec, 2026-11-20. Full text as
returned, then what was done.

I read the code and ran nothing. I read: the diff; FR-022-022, FR-022-031 and FR-022-057; D022-055, D022-057, D022-059
and D022-060 (including its revision paragraph); the G022-014 and G022-015 rows; both OpenAPI documents; and the three
earlier reviews. I also read `operations.rs`, `runs.rs`, `dto_flow_control.rs`, `infra-postgres/src/flow_control.rs`,
`gate_queue.rs`, `readiness_gate.rs`, the scheduler's `grant_waiter`, `apps/control-plane/src/{config.rs,lib.rs}`,
`apps/execution-engine/tests/registry.rs`, `tests/integration/flow-control-step-policies/{rest.rs,admission.rs}`, the
runbook and the release note.

## Short answers

- **Is the cut correct?** Yes. The values come from PostgreSQL `timestamptz`, which sqlx decodes as UTC. Replacing the
  second and nanosecond fields rounds down toward the past, including before 1970: 1969-12-31T23:59:59.999999Z becomes
  23:59:00Z, with no rounding toward zero. Both `replace_*` calls take 0, which is always in range, so the
  `unwrap_or(at)` fallback can never run. If it ever did run, it would return the whole instant (minor 4).
- **Does it serve FR-022-022 for callers without the reveal grant?** Yes. They now get the oldest wait to the minute.
  The age they work out from it is too large by under 60 s.
- **Does it close the linking channel from security Major 1?** No, only partly (Major 1). `Run.concurrency.since` still
  gives the whole instant to the same caller. In any group where runs start in different minutes, matching the minute
  of each run's `since` to a key's instants still links the runs. The order of the listing within one minute also
  leaks. The sealed cursor leaks nothing.
- **Does the contract match the code?** The text matches what the code does. Its reason ("since whole they equal the
  `since` of the key's runs") suggests a protection the API does not give. The runbook, the release note and the
  G022-014 row are out of date (minor 5).
- **Do the tests prove it?** Only half. Removing the cut fails the test. Applying the cut to revealers as well passes
  it, and cutting to the hour or day passes it too (minor 3).
- **Regression for callers with the grant?** None in the code. The revealer branch is unchanged. The cursor is built
  from `KeyPosition.oldest_wait_us`, which the cut does not touch, so paging is not affected.
- **The configuration half of the commit** (a process with every switch off reads no key file) brings in a drain
  problem (Major 2).

## Major 1: the minute cut does not stop the linking it was written for, because `Run.concurrency.since` stays whole for the same caller

**Where.** `crates/application/src/operations.rs:685-691` cuts only the listing. `crates/application/src/runs.rs:2373-2398`
(`reveal_permit_labels`) handles only the label; `permit.since` is never touched. `crates/api-http/src/dto_flow_control.rs:35`
writes `since: permit.since` to the microsecond for every caller with `run:read`.

**Failing scenario.** Namespace `payments`, group `customers` (limit 1), a few starts an hour, a viewer without a
grant. `GET /runs` shows A `HOLDING` since `10:03:17.412305Z`, B `WAITING` since `10:41:02.118004Z`, C `HOLDING` since
`10:15:44.900120Z`, all `keyLabel: null`. `GET /concurrency-keys` shows key 1 (holders 1, waiters 1, `oldestHolderSince
10:03:00Z`, `oldestWaitSince 10:41:00Z`) and key 2 (holders 1, waiters 0, `oldestHolderSince 10:15:00Z`). The minute of
A's `since` matches key 1's oldest holder and B's its oldest wait: A and B share a customer, exactly the scenario of
security Major 1. The cut only gets in the way when two candidate runs of one group and state start in the same minute;
in low-rate groups the link is unique. Two more paths: the order (within one minute the listing is sorted by the whole
`oldest_wait_at`, `flow_control.rs:491-526`), and the guess oracle (G1185: a developer without the grant starts a run
with a guessed value and looks for a key whose `oldestHolderSince` equals the minute of the run's `since` with
`holders: 1`; the item count tells the same without instants).

**Why the tests miss it.** `rest::a_key_label_is_shown_only_to_a_caller_who_may_reveal_personal_data` never compares the
listing's instants with the `since` of the runs the same caller reads.

**What the records claim.** D022-060 says "linking needs runs one minute apart", the usual case, not the rare one. The
canonical contract (`openapi.yaml:2263`) gives as its reason that the instants "equal the `since` of the key's runs",
but that `since` is served whole beside them.

**Smallest fix.** A recorded decision: (a) accept the residual honestly in D022-060, G022-014 and the contract; or (b)
close it with a per (pool, scope, group) oldest wait for non-revealers (G022-014's aggregated listing), which changes
FR-022-022's per-key wording and is the owner's call. Cutting `Run.concurrency.since` as well would not be enough.
Under OD-2026-11-20-6 the stronger choice is (b) with an O-question; (a) is the smallest. **Confidence:** confirmed in
code.

## Major 2: with every switch off a process now never holds the ring, so existing waiters and gated steps cannot drain (FR-022-031)

**Where.** `apps/control-plane/src/config.rs:192-195` returns `Ok(None)` before the file is read when the switches are
off; the engine uses the same function (`apps/execution-engine/src/settings.rs:646`). Without a ring, granting from the
queue refuses (`gate_queue.rs:66-71`, `BinaryCapabilityMissing`) and the scheduler only logs (`passes.rs:5752-5759`);
deciding a step's gate refuses (`readiness_gate.rs:100-104`), and so does sealing a cursor.

**Failing scenario.** Key K has one holder and two `WAITING` runs; the operator turns the switch off to roll back and
leaves the Helm secret mounted (`_helpers.tpl:232-247` mounts it whenever `existingSecret` is set). Before the commit the
ring was still loaded and the grant pass granted the waiters; after it every waiter fails `CONCURRENCY_WAIT_TIMEOUT` at
its deadline and a running run's gated step cannot be decided. FR-022-031: "disabling MUST stop new publications and
starts while existing runs drain"; the plan's rollback keeps the grant pass running until waiters drain. The earlier m1
had a real point (`check_flow_control_keys` skipped a switched-off process's ring), but the cure goes beyond the spec.

**Why the tests miss it.** No test turns the switch off while holders or waiters exist; `admission.rs:314` builds a
switched-off store with a ring, a configuration production can no longer produce.

**Smallest fix.** Load the ring when a file is named whatever the switches, check it whenever held, let the switch
govern admission only; test admit, restart switched off with the file, end the holder, the waiter is granted.
**Confidence:** plausible (configuration and ring checks confirmed; the run's path after a step's
`BinaryCapabilityMissing` not traced).

## minor 3: the test passes if the cut is applied to revealers, or cut too coarsely

`rest.rs:445-455` asserts only `if !revealed` and only `(second, nanosecond) == (0, 0)`. Calling `to_the_minute` for
every caller, or cutting to the hour or day, passes. Fix: compare with the holder's and the waiter's `concurrency.since`
(equal when revealed, its minute otherwise).

## minor 4: `to_the_minute` fails open and has no unit test

`operations.rs:775-779`: the `unwrap_or(at)` fallback would return the whole instant. Do the arithmetic directly or
`expect`; unit test 1969-12-31T23:59:59.999999Z, an exact minute, and `:59.999999`.

## minor 5: documents say instants are hidden, or that exact instants go only to grant holders

`research.md` G022-014 ("instants are hidden"), `docs/runbooks/flow-control.md:55`, `docs/releases/unreleased.md:13`, and
the contract's reason. Reword with Major 1's decision.

## minor 6: the configuration change has no test of its own

Nothing asserts that switches off with a file gives `flow_control_keys == None` (or the reverse); reverting
`config.rs:192-195` fails no test.

## minor 7: the evidence for T022-026 does not show this commit

`tasks.md:116-127` cites "`40854f4d` and its successor"; the hand mutations listed do not include "cut removed" or "cut
for all callers".

## nit

The contract should say "truncated toward the past (UTC; seconds and fractions zero)", also on the `ConcurrencyKey`
field schemas. The `group` pattern change is correct (`maxLength: 130` covers the longest group, 129).

## Checked with no finding

The cursor and paging (sealed from the whole `oldest_wait_us`); callers with the grant (unchanged); one reveal decision
per page; time zone (UTC, `…:MM:00Z`).

---

## What was done (2026-11-20)

| Finding | Resolution |
|---|---|
| Major 1 | D022-061 (delegate, OD-2026-11-20-6): the end state is (b), the aggregated listing per (pool, scope, group) for callers who may not see labels, follow-up T022-111 (a release item beside T022-104, built on T022-105's counter rows so a page costs its own groups); until then the cut stays and every text says it narrows and does not stop linking (contract, fragment, runbook, release note, G022-014) |
| Major 2 | D022-062: a process with the switches off that is given the file holds it (read as strictly), the ring is checked whenever held; tests `registry::an_engine_with_its_switches_off_holds_a_given_key_file_to_drain_and_lists_no_gate` (red with the early return) and `run_admission::with_the_switch_off_a_process_holding_the_keys_drains_the_queue`; runbook "Turning the switch off" |
| minor 3 | the label test compares each instant with the holder's and the waiter's `since`: equal when revealed, its minute otherwise; red with the cut applied to revealers (hand mutation) |
| minor 4 | `to_the_minute` is subtraction (cannot fail); unit test `operations::tests::an_instant_is_cut_to_its_minute_toward_the_past` (the three cases and an exact minute) |
| minor 5 | reworded (see Major 1) |
| minor 6 | the registry test above |
| minor 7 | the evidence of this change names its commit and mutations |
| nit | the operation's description says "truncated to their minute (toward the past, UTC; seconds and fractions zero)"; the field schemas are not changed (a `$ref`'d schema change moves no pinned digest, but the description lives on the operation) |
