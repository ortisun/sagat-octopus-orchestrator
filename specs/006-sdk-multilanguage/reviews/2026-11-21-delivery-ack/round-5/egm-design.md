# Design critique, round 5: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Review status: complete.** No target standard was supplied, so I reviewed against the persona's audit vectors and my round-4 findings.

**What I read:**
- the persona and my round-4 report;
- the whole round-4-to-round-5 diff, and the full design diff where the new text touches the marks, the tasks and the gaps;
- Architecture decision 0099 (decisions 2 to 12, the options, the consequences);
- `stream-completion.md`, lines 251 to 700;
- `sdks/SPEC.md` §5 and §6.2 (lines 277 to 381);
- `research.md`, Decision 006-040 to Decision 006-063 and Gap 006-040 to Gap 006-048;
- `tasks.md`, phase "Delivery acknowledgement";
- `docs/runbooks/disaster-recovery.md` in full.

**Code and drafts I checked:**
- the operations statistics query that the gauges will extend: `crates/infra-postgres/src/operations.rs`, lines 452 to 458;
- the draft 032 epoch function: `specs/032-multi-region-failover/data-model.md`, lines 99 to 115.

## Verdict

The timeline rule is the right mechanism, and it is cheap. On an ordinary PostgreSQL promotion and on an archive recovery, it is sound.

What remains is mostly at the edges:
- **Major 1.** The design claims "a promotion or a point-in-time recovery changes the timeline". From that it concludes that only a logical dump can keep the timeline. That conclusion is too narrow. Several common ways to restore or switch over keep the timeline, including one the runbook itself recommends. The owner is asked to accept a residue whose scope is understated.
- **The Minors.** How a mark of "another" timeline is defined. What happens when the timeline cannot be read. A server answer that is not bounded within the host's attempt. A recovered lease's path, which is not complete.

The bounded fallback and the move of the cap to the withdrawal command are right.

No Blockers. 1 Major, 4 Minors, 4 Nits. With Major 1 fixed, I would approve. The Minors are text fixes that can be made before acceptance.

---

## Part 1: The round-4 Majors

**Major 1 (a committed acknowledgement that is later lost): partly resolved.**
- **Promotion.** The mechanism is correct.
  - A promoted server picks its new timeline before it leaves recovery, so it accepts no write under the old one.
  - `pg_walfile_name()` names the file with the insertion timeline, and refuses to run during recovery. So a standby can raise an error, but it can never stamp a wrong value.
  - Reading the timeline from `pg_control_checkpoint()` would have been wrong: after a fast promotion it lags until the first checkpoint. The design's choice is correct.
- **Archive recovery.** The procedure of runbook §4 (`recovery.signal`, then promote) starts a new timeline at the end of recovery.
- **What is done.** The rejected operator fence is argued correctly: no operator is in time for an automatic failover. The failure rows, the drill check and the owner assumption (Gap 006-048) are there.
- **Why only partly:**
  - Gap 006-048 says only "a logical dump" keeps the timeline. That is not true (new Major 1).
  - The runbook step exists only as part of Task 006-054. It is not written in `disaster-recovery.md`, which is one of the documents under review, and the owner's acceptance relies on it.
  - The paths of a mark of another timeline are incomplete (Minors 1 and 4).

**Major 2 (the fallback): resolved, with a Minor residue.**
- The fallback now needs evidence from the path.
- It counts calls, not leases.
- It lasts 10 minutes, doubling to an hour, with jitter.
- It is alerted (`SagatWorkersDecliningAcknowledgement`) and counted on both sides.
- The scenario covers both a 30 s outage that must not trigger a fallback and the case that must.
- **The residue:** the server can still fail to answer within the host's 6 s attempt bound, so trouble on the server's side can still look like evidence from the path. The list of refused statuses also differs between the two texts (Minor 3).

**Major 3 (the claim-time cap check): resolved.**
- The claim reads no count, and the `LEASE` line keeps its statements.
- The withdrawal table has the two rows at the cap. `attempt_capped` is gone from the skip reasons, and `SagatLeaseAttemptCapped` is driven by the `capped` outcome.
- Lifting the expiry condition for a provisional lease's hand-back is a sound addition: nothing ran, and the pass would withdraw the lease anyway.

