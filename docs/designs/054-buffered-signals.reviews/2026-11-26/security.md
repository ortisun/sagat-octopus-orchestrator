# Security review of design 054 (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: 9ba3ed8587738ff3835e157c3ac8d8c90fa42112
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. A full review of the design
and Architecture decisions 0111 and 0112. The reviewer read files only and ran nothing. Every finding is fixed; none is
accepted (a security finding is never settled by a delegate). The author's answer follows each finding.

Counts: Blocker 0, Major 7, Nit 2

Checked with no finding, per the reviewer: the tenant boundary of the SQL (composite foreign keys on tenant and run, tenant and
payload), authorization of signalling before the idempotency replay, the grants, the journal's detail not returned, closed metric
labels, the dynamic SQL bounded to migration-inserted `regprocedure` values.

## Major

1. **The journal read returns the pending payload's digest as `payloadRef` to every `run:read` caller,** so a low-entropy pending
   value can be confirmed by hashing guesses. *Answer: fixed:* the journal read answers a null `payloadRef` for `signal.buffered`,
   with a contract test (Requirement 054-014, Task 054-8); the same exposure for step outputs is recorded for feature 016 (Gap
   054-011).
2. **Inspection and disclosure by journal sequence reach a `signal.buffered` payload.** *Answer: fixed:* both selectors answer
   `NoRetainedPayload` for it, a test per selector (Requirement 054-014, Task 054-8).
3. **The list gives a payload's size to viewers,** which feature 016 treats as structure needing `run:payload:inspect`. *Answer:
   fixed:* `bytes` only for a caller who holds it, in the route and the Studio (Requirement 054-016, Technical Plan 13).
4. **The list would take the run's write locks through a `RunTx`.** *Answer: fixed:* a lock-free, tenant-scoped read of the pool,
   as `get` is (Technical Plans 13 and 17).
5. **A sender can exhaust the tenant's content quota through inboxes.** *Answer: fixed:* a namespace resource of Architecture
   decision 0027, `pending_signal_bytes`, charged by a delta trigger and checked under the policy fence (`429`,
   `PENDING_SIGNAL_BYTES`); the guide states what stays charged (Technical Plan 2, Requirement 054-005, Decision 054-017).
6. **A continuation multiplies journal rows outside any quota** (up to 1,000 per handoff across up to 10,000 segments). *Answer:
   fixed:* a signal crosses at most 100 continuations, then it is discarded and counted (`move_limit`), so each accepted signal
   writes at most 101 events, a bound per action (Technical Plan 7).
7. **The hold takes a requirement row `FOR SHARE` without the epoch, against the registry's lock order,** so a slow publication
   could stall the fleet's registrations behind a removal. *Answer: fixed:* the hold takes the epoch `FOR SHARE` first, reads the
   row without a lock, answers only for a capability with a floor row, and runs as the publication's last statement (Technical Plan
   15, Architecture decision 0112).

## Nit

8. **Privileges of the floor's functions.** *Answer: fixed:* `EXECUTE` revoked from `PUBLIC`, the trigger `SECURITY INVOKER` and
   enabled `ALWAYS`, a registered function checked for no argument, a `boolean` result and the trigger's owner (Technical Plan 15,
   Architecture decision 0112).
9. **A flood cannot be traced to a caller.** *Answer: fixed:* tenant and run identifiers on the span and in the authorized log
   fields of a refusal; refusals are not audited one by one, so a flood cannot amplify the audit log (Technical Plan 14).
