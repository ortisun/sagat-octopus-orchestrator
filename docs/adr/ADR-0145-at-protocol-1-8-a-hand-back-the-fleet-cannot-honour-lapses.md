# ADR-0145: At protocol 1.8, a hand-back the fleet cannot honour lapses

**Status:** Accepted (2026-11-24, delegate, OD-2026-11-24-4; revised before its merge by round 16 of the design review, Decision 006-068)
**Decided by:** agent, delegate (OD-2026-11-24-4), on the coordinator's disposition of Major 1 of the round-15 architecture
review of the delivery acknowledgement design (`specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-15/architecture.md`);
the reasoning is Decision 006-067 of `specs/006-sdk-multilanguage/research.md`, and Decision 006-068 for the round-16 revision
(rules 2, 3, 4 and 5; delegate, OD-2026-11-24-4). Reported to the owner afterwards, as
OD-2026-11-24-4 asks.
**Refines / supersedes:** **supersedes Architecture decision 0099 decision 9** for one case only: a hand-back of a lease under the
reserved code, still `LEASED` to it, that the server refuses for a reason of the fleet rather than of the lease (and, with it,
the clauses of decisions 2 and 7 of Architecture decision 0099 that repeat "its hand-back is an ordinary failure" for a mark that
does not count). Keeps Architecture decision 0098 decision 5 as built (Decision 006-089: from 1.8, a hand-back on a fleet that
does not withdraw records nothing and answers `STALE_ATTEMPT`) and extends it to the fleet reasons that 1.8 adds. Every other part
of Architecture decision 0099 decision 9 stands.
**Enforced by:** to be built with Task 006-050 of `specs/006-sdk-multilanguage/tasks.md`: the unit test of the hand-back verdict
over every reason of the fleet and of the lease; the root and scoped cases of Task 006-050 (each reason of the fleet answered
`STALE_ATTEMPT` with nothing written and the lease then ended by the lost-lease pass with `expire_attempt`; each reason of the
lease an ordinary failure; a hand-back past the expiry before the pass); the base's test
`a_hand_back_on_a_fleet_that_does_not_withdraw_leaves_the_lease_to_lapse`
(`tests/contract/durable-execution/grpc/lease_withdrawal.rs`), kept green through Task 006-050; the mixed-fleet case of Task
006-066; the host's unit case of Task 006-059 to Task 006-064 (a stale hand-back left to lapse, then the probe).

## Context

Architecture decision 0098 decision 5, as built by pull request #260, answers a hand-back from a session that reserves
`TASK_NOT_STARTED` on a fleet that does not withdraw (the requirement `lease-withdrawal-v1` not recorded, or the engine's switch
`SAGAT_LEASE_WITHDRAWAL=off`) with `STALE_ATTEMPT` and records nothing, so the lease lapses as before: an `Activity`'s lapse goes
to a review an operator can retry, and the attempt of work that never ran is not spent (Decision 006-089). Decision 006-080
removed the hand-back below 1.8 for the same reason: an ordinary failure would fail a one-attempt `Activity` for good.

Architecture decision 0099 decision 9, and the places that restate it, said instead that at 1.8 a hand-back whose reservation does
not count is an ordinary retryable failure, which spends the attempt. The reservation does not count, at 1.8, for reasons that
belong to the fleet: `delivery-acknowledgement-v1` not recorded, or the lease claimed before its current recording; a replay floor
(`lease-withdrawal-v1` or `lease-withdrawal-v2`) not recorded; the switch off for an unmarked lease; the lease claimed on another
incarnation of the database server. The window in which this happens is not rare. From Task 006-057, which offers 1.8, to the
operator's activation of `delivery-acknowledgement-v1`, and again after a rollback removes it, every stopping 1.8 host hands back
what it holds; as an ordinary failure, a one-attempt `Activity` fails for good where the base sends it to a review. That is the
regression D006-080 and D006-089 removed, brought back by the design that was to build on them.

Decision 006-064 made the hand-back of a lease whose reservation does not count an ordinary failure for a real reason: in a mixed
fleet an engine of the floor build serves a capability without recording `lease_capability_generation`, so a withdrawal could hide
a capability that was served. That reason argues against a withdrawal, not for an ordinary failure: a lapse withdraws nothing.

## Decision