**Round-4 Minors:**
- Minors 1, 3, 4 and 5: resolved. For Minor 5, request (5) is now a precondition: tasks, precondition (a).
- Minor 2: resolved as to the commit. The deadline check before `acknowledge` stops a commit after the host's attempt has ended. Its residue, the late answer, is part of new Minor 3.

**Round-4 Nits:**
- Nits 1, 3 and 4: resolved. Nit 4 is now a constant inside a trigger function, with the static check.
- Nit 2: resolved in the contract, but two summaries still say `FOR UPDATE` (new Nit 1).

---

## Part 2: Findings

### Major 1: Several common restore and switch-over methods keep the timeline, and the design names only a logical dump

**Location:**
- Architecture decision 0099, decision 2 (lines 54 to 66) and decision 12 (lines 127 to 136);
- `stream-completion.md`, "The timeline" (lines 411 to 418) and the failure rows (lines 654 to 656);
- Decision 006-062 (research.md line 620) and Gap 006-048 (line 653);
- `docs/runbooks/disaster-recovery.md`, §3 (line 36, "or your platform's snapshot tool") and §7 (lines 86 to 93);
- Task 006-054.

**What is wrong.** The rule ends marks only when the timeline changes. Each of these loses committed transactions and keeps the timeline:
1. **A restored snapshot.** This means a volume, filesystem or virtual-machine snapshot, started as it is. PostgreSQL runs crash recovery, and crash recovery never starts a timeline. Runbook §3 offers "your platform's snapshot tool" for base backups.
2. **A base backup started without `recovery.signal`.** It replays to the end of the backup by crash recovery, on the same timeline. This is the quick "restore last night's backup".
3. **A restore in a managed service whose storage is not PostgreSQL's own write-ahead log** (shared-storage services). Whether a point-in-time restore there advances the PostgreSQL timeline is not established. Task 006-044 verifies only the pinned image. On these services an unchanged timeline on failover is harmless, because they do not lose commits there. Their restores are the risk.
4. **A switch to another cluster through logical replication before it has caught up**, for example a home-made blue-green upgrade or a migration. The target has its own timeline, often 1. That equals the timeline of a primary that never failed over, which is the most common case.
5. **A primary crash with `synchronous_commit = off` (or `fsync = off`).** The crash recovery stays on the same timeline and loses the last commits. The host has already been answered `ACKNOWLEDGED`.

**Why it matters:**
- Each case brings back round-4 Major 1 exactly: a provisional lease whose acknowledgement was lost is withdrawn as `NOT_ACKNOWLEDGED`, while its handler runs.
- The owner is asked (Gap 006-048, decision 12) to accept a residue described as "a logical dump". The real list includes the platform's own suggested backup tool.
- The drill checks that the timeline changed, but only for the two disasters it creates itself, both of which change it by construction. It says nothing about the operator's tools.

**Fix:**
- **Make the runbook step unconditional.** `end_lease_marks()` becomes a step of §7 after every restore, of any kind, before step 5 starts the engines. The step is harmless: it gives every mark today's meaning and costs only the protection of the leases in flight. The timeline stays the mechanism for automatic promotion, the one case no operator is in time for.
- **Write the runbook text in this design**, not only in Task 006-054. Add to "To drill your own deployment": read `sagat_database_timeline()` before and after your own restore and failover tools; where it does not change, your procedure must run `end_lease_marks()`.
- **Rewrite Gap 006-048 with the list above**, and have the owner accept that list.
- **Make durable commit a precondition of marking.** The timeline probe also requires `current_setting('synchronous_commit') <> 'off'` and `fsync` on, and marks nothing otherwise, with a skip reason. Alternatively, the acknowledgement's settings statement raises `synchronous_commit` to at least `local` when the session has it `off`.

### Minor 1: "An earlier timeline" is not the same test as "not the current timeline", and the timeline can move backwards or be null

