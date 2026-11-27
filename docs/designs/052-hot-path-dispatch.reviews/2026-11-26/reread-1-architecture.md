# Targeted re-read 1 of design 052: architecture (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: f079afa66db35059c7525cda69aa9a986c79d4bc
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`architecture-full.md`, not a full review. The reviewer read files only, ran nothing, and could not list branches.

Full round: the Blocker and Majors 1 to 6 and 8 resolved; Major 7 and Nits 1 and 5 partly (below); Nits 2 to 4 resolved.

## Major

1. A poller alone in the fleet can lose oldest-first order (FR-019-013, Owner decision 2026-11-12-3): contention also
   counts run locks of its own completions and timers. Fix: say the lone-poller order holds only while uncontended, in
   Owner decision 2026-11-24-526 and ADR-0133; hand a gap to feature 019.
2. The CI run with the wake off goes beyond the constitution's list of suites. Fix: the lost-wake test and a wake-off
   case in `integration_claim_transition_efficiency`, which already runs for lease and completion code.

## Nit

1. The payload is defined two ways (empty, and the instance's identifier).
2. Requirement 052-055 changes `specs/006-sdk-multilanguage/stream-completion.md`'s hint rule, unlisted.
3. The ADRs' Enforced by omit seven tests; their Evidence cites Task 052-5 for Task 052-9; ADR-0134's option 1 names NATS
   only.
4. The control plane's new broker connection is not in ADR-0134's Consequences; the Redis transport's crate is unnamed.
5. A broker outage at start keeps the wake local for the process's life; check again with a backoff.
6. The span event `woken` and the coalesced counter are in no requirement; Task 053-8 does not name the withdrawn
   identifiers.

Counts: Blocker 0, Major 2, Nit 6

All eight were fixed at the next commit.
