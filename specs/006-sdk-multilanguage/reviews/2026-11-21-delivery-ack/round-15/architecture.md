Round 15 has no Blockers and 1 Major. It has 5 Minors and 8 Nits. Every finding has a disposition by the coordinator, delegate
(OD-2026-11-24-4), applied on the branch `docs/delivery-acknowledgement` (pull request #264) and recorded below with each finding.

# Architecture review, round 15: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**What blocks:**
- **Merging the design:** Major 1, because the design would bring back, on the day 1.8 is offered, the regression that D006-080
  and D006-089 removed from pull request #260.
- **Task 006-050:** Major 1 and Minor 3 settle its verdict; Minor 4 moves a case into it.
- **Task 006-052:** nothing after Minor 4.

**Disposition, in short:** Major 1 is settled by a new decision, Decision 006-067, and a new record, Architecture decision 0145
"At protocol 1.8, a hand-back the fleet cannot honour lapses", which supersedes decision 9 of Architecture decision 0099 for that
case only. The coordinator named the record ADR-0135; that number, and 0136 and 0137, are already taken on the local branch
`feature/022-step-policies` (worktree `sagat-wt-022-policies`), and 0143 and 0144 in `sagat-wt-product-surface`, so the record
takes 0145, the next number above every one in use (rule 1 of `docs/adr/README.md`). Every Minor and Nit is applied as the
coordinator decided.

---

## Major

### Major 1. A 1.8 hand-back that is not honoured is an ordinary failure in the design, a lapse on the base

**Where:**
- Architecture decision 0099 decision 9 (about lines 149 to 151), its Consequences (about line 280) and its Compatibility (about
  lines 298 and 299).
- `stream-completion.md` about lines 359 to 365; the rows of "The withdrawals" at about lines 699, 701 and 704; the failure rows
  at about lines 924 and 933; the acceptance matrix at about lines 997 and 1006; the rollback, steps 1 and 2 (about lines 862 to
  866: step 1 says lapse, step 2 says ordinary failure).
- `research.md` Decision 006-064 (about lines 1156 to 1158) and Decision 006-045 (about lines 861 and 862).
- `tasks.md` Task 006-050's evidence (about lines 301 to 322) and the mixed-fleet case of Task 006-066.
- `sdks/SPEC.md` about lines 432 to 442.

**What is wrong:** the design says a hand-back whose reservation does not count is an ordinary retryable failure, which spends the
attempt. The base (pull request #260, D006-089, Architecture decision 0098 decision 5 as built) answers `STALE_ATTEMPT`, records
nothing and lets the lease lapse when the fleet does not withdraw. In the window between Task 006-057 offering 1.8 and the
activation of `delivery-acknowledgement-v1`, and again after a rollback's removal, every stopping 1.8 host hands back; as an
ordinary failure a one-attempt `Activity` fails for good, where pull request #260 sends it to a review.

**Disposition (delegate, OD-2026-11-24-4): fixed.** New Decision 006-067 and new Architecture decision 0145, which supersedes
Architecture decision 0099 decision 9 for that case only. The rule: at 1.8, a hand-back of a lease under the reservation, still
`LEASED` to it, that is refused for a reason outside the lease records nothing and answers `STALE_ATTEMPT`, counted `lapsed`
(`disabled` for the switch). The reasons of the fleet: `delivery-acknowledgement-v1` not recorded, or the lease claimed before its
current recording; `lease-withdrawal-v1` or `lease-withdrawal-v2` not recorded; the switch off for an unmarked lease; the lease
claimed by another incarnation of the database server (a lapse, because it is a reason of the fleet, not of the lease). The
ordinary failure stays only for the reasons of the lease: the step `RUNNING`, a capability served, the cap of four. Decision
006-067 weighs the ordinary failure (spends the attempt, a regression against #260), the lapse (an unknown outcome that ends in
`expire_attempt`, never less safe, and it withdraws nothing, so Decision 006-064's worry about hiding a served capability does not
arise) and a withdrawal regardless (unsafe before the floor). Applied:
- Architecture decision 0099: its status line says "decision 9 partly superseded by Architecture decision 0145"; decision 9, the
  clauses of decisions 2 and 7 that repeat it, the Consequences and the Compatibility carry a dated pointer to the new record (the
  record is accepted, so its text is not rewritten, rule 2 of `docs/adr/README.md`). `docs/adr/README.md` lists 0145 and the new
  status of 0099.
- `stream-completion.md`: rule 5 of "Leases that never reach a host"; "The hand-back at 1.8" (the reservation bullet, the unmarked
  lease, a new bullet "A refusal for a reason of the fleet lapses"); "The marks" (a mark that does not count); the withdrawals'
  rows (the row at the cap split from the row with a requirement missing; the mark that does not count; the unmarked 1.8 lease);
  the host's item 4 and probe; rollback steps 2 and 3; the failure rows (the repeated acknowledgement under the margin, the
  failover, the removal, the switch off, another server); the acceptance rows "Hand-back", "Fallback" and "Released v0.2".
- `research.md`: Decision 006-064, Decision 006-045 and Decision 006-051 edited and marked "(round 15)"; Gap 006-047 (a hand-back
  once the promotion has come lapses); the probe's gloss in Decision 006-065.
- `tasks.md`: Task 006-050's description and evidence rewritten with root and scoped cases for each reason of the fleet and of the
  lease (the scoped case also closes the item of Gap 006-033 "the scoped half of D006-089 is covered by reading"), the base's test
  `a_hand_back_on_a_fleet_that_does_not_withdraw_leaves_the_lease_to_lapse` kept green through Task 006-050; the mixed-fleet case
  of Task 006-066; the 18 s failover case of Task 006-058.
- Host side: the gloss "because the pass won" (`stream-completion.md`, the host's fallback; Decision 006-065) and "the lease is no
  longer this host's" (`sdks/SPEC.md` §6.2) now also mean "left to lapse"; Task 006-059 gains the unit case: a hand-back answered
  `STALE_ATTEMPT` with the lease left to lapse, the probe answered `ACKNOWLEDGED` (counted `recovered`) or `CANCELLED`, one more
  hand-back, nothing run, the call not counted.
- What is left, recorded in Decision 006-067 and Architecture decision 0145: an engine of the floor build clamps the holder's minor,
  so a 1.8 hand-back that reaches it in a rolling upgrade or after a rollback is still an ordinary failure.

---

## Minors

### Minor 1. Restatements that lost D006-080 and D006-089

**Where:** `stream-completion.md` about lines 350 to 354; Architecture decision 0099 about lines 290 and 291 and `research.md` about
lines 1107 to 1109 (the end of Decision 006-059: "Below 1.8 a hand-back is an ordinary failure... the owner weighs it", which
D006-080 closed: below 1.8 there is no hand-back, the lease lapses); Architecture decision 0099 about line 4 and `research.md`
about lines 786 to 794 cite only D006-030 to D006-039 as the amendments; `research.md` about line 794, "the requests are still to
be made on that branch", is answered by D006-090.

**Disposition (delegate): fixed.** Each place now cites D006-080 to D006-093 and states the base's current rule (below 1.8 no
hand-back and a lapse; from 1.8 on a fleet that does not withdraw, `STALE_ATTEMPT` and a lapse; otherwise the ordinary failure),
marked "(round 15)"; the introduction of the delivery acknowledgement part of `research.md` says what D006-090 answered.

### Minor 2. The "Refined by Architecture decision 0099" pointers are missing from Architecture decision 0098 decisions 5 and 8; five places give five different times

**Disposition (delegate): fixed in this pull request.** Architecture decision 0098 decision 5 gains its pointer (the reservation
read from the lease, honoured only while `delivery-acknowledgement-v1` is recorded, refusals for a reason of the fleet keeping
D006-089, as Architecture decision 0145 says) and decision 8 its own (the switch does not reach a marked lease; `lease-withdrawal-v2`
is the floor for the causes of 1.8). Architecture decision 0098's Rollback (about line 106), D006-090 (4), Architecture decision
0099's Refines line (about line 5), Decision 006-047 and Decision 006-059 (4) now say "done in pull request #264"; the item is
dropped from Task 006-046; Architecture decision 0099's Refines line also names decision 5's switch clause.

### Minor 3. The item of Gap 006-033 "a 1.8 hand-back retried past the lease's expiry but before the reaper is an ordinary failure" is not settled for unmarked 1.8 leases

**Where:** the unmarked kinds `short_deadline`, `declined`, `flagged_before` and `switch_off`.

**Disposition (delegate): fixed.** Withdrawn while the reservation is honoured, otherwise `STALE_ATTEMPT` per Decision 006-067:
the expiry is not a reason of the lease. Task 006-050 has a root and a scoped case 1 ms past the expiry with the pass held, for a
marked lease and for each unmarked kind; the item of Gap 006-033 is closed by name; `stream-completion.md` ("The hand-back at
1.8", the unmarked 1.8 lease's row of "The withdrawals") says so.

### Minor 4. Precondition (a) lets Task 006-052 proceed, but one of its cases needs Task 006-050's verdict

**Where:** `tasks.md` about lines 355 and 356, "`WriteBlob` root and scoped followed by a hand-back, an ordinary failure and a
violation".

**Disposition (delegate): fixed.** The case moved to Task 006-050, and precondition (a) names it; (a) is re-dated to pull request
#260's final head (2026-11-24, with D006-092 and D006-093; still only request (3)); D006-090 says "(1) for the secret fence", as
(a) does.

### Minor 5. The design's new test seams have no fitness function

**Where:** the adapter hook that reads `synchronous_commit` (`tasks.md` about lines 287 and 288), the per-connection counter's hook
(about lines 278 and 279, and 418 and 419), the compiled minor raised for tests (`stream-completion.md` about line 606), any
barrier in `AcknowledgementTx`.

**Disposition (delegate): fixed.** `stream-completion.md` gains a paragraph "Test seams": each goes behind the `test-support`
feature of `sagat-application` (checked by `test_support_errors` in `scripts/check-architecture.py`) or is a probe refused in
production by `test_probe_errors` (D006-093). Task 006-049's evidence gains a negative case of each check; Task 006-058's hook
points to it.

---

## Nits

1. `worker_lease_marks_skipped_total` is counted after the commit, as the marks are; Task 006-047's matrix lacks a marked claim
   whose consumer goes before the commit (D006-092). **Fixed:** "The marks" and "Observability" say so; Task 006-047's matrix has the
   case (no mark, neither counter moves), root and scoped.
2. Task 006-057 does not name the removal of the compile-time assert at `withdrawal.rs:28-31` and of the session-minor path
   (`lease_holder_minor`) once Task 006-050 reads `lease_hand_back_generation`. **Fixed** in Task 006-057.
3. The flag `withdrawn_on_hand_back` of a `NOT_SENT` withdrawal is not stated. **Fixed:** it is `false` (the marks' table and Task
   006-050's evidence).
4. `outcome="lapsed"` has more than one meaning. **Fixed:** "Observability" defines one: the outcome says why a withdrawal was or
   was not written, never what followed.
5. `sdks/SPEC.md` about line 816, the 1.8 row says "put to the owner (Owner question 006-002)", which OD-2026-11-21-6 answered.
   **Fixed.**
6. Architecture decision 0098's status says round 3 was "the final review round" while its Decided-by line cites the final review
   of 2026-11-24 (round 4, D006-093). **Fixed:** the status says the final review closed on 2026-11-24 with no Blocker and no Major
   (round 4, recorded in D006-093); its Evidence line adds D006-020 to D006-028 and G006-020 to G006-022.
7. `research.md` about lines 427 and 428 say "040 to 061". **Fixed:** "040 to 079", the design using 040 to 067 now that Decision
   006-067 exists (the coordinator's "040 to 066" was written before it); the introduction of D006-080 says 067 too.
8. `sdks/SPEC.md` about line 441, "it retries it as an ordinary failure". **Fixed:** "the server records it as an ordinary failure".

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0145-at-protocol-1-8-a-hand-back-the-fleet-cannot-honour-lapses.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/README.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/withdrawal.rs (the base's verdict, lines
  190 to 270)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs (the hand-back's use, lines 2520 to 2560)