**Location:**
- Architecture decision 0099, decision 2 ("the offer is of an earlier timeline");
- `stream-completion.md` lines 413, 478 and 506 ("earlier"), against line 398 ("its timeline is the database's current one");
- Task 006-047 ("another timeline");
- Task 006-050 ("an earlier timeline");
- `end_lease_marks()`, which sets the column to null (line 432).

**What is wrong:**
- `pg_upgrade` starts the new cluster on timeline 1 (it resets the write-ahead log with timeline 1). A logical restore into a new cluster also starts at 1. After either, a mark from timeline 3 is of a *higher* timeline.
- `end_lease_marks()` leaves null. In SQL, `lease_offered_timeline <> current` is unknown for null.
- A test written literally as "earlier" puts both rows in neither branch. They fall to "anything else: `NOT_HELD`", so the host drops a lease that was safe to run, and it lapses into a review.

**Fix:**
- Define it once. The offer counts only when `lease_offered_timeline` is not null and equals the current timeline. Every other value, null included, is "another timeline".
- Replace "earlier" everywhere.
- The parity test covers null, a lower value and a higher value.

### Minor 2: A timeline that cannot be read becomes an error in shared statements, not "no mark counts"

**Location:**
- `stream-completion.md`, "The timeline" (lines 411 to 418);
- the gauges (lines 441 to 442) in `crates/infra-postgres/src/operations.rs`, lines 452 to 458;
- `lock_matched` (line 472);
- the withdrawal re-read;
- Task 006-044.

**What is wrong:**
- **The operations snapshot can fail.** The design expects some databases where the runtime role cannot call the write-ahead log functions (`no_timeline`). But the gauges `worker_leases_marked`, `worker_leases_provisional` and `worker_leases_provisional_overdue` are `FILTER` clauses of the single leased-steps query of the operations snapshot. That query propagates its error with `?`. If the "mark counts" macro calls the function, the whole snapshot fails on every scrape on such a database, before any lease is marked.
- **The pass can stall.** If the privilege disappears after marks were written, the withdrawal re-read fails instead of deciding. Those leases stay `LEASED` past their expiry, where today they would lapse.
- **The acknowledgement fails** with a retryable error instead of an answer.
- **A read-routing pooler** can send the plain `SELECT` of the probe to a standby, where it raises an error. Marking then flips with each registry heartbeat. That is harmless for exactness, but silent.
- **No alert.** No alert reads the `no_timeline` skips. That is the round-4 pattern again: silence looks like health.

**Fix:**
- Write `sagat_database_timeline()` as `LANGUAGE sql STABLE` with no exception block. A PL/pgSQL handler opens a subtransaction on each call, and past 64 in one transaction that overflows the subtransaction cache.
- Guard it so it returns null instead of raising: `CASE WHEN NOT pg_is_in_recovery() AND has_function_privilege(...) THEN ... END`. A null current timeline means no mark counts, which is today's meaning.
- Evaluate it once per statement, as a scalar subquery.
- Test as a role without the privilege: the operations snapshot, the pass and the acknowledgement still work.
- Alert on `worker_lease_marks_skipped_total{reason="no_timeline"}` increasing while the requirement is recorded.

### Minor 3: The server's answer is not bounded within the host's attempt, so the server's own trouble can still trigger the fallback

**Location:**
- `stream-completion.md`, "The server's acknowledgement", steps 1 and 7 (lines 450 to 453), and host item 5 (line 582);
- Decision 006-043;
- `sdks/SPEC.md` §6.2, lines 372 to 375.

**What is wrong:**
- **The call can outlast the host's bound.**
  - The 3 s deadline is checked once, before `acknowledge`. About eight statements each have up to 1 s.
  - Waiting for a pooled connection happens before the transaction, outside the deadline.
  - The commit can wait on a synchronous standby without limit.
  - So in a slow database, with waits on the binding lock or the membership lock, a call can go unanswered past the host's 6 s bound. Heartbeats, which take fewer locks, are still answered. The third trigger then counts it as "a path that swallows this one call".
  - In a fleet-wide slowdown, every host can fall back for about 10 minutes, handing back and registering again during the incident. Because the fallback is now bounded, this is a Minor.
