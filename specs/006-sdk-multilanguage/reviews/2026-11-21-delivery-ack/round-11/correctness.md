**Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 11**

## Verdict

Round 11 fixes the defect I reported in round 10: an `OK` answer from before the database failed no longer counts toward the early give-up. But the new evidence rule has a mirror-image defect. The first `OK` answer, or the first frame, after the database comes back now gives up early every flagged lease that is in its retry backoff at that moment. After a promotion, that hand-back is an ordinary failure that spends the attempt (Major 1).

Dropping the fallback also dropped, from the record's header, its Rollback and precondition (a), the rule that no build of pull request #260 is released without the cause. The rollback floor and the engines-only requirement both depend on that rule (Major 2).

**Counts:** 0 Blockers, 2 Majors, 7 Minors, 7 Nits.

Nothing was built or run. Claims about the code come from reading `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`.

## Status of the round-10 findings

### Major 1 from my report (an `OK` answer from before the failure counted toward the early give-up): partly resolved

- **What is fixed:** the `OK` must now answer a call sent after this lease's most recent failed attempt, with at least one attempt ended. This is written in `sdks/SPEC.md:380-385`, item 4 (`stream-completion.md:729-741`), Decision 006-065, the failure row and Task 006-058. The scenario I gave (a heartbeat answered 20 ms before the failure) no longer gives the lease up.
- **What is left:** the rule has no requirement on this lease's own attempts after the evidence. So the database's recovery is itself taken as the evidence that the acknowledgement is failing (Major 1 below).

### Minors from my report

| Round 10 | Status | Reason |
|---|---|---|
| Minor 1: the 18 s failover case | Partly resolved | It now expects the outcome of Gap 006-047, but writes that outcome as "an ordinary failure, no review". The lease can also lapse into a review (Minor 2). |
| Minor 2: `$flag` per row | Partly resolved | The binding is now per call, but it is keyed by node. A scoped row is identified by node and instance path (Minor 3). |
| Minor 3: the earlier deadline in `sdks/SPEC.md` | Text added, cannot be built | `TaskLease` carries no timeout and no attempt deadline (Minor 4). |
| Minor 4: the fallback against Task 006-057 | Resolved | The fallback is removed and Task 006-057 stays engines-only. Removing it opened Major 2. |
| Minor 5: the cap rows | Resolved | `stream-completion.md:908`, `:910`. |
| Minor 6: a heartbeat as a false positive | Resolved, but too broadly | It also silences the true positive after `NOT_ACKNOWLEDGED` (Minor 5). |

### Nits from my report

