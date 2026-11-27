# Resolution of round 5 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed commit of round 5: 9b64ac22698c34d847c9658b8a08b5a5c68814d2

How the Majors of round 5 were answered, by the author under the owner's delegation (OD-2026-11-20-6,
OD-2026-11-24-4). The decisions are Decision 044-148 to Decision 044-158 and Gap 044-024. Every Nit was applied.

Counts before fixes: design re-read Blocker 0, Major 1, Nit 12; implementation readiness re-read 0, 1, 11; security 0,
2, 9; architecture 0, 3, 11; correctness 0, 5, 5. Total Blocker 0, Major 12, Nit 48. Round 4 had Blocker 1, Major 31,
Nit 69.

| Findings | Answer |
|---|---|
| design 1; correctness 4 (updates without an outcome when the run ends) | The stop drain and the handoff finish every open update, `REJECTED` or `FAILED` with `RUN_ENDED` or `RUN_CONTINUED`; admission to a stopping run is `409 RUN_NOT_MUTABLE`; barrier tests (Decision 044-149) |
| correctness 1 (three cancel paths bypass cooperation) | One domain function `execution::request_cancellation` for the route, the chain cancel, the `propagate` children pass and the pending reset's source; a source-text check; three tests (Decision 044-148) |
| correctness 2 (the closing window refuses consumption and update decisions) | Admitted; the history-limit test consumes a signal in the crossing completion (Decision 044-152) |
| correctness 3 (a deferred terminal is outside the chain) | The deferral record holds kind, target and body digest; a differing re-emission is `CHAIN_MISMATCH`; the replayer and the move check compare it (Decision 044-150) |
| correctness 5; architecture 14 (reset and the cancellation deadline) | The event carries the grace; the reset re-arms the deadline; `verify_run` derives it (Decision 044-148) |
| architecture 1 (constitution draft: "refused", "all or nothing") | Kept the deferral, which round 4 chose because a refused completion repeats for ever on an obligation the function cannot settle; Gap 044-024 asks the draft's owner for the wording; principles I and IV are "Pass with conditions". Settled in writing below |
| architecture 2 (schema-less payloads `pii` diverges from Architecture decision 0033) | Kept `pii` for code runs and stated as a refinement of Architecture decision 0033 (as Decision 302 did for events) and an extension of Architecture decision 0036 decision 6, with the reason; a test; Task 044-6 widens the rules check (Decision 044-154) |
| architecture 3; readiness 1 (rustdoc JSON cannot list calls) | A text check over every method name against a checked-in `(file, method, count)` allowlist, no new dependency, a negative fixture (Decision 044-151) |
| security 1 (approval initiators still partial) | Every engine-held approval, activity or command; starter, external actors up to the activation, movers and review retriers; one test case per kind (Decision 044-153) |
| security 2 (opaque secret reference) | Dropped: a `secret` field reaches the lease as the typed placeholder; a code run cannot route a secret (Decision 044-154) |

## Settled in writing (delegate, OD-2026-11-24-4)

- **Architecture round 5, finding 1.** The reviewer offered two fixes: register a gap for the draft's wording, or go back
  to refusing the whole completion. Delegate (OD-2026-11-24-4): the gap. Refusing the whole completion was the round-3
  design, and round 4's correctness review showed that it loops when the cause is an obligation (an aborting saga, a
  step under review) that no new event settles, or deadlocks when the function has already returned. Deferral keeps
  every applied command atomic, takes no command number for the deferred one, and is now chained by its digest, so the
  guarantee the clause protects (no run ends past an unseen event) holds. The draft is another session's branch, so the
  change is asked for there (Gap 044-024) rather than made here.
- **Architecture round 5, finding 2.** Keep `pii` rather than structure only for everyone. Delegate (OD-2026-11-24-4):
  structure only for everyone would leave no reader able to repair a code run whose state or side-effect values explain
  its review, and would make the history export unusable for the replayer; Decision 302 of feature 016 already made the
  same choice for event data. The reveal grant stays the gate.

The other ADR contradictions the correctness review listed (Architecture decisions 0103, 0104, 0105, 0107, 0108) were
rewritten to match the design, and every name in their "Enforced by" lines is now a test that a task creates.