- **"A deadline or store status from the server"** names no status an SDK can recognise.
- **The two lists of refused statuses differ.** `sdks/SPEC.md` counts any status §5 does not repeat, which includes a non-retryable `INTERNAL` caused by a server bug. `stream-completion.md` lists four.

**Fix:**
- Start the call's deadline when the request arrives, so it covers waiting for a connection.
- Before each statement, set `statement_timeout` to the smaller of 1 s and the time left, or cancel the statement in flight when the deadline passes.
- At the deadline, answer one named status (for example `UNAVAILABLE` with a retry hint), and name it in both texts.
- Make the refused list closed and identical: `PERMISSION_DENIED`, `INVALID_ARGUMENT`, `FAILED_PRECONDITION`, `NOT_FOUND`.
- Test: an exhausted pool plus a slow lock past the deadline is answered within about 3.5 s.
- State that a commit waiting on a synchronous standby is outside the bound, and that the host's give-up covers it.

### Minor 4: The path of a mark of another timeline is incomplete, and partly more costly than it needs to be

**Location:**
- `stream-completion.md`, step 5 of the acknowledgement (lines 476 to 479), the withdrawal table (line 506) and the failure row (line 654);
- Task 006-050 ("a mark of an earlier timeline ... an ordinary failure"; `NOT_SENT` listed among the withdrawals of a marked lease).

**What is wrong:**
1. **The `recovered` verdict has no time or stop condition.**
   - The runbook allows up to five minutes for a failover. Most repeated acknowledgements reach the new primary with the lease near or past its expiry.
   - Answered `ACKNOWLEDGED` with `lease_time_left_ms` at 0, the host starts a handler on a lease the pass is about to expire. The result is a review with a handler running.
   - A recovered lease whose step has `cancel_requested` is also answered `ACKNOWLEDGED`.
2. **A hand-back of a recovered lease spends the attempt.** A hand-back is the host's word that nothing started, and what the database lost does not change that word. Decision 12's first assumption already accepts it. Architecture decision 0098 withdraws the same hand-back for an unmarked 1.8 lease (`lease_hand_back_generation` is set).
3. **A `NOT_SENT` withdrawal of a recovered lease would be refused.** `NOT_SENT` comes from the server's own knowledge that the frame never left the process.

**Fix:**
- For every withdrawal, treat a mark of another timeline as *unmarked*: Architecture decision 0098's rules, with its expiry condition and its switch.
- For the acknowledgement, answer `ACKNOWLEDGED` only with one receive bound left and no stop asked; otherwise answer `CANCELLED`, so the host hands the lease back and nothing runs.
- Add both to the unit test of the pure function.

### Nits

1. `docs/adr/README.md` line 51 and Decision 006-040 (research.md line 443) still say `FOR UPDATE SKIP LOCKED` and "the clock sampled after the locks". The contract now says `FOR NO KEY UPDATE OF steps SKIP LOCKED`, with the clock as a column.
2. Decision 006-062 should record why the timeline is read from `pg_walfile_name(pg_current_wal_lsn())` and not from `pg_control_checkpoint()`: the second lags after a fast promotion. It should also record that the function refuses to run during recovery. Otherwise someone will "simplify" it.
3. Draft 032 defines a second database epoch, `sagat_region_epoch()`. Add a cross-reference in Decision 006-062. A region activation promotes, so the timeline already ends the marks, and draft 032 need not stamp its epoch on them.
4. The first decline period (8 to 12 minutes) is shorter than the alert's 15 minutes, so a single fleet-wide fallback never alerts. Add a rate alert on `worker_registrations_declining_total`, for example above 10% of live sessions in 15 minutes, or state that this is intended.

---

## Part 3: Answers to the questions

**Is the timeline rule sound?**

Yes, for automatic promotion and for archive recovery:
- the timeline changes before any write is possible;
- the value read is the insertion timeline;
- the function cannot return a wrong value on a standby.

It is not sufficient for restores that keep the timeline (Major 1).

What a mark of another timeline does on every path, as written:

