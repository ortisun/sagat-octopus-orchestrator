# Security review of design 045, round 2 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 5033df9d
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author; every finding and its severity kept, with the author's answer.

Counts: Blocker 0, Major 5, Nit 6

First round: Majors 1, 2, 3, 5 and 6 closed (Major 3 for versions published after the upgrade, reopened below as Major 3);
Major 4 not closed (Major 4 below); Major 7 closed on the publication side only (Majors 1 and 2 below); Nits 1, 3, 4, 6, 7, 8
closed; the contested Nit 2 holds (a worker chooses its own code anyway); Nit 5 not closed (the `tenant_id` predicate on 022's
rows is no longer said).

## Major

1. **The replay-floor rule can race and disagrees with the ADR** (journal, published version, unfinished run: three rules); a
   journal scan also times out on a large table. *Answer: accepted*: the rule is on versions. Publication records, in the statement
   that inserts the version, which of this design's levels the version uses (`workflow_versions.activity_timeouts_level`: 0 none,
   1 timeouts, 2 the handler source); removal is refused while a version with that level is not purged. Publication holds the
   requirement, so the check cannot race. Requirement 045-017, Technical Plan 13 and ADR-0110 say the same.
2. **Publications can starve the operator's activate and remove** (a new `FOR SHARE` overtakes a queued `FOR UPDATE`). *Answer:
   accepted*: the hold takes a shared advisory lock on the registry's key and the operator functions take it exclusively before the
   epoch row, so the queue is fair; the hold is the last statement before the publication commits.
3. **Documents published before the upgrade can carry the new keys** (the root `activity` and `script` schemas are open). *Answer:
   accepted*: the new fields are read only for a version whose level is above 0, which only a publication under the requirement can
   set; other versions ignore them; the inventory of the upgrade lists stored root nodes that carry the keys; a test with such a
   stored document.
4. **The worker's delay is floored at the initial delay, not the computed one.** *Answer: accepted*: the delay is the larger of the
   worker's and the policy's computed delay for that attempt, at most 86,400,000 ms.
5. **Paused runs leave passed deadlines at the head of the index.** *Answer: accepted*: the pause writes the run's claimable steps'
   start deadline and close cutoff as null (the domain keeps them in its state); the resume writes them back, the start deadline
   moved by the paused time; the paused-flood case joins Task 045-17.

## Nit (each accepted)

1. The claim checks run in the wrong order: the eligibility predicate is applied in the claim's first filter, beside the managed-kind
   check, before the secret-pool refusal and its audit.
2. The close scan's fixed 6 s window: the domain computes a close cutoff (the close deadline less the floor), which is the indexed
   column.
3. The definer function's text: `REVOKE ALL ... FROM PUBLIC` before the grant, schema-qualified names, the capability checked against
   the closed list, the recorded state returned after the lock, a `lock_timeout` on publication.
4. An out-of-range `next_retry_delay_ms`: refused `INVALID_ARGUMENT`; the fingerprint takes the checked value; the path report
   receives it.
5. Where Requirement 045-019 applies: at the parse of the code, before the fingerprint and the path branch; a worker-sent
   `ATTEMPT_TIMEOUT` or `LEASE_EXPIRED` never gets a `deadline` field or a timeout count.
6. The heartbeat bound for older hosts: two beats per second per lease (the 500 ms clamp) is the bound the server must carry.
