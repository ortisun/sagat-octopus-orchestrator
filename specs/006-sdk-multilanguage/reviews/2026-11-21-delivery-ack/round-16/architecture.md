Round 16 has no Blockers and no Majors. It has 4 Minors and 4 Nits, all about the round-15 fix (Decision 006-067, Architecture
decision 0145). Every finding has a disposition by the coordinator, delegate (OD-2026-11-24-4), applied on the branch
`docs/delivery-acknowledgement` (pull request #264); Minors 1 to 3 are one decision, Decision 006-068.

# Architecture review, round 16 (targeted): delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**What blocks:** nothing blocks merging the design. Minors 1 to 3 change the verdict that Task 006-050 builds and must be applied
before it starts; Minor 4 belongs to Task 006-046's runbook work.

**Number:** Architecture decision 0145 stays; 0145 is free on the remote branches.

---

## Minors

### Minor 1. A lease the floor build claimed for a 1.8 session, handed back to this build, is an ordinary failure

**Where:** Architecture decision 0145 decisions 3 and 5 (about lines 55 to 63); Decision 006-067 "What it leaves" (`research.md`
about lines 1296 to 1298); `tasks.md` about line 357; Task 006-066 (about lines 552 and 553).

**What is wrong:** an engine of the floor build claims without writing `lease_hand_back_generation`. In a mixed fleet a 1.8
session can hold such a lease; handed back to this build it carries no reservation and so is an ordinary failure, the regression
of Decision 006-067 in another form.

**Disposition (delegate, OD-2026-11-24-4): fixed, Decision 006-068.** A reason of the fleet: when the lease has no reservation and
the holder's stored session minor is 8 or more, the verdict is the lapse (`STALE_ATTEMPT`, nothing written, counted `lapsed`).
Reading the session row is safe here because a lapse never withdraws. Applied to Architecture decision 0145 (rule 2),
Decision 006-067 (a pointer) and Decision 006-068, `stream-completion.md` ("The hand-back at 1.8", the floor bullet and the fleet
bullet; a new row of "The withdrawals"; the meaning of `lapsed`), `sdks/SPEC.md` §6.2, Task 006-050 (root and scoped) and Task
006-066 (the stream on a floor engine, the hand-back to this build: a lapse, root and scoped). A consequence recorded in Decision
006-068 and Task 006-050: a hand-back of a lease claimed at 1.7 while the session row now says 1.8 is a lapse too, where it was an
ordinary failure; and Task 006-057 keeps the session-minor path (`lease_holder_minor`) to decide that lapse alone, never a
withdrawal, instead of removing it.

### Minor 2. After a binary rollback to the floor, sessions stored at 1.8 hand back and each spends the attempt

**Disposition (delegate): fixed, request (10) of Decision 006-059**, built by the follow-up of pull request #260 (unbuilt, so the
request costs nothing): in `hand_back_verdict`, a session whose stored minor before the clamp is 8 or more while its clamped minor
is below 8 decides `Lapse` instead of the ordinary failure; the clamped minor alone still decides `Withdraw`. (The finding's words
were "whose clamped minor is below the build's ceiling"; the clamped minor equals the ceiling in that case, so the text reads
"below 8", the minor that reserves the code.) Applied to Architecture decision 0145 rule 5 (now a request to that build) and its
Compatibility, `stream-completion.md` (the floor bullet and rollback step 3), Decision 006-055, Decision 006-059 (the new request
(10)), precondition (a) of `tasks.md` (the request belongs to the follow-up; the "Follow-up of pull request #260" tasks are not on
this branch) and Task 006-066 (a session stored at 8 handing back on the floor: a lapse with request (10), the ordinary failure
without it, reported).

### Minor 3. The order of the reasons is not stated

**Disposition (delegate): fixed, Decision 006-068.** The first that holds decides: the step no longer held by the lease (stale);
a capability served (the ordinary failure, counted as a violation, Decision 006-053); the fleet's reasons (the lapse, `disabled`
when the switch is the reason, `lapsed` otherwise); the cap (the ordinary failure). Stated in Architecture decision 0145 rule 3
and in `stream-completion.md` ("The hand-back at 1.8", the fleet bullet); Task 006-050's unit test of the pure function covers the
pairs. Two readings, recorded with the text: the step `RUNNING` under the lease is judged with a capability served (it is a
reason of the lease, and the lease still holds the step, so it is not stale); and when the switch is off together with another
reason of the fleet the count is `disabled`, because the base's gate (`gate_outcome`) reads the switch first.

### Minor 4. The runbook does not name the fleet reasons of 1.8 or the cost of the window before the activation

**Disposition (delegate): fixed.** Task 006-046's runbook list (`tasks.md`) and the `lapsed` row of `docs/runbooks/lease-withdrawal.md`
name the reasons (a lease claimed before the current recording, on another server incarnation, or without reservation from a 1.8
session) and the cost of the window (every stopping 1.8 host sends its `Activity` leases to a review), with the advice to activate
`delivery-acknowledgement-v1` soon after the whole fleet runs the build.

---

## Nits

1. `stream-completion.md` about lines 375 and 376, "the safe direction". **Fixed:** "safe against a withdrawal, at the cost of the
   attempt".
2. Architecture decision 0145 rule 2 names only "a lease whose mark does not count" for the switch, and rule 4 orders the pass and
   the hand-back by the step row's lock alone. **Fixed:** rule 2 names "an unmarked lease or a lease whose mark does not count";
   rule 4 adds that for an unmarked lease they are ordered by the run or path lock both take first.
3. `tasks.md` about line 341 does not say how the base's test stays green. **Fixed:** its claim runs at 1.8 under the ceiling the
   `test-support` feature raises, so the claim writes `lease_hand_back_generation`; the test stays green by construction, not by
   changing what it expects.
4. `docs/adr/README.md` about lines 51 to 53 lists only the numbers with no file on `develop`. **Fixed:** it also lists 0093 to 0097
   and 0101 to 0144, with no file on this branch, as reserved or used on other branches.

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0145-at-protocol-1-8-a-hand-back-the-fleet-cannot-honour-lapses.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/README.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/lease-withdrawal.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/withdrawal.rs (`gate_outcome`, `hand_back_verdict`)