| Path | Outcome as written |
|---|---|
| Acknowledgement | `ACKNOWLEDGED`, counted `recovered`, with no time check (Minor 4) |
| Heartbeat | Served, no violation. This removes the false alerts of round 4 |
| Capability | Served and recorded |
| Completion | As today |
| Hand-back | An ordinary failure (Minor 4) |
| The pass at a deadline | `expire_attempt`, today's meaning |
| `NOT_SENT` | Refused (Minor 4) |
| Gauges, the rollback drain, the refusing `down` | Not counted, provided they all use the macro (Minor 2) |

A claim lost on failover is now safer than today: the acknowledgement finds no row and answers `NOT_HELD`, so nothing runs.

What can go wrong:
- **Transaction-pooling poolers:** harmless. The value belongs to the whole server and is read inside the statement.
- **Read-routing poolers:** the probe can land on a standby and fail (Minor 2).
- **A pooler that fails over transparently:** fine.
- **An old primary that was not fenced** keeps its timeline. That is split brain, covered by the runbook's fencing.
- **Planned switchovers** (monthly maintenance, rolling minor upgrades) end protection for the leases in flight. This is correct and counted as `recovered`, but worth one sentence in Gap 006-047.

**Is the bounded fallback right?**

Yes:
- the evidence-based triggers;
- counting by call;
- the jittered, doubling period that resets on success;
- the alert;
- the hand-back of flagged leases received while declining.

A path that keeps swallowing the call costs, per cycle, up to the give-up time and one withdrawal per flagged lease. That is acceptable.

The residue is Minor 3 and Nit 4. The third trigger is the most expensive host rule across six languages. It earns its place: without it, a path that swallows the call burns attempts silently. But it is only correct if the server always answers within its bound.

**Simpler or more complex than needed? What round 5 adds against round 4:**

| Area | Added | Removed |
|---|---|---|
| Storage | 1 column (`lease_offered_timeline`); 2 functions (`sagat_database_timeline()`, `end_lease_marks()`); 1 trigger with its function | 1 table |
| Fleet control | 1 field (`requires`); 2 typed refusals; 1 static check of the pair | |
| Server | A probe per registry heartbeat; a call deadline; 1 shared binding-check function over a narrow trait; the audit gate moved to the application layer; the capability record committed before the serve (root `WriteBlob`'s check transaction now commits) | The claim's cap check |
| Hand-back | 1 rule (expiry lifted for provisional leases) | |
| Signals | 1 verdict (`recovered`); 1 outcome (`capped`); 1 skip reason (`no_timeline`); 1 counter; 1 alert (five in all) | 1 skip reason (`attempt_capped`) |
| Host | About 7 rules: the decline period with jitter, doubling and reset; the third trigger's condition; closing the receive; handing back while declining; `RETRY`'s own backoff; the corrected first heartbeat; the stop within its timeout | |

Overall it is slightly larger than round 4, at about 14 host rules. Every addition answers a named failure. The timeline is the cheapest mechanical answer to durability: one column, one function call, no extra statement.

Still marginal, as before: `withdrawn_lease_token_hash` and the `provisional_overdue` gauge. Nothing new fails to earn its place.

**What fails first in production?**
1. **On a database where the runtime role cannot call the write-ahead log functions** (a managed service or a hardened role): the operations snapshot fails at the first scrape after the migration, if the gauges call the function unguarded (Minor 2).
2. **The first database slowdown with lock waits:** answers arrive after 6 s, and hosts fall back for about 10 minutes with a storm of hand-backs and registrations. This is bounded and alerted (Minor 3).
3. **The first failover, planned or not:** the leases in flight become `recovered`. Repeated acknowledgements answered with no time left start handlers on expiring leases, and hand-backs spend attempts (Minor 4).
4. **The first restore with a snapshot tool, or a managed restore that keeps the timeline, done without the step:** a false "nothing ran" (Major 1). This is rare, but it is the worst outcome in the design.
5. **After a major-version upgrade, if "earlier" is implemented literally:** leases are dropped as `NOT_HELD` (Minor 1).

## Files referenced

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/README.md (line 51)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 251 to 700)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (lines 277 to 381)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (lines 438 to 663)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (phase "Delivery acknowledgement")
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/operations.rs (lines 452 to 458)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/032-multi-region-failover/data-model.md (lines 99 to 115)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-4/egm-design.md
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round4-to-5.diff
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round5.diff
