# Security review of design 045 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 67cc649e
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and its severity is kept, with the author's answer.

Counts: Blocker 0, Major 7, Nit 8

No Blocker: nothing lets one tenant read or write another tenant's rows (the new columns sit under the existing grant on
`steps`; tenant scope comes from `tenant_id` predicates; the lists are bounded and the code pattern is linear; labels are
closed; the secret-in-code check still runs first and its forced `retryable = false` lands on row 3).

## Major

1. **The claim's new conditions are only on the candidate hint**: under the run lock `claim_root_run`
   (`crates/application/src/worker.rs`) leases every `READY` root step of the run for the session's activities, so a parallel
   step with `heartbeatFrom: "handler"` or a passed start deadline would be leased with its sibling. *Answer: accepted*: the
   conditions are part of the check under the lock, on the transaction's time, in the root and the path claims; the candidate
   query keeps them only as a hint; a barrier test with two `READY` steps in one run (Technical Plan 3, Task 045-7).
2. **The new pass's fairness between tenants is unspecified**, and Gap 045-001 suggested sharing the leases window. *Answer:
   accepted*: `step_deadlines` is its own `pick` pass with the tenant exclusion, the per-shard limit, the shard fence and its own
   window; Gap 045-001 is closed as "never shares"; a lag criterion per tenant under a one-tenant flood (Non-functional
   requirement 045-002).
3. **Rows a poll can never claim sit at the head of the claim index** (expired start deadlines are the oldest rows; a
   handler-sourced step in a namespace with only older hosts waits without bound). *Answer: accepted*: `heartbeatFrom: "handler"`
   requires `scheduleToStartMs`, so such a row ends at its deadline; the bound is written down; a budget test with rows the poll
   cannot claim ahead of claimable work (Task 045-7).
4. **`next_retry_delay_ms` lets a worker override the publisher's backoff**, unseen in the journal. *Answer: accepted*: the
   worker's delay is clamped to at least the effective policy's `initialDelayMs` and at most 86,400,000 ms, and the event records
   `delaySource: worker` (Requirement 045-008, Technical Plan 6).
5. **A short `heartbeatTimeoutMs` multiplies run-lock transactions** with no quota. *Answer: accepted*: the minimum rises from 3,000
   to 10,000 ms. A beat costs one run transaction; today a lease beats every 10 s (a third of 30 s), 0.1 beats per second; at the
   new minimum a 1.9 host beats every 3,333 ms, 0.3 beats per second, three times today's rate per lease at most, and a lease
   count is already bounded by the activity's quotas. Measured in Task 045-17.
6. **A worker can report the two new platform codes** and so falsify "nothing ran". *Answer: accepted*: `fail()` maps a worker's
   `SCHEDULE_TO_START_TIMEOUT` or `SCHEDULE_TO_CLOSE_TIMEOUT` to `WORKER_ERROR`; timeouts are counted from the pass, never from a
   code (Requirement 045-002, Technical Plan 6).
7. **Publication and removal of the requirement can race** (`sagat_app` cannot lock the requirement epoch), and removal ignores runs
   in flight of retired versions. *Answer: accepted*: a `SECURITY DEFINER` function, `platform_requirement_hold(capability)`, takes
   the epoch `FOR SHARE` in the publication transaction (one grant, recorded in Non-functional requirement 045-003); removal is
   refused while a version that uses the fields has an unfinished run (Technical Plan 13).

## Nit

1. The fail fingerprint omits `next_retry_delay_ms`. *Answer: accepted*: it is part of the fingerprint.
2. The lists accept `[A-Z0-9_]` while the domain accepts a wider code grammar. *Answer: contested*: every SDK replaces a code
   outside `[A-Z0-9_]{1,64}` with `WORKER_ERROR` before sending (`sdks/SPEC.md` §6.5), so the lists name exactly what a host can
   send; a raw gRPC client that sends another code reaches the base policy, which is the conservative outcome for the author's
   intent only when the code is unknown to the author anyway.
3. The logging rule for a worker's code is ambiguous. *Answer: accepted*: logs carry only closed values; the code stays in the
   journal.
4. The registry's capability lists are closed `CHECK` lists. *Answer: accepted*: the migration and its down migration add the two
   capabilities (Technical Plan 2, Task 045-6).
5. The gate exit deletes 022's rows. *Answer: accepted*: the grant is 022's; Task 045-10 checks it exists and adds it with a
   `tenant_id` predicate if not.
6. A 1.8 session that sends field 14. *Answer: accepted*: refused `INVALID_ARGUMENT`, tested.
7. A merged retry policy is not validated as a whole. *Answer: accepted*: each effective policy is validated at publication.
8. The pass's fence is described inaccurately. *Answer: accepted*: both the shard fence (`View::holds`) and the step's own check
   are required, named apart.