1. At protocol 1.8, a hand-back (`FailStep` with `TASK_NOT_STARTED`) of a lease under the reservation
   (`lease_hand_back_generation` equal to the lease's generation), whose step is still `LEASED` to that lease, that the server
   refuses for a reason of the fleet, MUST record nothing (no journal event, no failure, no change of the step row) and MUST answer
   `STALE_ATTEMPT`, so the lease lapses and the lost-lease pass ends it with `expire_attempt` as today (Architecture decision 0098
   decision 6: an unknown outcome, consultation or review). It is counted
   `worker_leases_undelivered_total{cause="NOT_STARTED", outcome="lapsed"}`, or `outcome="disabled"` for the switch.
2. The reasons of the fleet are exactly: `delivery-acknowledgement-v1` not recorded for engines, or the lease claimed before its
   current recording; `lease-withdrawal-v1` not recorded, or `lease-withdrawal-v2` not recorded for both process kinds; the engine's
   switch off, for an unmarked lease or a lease whose mark does not count; the lease claimed on another incarnation of the database
   server (`lease_claim_incarnation` other than the current `pg_postmaster_start_time()`); and a lease that carries no reservation
   (`lease_hand_back_generation` not its generation, as when an engine of the floor build claimed it) while the holder's stored
   session minor is 8 or more. For the last, the session row is read, which Architecture decision 0099 decision 9 forbids for a withdrawal because
   registering again overwrites it; it is safe here because a lapse never withdraws (round 16).
3. The ordinary retryable failure, with the host's `retryable` flag, MUST remain only for the reasons of the lease: the step
   `RUNNING`, a capability served to the lease (also counted and audited as a protocol violation), and the cap of four withdrawals
   per attempt reached (Architecture decision 0098 decision 7). A hand-back from a session below 1.8, whose code is not reserved, is
   an ordinary failure as the base decides. The reasons MUST be judged in this order, the first that holds deciding (round 16): the
   step no longer held by the lease (its fence misses: the stale answer of any report); the step `RUNNING` under the lease, or a
   capability served (the ordinary failure; a capability also a violation, Decision 006-053); the reasons of the fleet of rule 2
   (the lapse, counted `disabled` when the switch is off for the lease, whatever other reason holds with it, as the base's gate
   reads the switch first, and `lapsed` otherwise); the cap (the ordinary failure).
4. The expiry is not a reason of the lease: a hand-back under the reservation that lands after the lease's expiry, before the pass
   has ended the lease, MUST be withdrawn while the reservation is honoured and MUST be answered `STALE_ATTEMPT` per rule 1
   otherwise, whether the lease is marked or not. The pass and such a hand-back are ordered by the step row's lock and, for an unmarked lease, by the run or path lock that both
   take first (round 16).
5. An engine of the floor build clamps the holder's minor to its own ceiling (Decision 006-059 (3)), so, as built, a 1.8 hand-back
   that reaches it is an ordinary failure. This decision makes a request of that build (round 16; request (10) of Decision
   006-059, built by the follow-up of pull request #260 as Task 006-074, decision D006-099): in `hand_back_verdict`, a session whose stored
   minor, before the clamp, is 8 or more while the clamped minor is below 8 MUST decide the lapse of rule 1 instead of an ordinary
   failure; the clamped minor alone still decides a withdrawal, so that build never withdraws a 1.8 hand-back.

## Considered options

1. **The lapse (chosen).** The outcome of an unknown lease, ending in `expire_attempt`: an `Activity` goes to a review an operator
   can retry, as the base does before the activation. Never less safe than the ordinary failure: it withdraws nothing, so D006-064's
   worry about hiding a capability served by an engine that did not record it does not arise, and the step's outcome is decided by
   the same path as any lease the host never reported. It changes no wire, no SDK and no column: the host already ends a hand-back
   quietly on `STALE_ATTEMPT` (`sdks/SPEC.md` §6.2), and the verdict function already has the variant (`HandBackVerdict::Lapse`).
2. **The ordinary failure (Architecture decision 0099 decision 9 as written).** A known outcome, but it spends the attempt of work
   that never ran, and a one-attempt `Activity` fails for good: a regression against pull request #260 on every fleet between Task
   006-057 and the activation, and after every rollback's removal. Rejected.
3. **A withdrawal regardless of the fleet.** Gives the attempt back, but is unsafe before the replay floor is recorded (an older
   replica could meet an event it cannot replay, the reason the requirements exist) and, in a mixed fleet, could hide a capability
   an engine of the floor build served without recording it. Rejected.

## Consequences

Good: a stopping 1.8 host costs no attempt and no permanent failure on a fleet that has not activated, or has removed, delivery
acknowledgement; the rule is one sentence for every reason of the fleet, the same as the base's; the case of another server
incarnation, where the capability record may have been lost, is never a withdrawal.

Bad: in that window a one-attempt `Activity` handed back goes to a review instead of a retry, which needs an operator; the lease
holds its slot until its expiry and the pass; a host's probe after such a stale hand-back finds the lease still held (it answers
`ACKNOWLEDGED`, counted `recovered` for a mark that does not count, or `CANCELLED`), so the host hands back once more, runs nothing,
and does not count the call (`sdks/SPEC.md` §6.2). A failover during a hand-back (Gap 006-047) now ends in that lapse rather than in
an ordinary failure.

Compatibility with runs in flight: nothing written changes meaning; a lease that lapses keeps today's meaning. In a mixed fleet,
and after a rollback, a hand-back that reaches the floor build lapses once that build has request (10) (rule 5), and one that
reaches this build for a lease the floor build claimed lapses by rule 2; until the follow-up is released, a floor build without
request (10) still answers it with the ordinary failure, the residue of a rolling upgrade or a rollback.

Rollback: nothing to roll back on its own; the rollback of `stream-completion.md` ("Rollback") applies, and after its removal of
the requirement every 1.8 hand-back that this build receives lapses by this decision.

## Evidence

`specs/006-sdk-multilanguage/research.md` Decision 006-067, D006-080, D006-089, Decision 006-064 and Decision 006-045, and Gap
006-033 and Gap 006-047; `specs/006-sdk-multilanguage/stream-completion.md`, "The hand-back at 1.8" and "The withdrawals";
`sdks/SPEC.md` §6.2; the base's verdict function `hand_back_verdict` (`crates/application/src/worker/withdrawal.rs`) and its use
in `crates/application/src/worker.rs`; the round-15 and round-16 architecture reviews.