- Resolved: 1, 2, 3, 5, 6, 7, 8, 9, 10, 11 and 12.
- Nit 4 (which lease's give-up orders a call) is fixed in `stream-completion.md` and Decision 006-065, but not in `sdks/SPEC.md:396` (Nit 1).

### Other round-10 reports, in my domain

| Finding | Status |
|---|---|
| Security Major 1, architecture Major 1, design Major 1 (the fallback's floor) | The fallback is gone. But the replacement guards only Task 006-057, not the release of #260 without the cause (Major 2). |
| Security Minor 1, architecture Minor 4, design Minor 5 (a beat that returns no epoch) | Resolved. With the latch, a beat with no epoch commits `last_seen`, so the census still counts the engine. An engine whose window had not lapsed was therefore live at the census, and a lapsed one never reopens. I found no case in which a floor engine serves after an activation. |
| Security Minor 2 (the overload's privileges) | Resolved. |
| Security Minor 3 (heartbeat kind) | Resolved too broadly (Minor 5). |
| Architecture Major 2 (Task 006-046's evidence) | Resolved. |
| Architecture Minor 2 (the per-node channel) | Partly resolved (Minor 3). |
| Architecture Minor 3 (the domain against the application) | Resolved in `stream-completion.md:418-420`. The record still says "the withdrawal command" (Nit 2). |
| Architecture Minor 5 (the denominator series, the `mark` vocabulary) | Resolved. Timing of the increment is Nit 6. |
| Design Minor 2 (the evidence rule) | Parts 1 and 3 are resolved. Part 2 (a quiet host) moved into Minor 6. The rule's new edge is Major 1. |
| Design Minor 4 (the earlier deadline) | Cannot be built (Minor 4). |
| Design Minor 6 (stream credit) | Rule added; how a host observes it is not specified (Minor 7). |

## Blocker

None.

## Major

**Major 1. The early give-up now fires at the end of a failover. Every flagged lease that is in backoff when the first call after the promotion is answered `OK`, or the first frame arrives, is given up. Its hand-back after a promotion is an ordinary failure that spends the attempt.**

Confidence: confirmed in the documents.

**Where:**
- `sdks/SPEC.md:380-385`: "after the most recent of them ended, another of its calls … was sent and answered `OK`, or a frame with leases arrived … it then starts no further attempt and waits for one in flight";
- item 4 (`stream-completion.md:729-741`, "otherwise gives the lease up");
- Decision 006-065;
- the failure row (`stream-completion.md:816`);
- the Fallback row (`:911`);
- Task 006-058.

**Scenario:** a 30 s time-to-live, a 15 s failover from F to the promotion P = F + 15 s, and a host with a running task.
1. Flagged lease L is received at F − 50 ms. Its acknowledgement attempts end with `UNAVAILABLE` (`store` or `ACKNOWLEDGEMENT_DEADLINE`, each within about 4 s). Between them the host waits the §5 backoff: 200 ms doubling to 5 s, times a random factor in [0.5, 1].
2. Attempt 4 is sent at about F + 11.6 s and ends at about P + 0.1 s. L is then in backoff, about 0.8 to 1.6 s, with nothing in flight.
3. At P + 0.2 s, claims resume and a frame with leases arrives. Or the running task's heartbeat, sent after P + 0.1 s, is answered `OK`.
4. Every condition now holds:
   - more than 6 s since L's first attempt;
   - at least one attempt ended, and none was a shed;
   - evidence that came after L's most recent failure, with no failed call since.
5. No attempt is in flight, so the host gives L up and hands it back.
6. The mark belongs to the old incarnation, so the hand-back is an ordinary failure. The attempt is spent; with `maxAttempts = 1` the step fails for good.
7. Without the early give-up, attempt 5 at about P + 1 s is answered `ACKNOWLEDGED` (`recovered`, about 14 s left) and the task runs. That is what the failure row promises.

**Who is hit:** every flagged lease whose acknowledgement is in backoff at the first evidence after any change of incarnation (a promotion, or a restart of PostgreSQL). After several failures the backoff is 2.5 to 5 s against attempts of 0 to 4 s, so this is a large share of the leases in flight.

The same thing happens on the same server after a plain outage. There it costs only an extra hand-back and claim, because the mark still counts.

**Why the tests miss it:** the 30 s case ("three flagged frames in flight and a running task that heartbeats 20 ms before the failure") depends on timing. It fails only if one of the three leases happens to be in backoff when the first frame or heartbeat after the promotion lands. So it can pass on a build that follows the rule.

**Smallest fix:**
- When the condition holds and no attempt is in flight, send one more attempt at once, outside the backoff. Give up only if that attempt also ends without a verdict and without a shed.
- Put differently, the evidence must be followed by a failure of an attempt for this lease sent after it.
- This keeps the failover safe at both ends. Before the failure no `OK` counts; after the promotion the extra attempt is answered.
- Write it in §6.2, item 4 and Decision 006-065.
- Add a host unit test with a barrier: a lease in backoff when the first `OK` after a run of failures arrives sends its next attempt at once, is answered `ACKNOWLEDGED`, runs, and is not handed back.
- Make the 30 s conformance case deterministic: hold one lease in backoff across the promotion.

**Major 2. Dropping the fallback also dropped "before any build of it is released" from the record's header, its Rollback and precondition (a). A released build of #260 without the cause then lists `lease-withdrawal-v1` and is admitted, so it cannot replay `NOT_ACKNOWLEDGED`.**

Confidence: confirmed in the documents and the code.

**Where:**
- Precondition (a) (`tasks.md:142-146`, now only "go into a follow-up of pull request #260 … a released build with it … is a hard precondition of Task 006-057").
- The record's header, and its Rollback (`ADR-0099…md:272-276`: "the released follow-up is a hard precondition of Task 006-057, so no mark, and so no `NOT_ACKNOWLEDGED`, exists before the floor does … so the registry never admits a binary below that build once any withdrawal exists").
- Against these, Decision 006-059 (`research.md:786`) still says "before any build of it is released".

**Code:**
- `WithdrawalCause` has only `NotSent` and `NotStarted` (`crates/domain/src/execution/event.rs:438-443`), and the codec fails on any other cause (`crates/infra-postgres/src/event_codec.rs:544`).
- The #260 build lists `lease-withdrawal-v1` (`crates/application/src/instances.rs:140`).

**Scenario:**
1. A release (say 0.3.0) carries #260 without its follow-up. A later release has (2) and (7), which satisfies Task 006-057, and the feature is activated.
2. **Forward path:** `delivery-acknowledgement-v1` is recorded for engines only. A control plane of the earlier release lists `lease-withdrawal-v1` and is admitted while the feature is active. Control planes replay the event (Decision 006-047 says so). Every run whose journal holds `NOT_ACKNOWLEDGED` then fails to load there.
3. **Rollback:** after the removal of `delivery-acknowledgement-v1`, an engine of the earlier release is admitted too.

The Rollback's reason ("no mark exists before the floor does") is about when the events start. It does not keep out a binary below the floor that lists `lease-withdrawal-v1`.

**Why the tests miss it:** the `provisional-lease-cutover` case starts from v0.2 and rolls back to the floor. No case admits a release between the two.

**Smallest fix (either):**
- **(a)** Restore in the header, decision 10, the Rollback and precondition (a): "no build of #260 is released without (2)". Make it checkable, for example a step of the release job that fails when a tag's `WithdrawalCause` lacks a cause the events contract lists.
- **(b)** The follow-up lists a capability of its own for both kinds, for example `lease-withdrawal-v2`. The activation of `delivery-acknowledgement-v1` refuses until it is recorded for both kinds, and the replay-floor guard refuses its removal while a journal holds the cause. This is independent of release history.

## Minor

**Minor 1. "The other dormant tasks proceed without it" (`tasks.md:146`) is false. Task 006-047, Task 006-050 and Task 006-051 need requests (1), (2), (5) and (8) of the follow-up.**

Confidence: confirmed in the code.

**What the tasks need that the base lacks:**
- Task 006-050 writes the cause `NOT_ACKNOWLEDGED`, which does not exist (`event.rs:438-443`).
- Task 006-047 and Task 006-050 read `lease-withdrawal-v1` "for both kinds". `lease_withdrawal_required()` returns `FOUND` for any record (`20270208000000_lease_withdrawal.up.sql:71-76`), so it lacks request (8).
- The evidence of Task 006-050 and Task 006-051 includes the hand-back under a stop with declared secrets, which is request (1), and the cap's derivation, which is request (5).

**Why it matters:** a builder who follows the line either adds the variant and the function here, in conflict with the follow-up, or cannot build.

**Fix:** say that every task that writes or decodes the cause, or reads a requirement's kinds, waits for the follow-up's merge, and name them. Only the release is the precondition of Task 006-057.

**Minor 2. The 18 s failover case expects "an ordinary failure, no review". Gap 006-047, which the case names, says such a lease "still lapses into a review".**

Confidence: confirmed in the documents and the code.

**Where:** the Fallback row (`stream-completion.md:911`), the failure row (`:816`), Task 006-058.

**Scenario:**
1. The ordinary give-up comes at about c + 12 s, where c is the claim.
2. The hand-back is a `FailStep`, tried at most 5 times. The backoff between tries totals 0.75 to 3 s. Each try is bounded by the pool's 3 s acquire timeout (`crates/infra-postgres/src/pool.rs:32`), and fails fast when connections are refused.
3. So all five tries can end before the promotion at c + 15 s or later. The lease then lapses, and at c + 18 s the pass on the new server runs `expire_attempt`, counted `recovered`. For an `Activity` that is a review.

**Fix:** expect "an ordinary failure, or `recovered` and, for an `Activity`, a review (Gap 006-047)". Or pin the harness so that the hand-back's last try lands after the promotion.

**Minor 3. `mark_withdrawn_attempt(&node, generation)` is keyed by node. A scoped row is identified by node and instance path, so a sibling instance of the same node passes the key and the fence.**

Confidence: confirmed in the code.

**Code:**
- `upsert_scoped_instance` writes `WHERE … node = $3 … AND instance_path_nodes = $17 AND instance_path_indices = $18` (`crates/infra-postgres/src/execution.rs:7089-7090`). It runs once per touched `ExecutionInstanceId` (`:6189-6196`).
- The existing per-row effects filter by `&grant.instance == instance` (`:6952-6963`).

**Scenario:**
1. A collection whose body node X has instances [0] and [1], both `LEASED` at generation 1.
2. Instance [0] is withdrawn on its hand-back under a stop, and the drain touches [1].
3. With a node key, [1]'s flag is set to its attempt, so its next lease is unprotected. That is the safe direction, but the value is wrong.

**Why the tests miss it:** the new test uses "two `LEASED` siblings of one run", which a node key passes if they are different nodes.

**Fix:** key the method by `&ExecutionInstanceId` and generation. Add the case "two instances of one collection node at equal generations, one withdrawn, the other's flag unchanged".

**Minor 4. The give-up "from the earlier of its expiry and, when the lease carries its timeout, its attempt deadline" cannot be built: no lease carries a timeout.**

Confidence: confirmed in the code.

**Where:** `sdks/SPEC.md:376-377`, item 4 (`stream-completion.md:729-730`), Task 006-059 ("the give-up counted from the earlier deadline"), and the `REGISTRY_SILENT` row of §5 (`sdks/SPEC.md:282`, "its `timeoutMs`").

**Code:**
- `TaskLease` has fields 1 to 18 (`specs/001-platform-foundation/contracts/worker.proto:94-126`). The design adds only field 23, and §6.4 says "the lease carries no TTL".
- The grant sets the expiry without capping it at the attempt deadline (`execution.rs:7066-7068`).
- So the host learns the attempt deadline only from `attempt_time_left_ms`, which comes with `ACKNOWLEDGED`, after the give-up is no longer needed.

**Effect:** the round-10 scenario stands. With `timeoutMs` = 20 s and a 60 s time-to-live, the pass withdraws at 20 s (`NOT_ACKNOWLEDGED`, which does not set the flag) and the hand-back at 54 s is stale. When the early give-up does not apply, the attempt is claimed and flagged again, up to the cap.

**Fix:** add an attempt deadline to `TaskLease` at 1.8 (field 24, a time left in milliseconds, set only on a flagged lease with a timeout). Or cap a flagged lease's `expires_at_epoch_ms` at its attempt deadline in the frame. Or delete the clause and state the cost.

**Minor 5. The `heartbeat` kind is not alerted after any withdrawal, `NOT_ACKNOWLEDGED` included. After `NOT_ACKNOWLEDGED`, a heartbeat is the only call a handler that really ran makes.**

Confidence: confirmed in the documents.

**Where:** the failure row (`stream-completion.md:830`, "a marked lease withdrawn as `NOT_ACKNOWLEDGED` or on its hand-back … a heartbeat under its own kind, not alerted"), and `worker_calls_after_withdrawal_total{kind}` with no cause label. Item 3 (`:714`) and Decision 006-053 (`research.md:719`) limit the exemption to the hand-back, but the counter cannot tell the two apart.

**Why it matters:**
- After `NOT_ACKNOWLEDGED`, no host that keeps the protocol heartbeats. Only a lost committed acknowledgement (Gap 006-042, Gap 006-048) or a broken host does.
- On `STALE_ATTEMPT` to a heartbeat, the SDK cancels with `lease-lost` and "the eventual result is dropped" (`sdks/SPEC.md:511`, `:516-517`). So no report follows.
- "A handler that really ran also reports" (`:714`) is therefore false. The detector for the trust assumption loses its main signal.

**Fix:** use the `heartbeat` kind only after the hand-back's withdrawal (`NOT_STARTED`). After `NOT_ACKNOWLEDGED`, count it under an alerted kind, or add a closed `cause` label and alert on it. Correct the sentence in item 3.

**Minor 6. "A host whose only work is flagged leases … waits for the ordinary give-up" contradicts the frame evidence.**

Confidence: confirmed in the documents.

**Where:** item 4 (`stream-completion.md:740`, "which has no other call"), `sdks/SPEC.md:385`, and the Fallback row (`:911`, "on a host whose only work is flagged leases, the same answers wait for the ordinary give-up").

**Why:** under a statement timeout on the acknowledgement alone, claims succeed. A host with free slots keeps receiving frames, and "a frame with leases arrived" is evidence. So it gives up early. The row passes or fails depending on the queue and the free slots.

**Fix:** state "with its slots full (`max_inflight` equal to the leases in flight) or no work queued" in the row, and "no other call and no frame" in the text.

**Minor 7. "An attempt that the host's transport never started on a stream does not count" has no observable mechanism across the six transports.**

Confidence: plausible.

**Where:** `sdks/SPEC.md:393`, item 5, `stream-completion.md:561-563`, and the case "a connection with more than 128 calls … does not fall back".

**Why:** the transports used by the six SDKs (grpc-go, grpc-java with Netty, grpc-js, grpcio, Grpc.Net.Client and tonic with h2) queue a call that waits for stream credit inside the transport. A deadline there reads as `DEADLINE_EXCEEDED`, the same as an unanswered call. So six SDKs cannot implement the rule the same way.

**Fix:** name the mechanism. For example, each SDK holds its own per-channel limit of concurrent calls below the server's `max_concurrent_streams`, and an attempt still waiting for that permit "was not started". Or `AcknowledgeTasks` goes on a channel of its own. Add a unit test per SDK.

## Nit

1. `sdks/SPEC.md:396` still says "Calls are ordered by their give-ups". `stream-completion.md` and Decision 006-065 now say "by the give-up of the last of their leases".
2. The record's wording:
   - Decision 2 (`ADR-0099…md:66`) still says "only the withdrawal command binds true" (architecture round-10 Minor 3).
   - Decision 5 (`:102`) says "That withdrawal sets `provisional_withdrawn_attempt`" without "for a lease flagged at its claim", which the contract and Decision 006-066 now say.
3. Two terms of the evidence rule are loose:
   - "No failed call to the database since" does not say whether a refusal the database answered (stale, not held) counts as failed, or whether "since" means since the evidence or since the lease's failure. Define "failed" as "ended without the database's answer", and "since" as "since that answer".
   - A frame counts by its arrival. A frame claimed before the failure but read late by the host passes. Count a frame read after the lease's most recent failure.
4. Timing of the early give-up:
   - With the evidence limited to calls sent after the latest failure, and heartbeats every 10 s or less, the early give-up comes at the first qualifying answer, up to one heartbeat interval after one receive bound.
   - The row "two minutes of answers with the server's envelope … give each lease up after one receive bound" and Decision 006-065 ("runs unflagged on its next lease within about 6 s") should say so.
5. The cap row of the failure table (`stream-completion.md:820`) still says that a marked lease at the cap exists "only when earlier withdrawals were written without the flag (by the floor build)". Add "or after the projection is rebuilt", which the marks section now says.
6. `worker_lease_marks_total`, the denominator of two alerts, should be incremented after the claim commits. Otherwise rolled-back claims inflate the ratio.
7. The 30 s failover case should state the replication mode. With asynchronous replication a claim can be lost at the promotion, and the acknowledgement then answers `NOT_HELD`, not `recovered`.

## Checks made against the code

- **The early give-up's timing:**
  - The acknowledgement's backoff is the §5 default (`sdks/SPEC.md:31`, `:269`): 200 ms doubling to 5 s, times a random factor in [0.5, 1].
  - The server's deadline bounds each attempt to about 4 s, and pool acquisition is bounded to 3 s (`pool.rs:32`).
  - A lease therefore spends a large share of a 15 s outage in backoff (Major 1).
- **The floor:** the codec rejects an unknown cause (`event_codec.rs:544`), and `lease-withdrawal-v1` is listed by the #260 build (`instances.rs:140`) (Major 2).
- **The scoped row identity:** a scoped row is keyed by node and instance path, and the per-row effects filter by `ExecutionInstanceId` (Minor 3).
- **The lease fields:** `TaskLease` carries `expires_at_epoch_ms` and no timeout, and the grant's expiry is not capped at the attempt deadline (Minor 4).
- **The latch of a beat with no epoch:** the census reads `last_seen`, which such a beat commits. An engine whose 25 s window had not lapsed has a `last_seen` within 30 s at any census, so it blocks an activation it does not list. I found no defect.
- **The heartbeat after a withdrawal:** the SDK drops a handler's result on a stale heartbeat (`sdks/SPEC.md:516-517`) (Minor 5).

## Evidence to ask for once built

- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`:
  - the 30 s failover with one lease held in backoff across the promotion and a frame arriving right after it. Expected: one more attempt, `ACKNOWLEDGED` (`recovered`), no hand-back, no attempt spent (Major 1);
  - the 18 s case asserting either branch of Gap 006-047 (Minor 2).
- `cargo test -p sagat-tests --test integration_durable_execution`: two instances of one collection node at equal generations, one withdrawn, the other's flag unchanged (Minor 3).
- The released-v0.2 job, if Major 2 takes option (b): a process of a release of #260 without the cause refused at registration while a `NOT_ACKNOWLEDGED` event exists.
- The SDK unit tests: the per-channel permit that defines "not started" (Minor 7), and a heartbeat after `NOT_ACKNOWLEDGED` counted under an alerted kind (Minor 5).

## Files consulted

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-10/ (correctness.md, security.md, architecture.md, egm-design.md)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/001-platform-foundation/contracts/worker.proto
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/event_codec.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/pool.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/event.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round10-to-11.diff
