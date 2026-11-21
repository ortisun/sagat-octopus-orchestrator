# Security review, round 8: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Verdict: no Blocker, no Major, 4 Minor, 6 Nit.**

All six of my round-7 Majors and Minors are addressed. Major 1 and Minors 1, 2, 4 and 5 are resolved; Minor 3 is partly resolved. The once-per-attempt bound holds up against a hostile worker or tenant: it is state on one step row, and only the lease's holder or the lost-lease pass can set it. The new defects are:

- The column that carries the bound can legally be cleared together with the lease (Minor 1).
- With the breaker gone, nothing limits the throughput lost during server-wide trouble (Minor 2).
- "No answer" still counts toward a host's decline, and there is no longer a time floor (Minor 3).
- The restore step does not stop the engines before the restore (Minor 4).

"Verified" means I read the cited code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`, or in the floor branch `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases` where I say so. "Design reading" means the defect is in the documents alone.

---

## Status of round 7

### My report

| Round-7 finding | Status | Reason |
|---|---|---|
| Major 1: an unauthenticated flood holds the breaker closed | **Resolved** | The breaker is withdrawn. Decision 006-066 needs no observation, so a flood has nothing to hold. A flood cannot set `provisional_withdrawn_attempt` either: only a withdrawal of a provisional lease sets it. The residue is the cost of having no relief at all (new Minor 2). |
| Minor 1: one tenant's lock waits open the breaker for everyone | **Resolved** | No engine-wide state is left. A tenant's own lock timeouts cost only its own leases, one give-up per attempt. |
| Minor 2: statement timeouts and an unavailable store carry no reason | **Resolved** (verified) | `to_status` always writes `x-sagat-error` (`crates/api-grpc/src/status.rs:28-37`). The admission shed, the unary timeout and the closed-gate refusal all go through it (`crates/api-grpc/src/limits.rs:673`, `:677`, `:691`). Residues: "no answer" still counts (Minor 3), and the "every status" wording (Nit 2). |
| Minor 3: `end-lease-marks` refuses in its own case | **Partly resolved** | The command, its census and its table are gone, and so is the defect. The restart replacing them works, but nothing stops the engines before the restore (Minor 4). |
| Minor 4: liveness rule not a precondition; boundary measured late | **Resolved** | (a) It is a hard precondition (`tasks.md`, precondition (a)). (b) It is measured from the start of the last successful beat, with a 25 s cut-off, and a boundary test holds the beat's answer back. (c) and (d) are named in Gap 006-044. |
| Minor 5: `lease-withdrawal-v1` read "for engines" | **Resolved** | Read for both kinds in the function, the marking condition, decision 10 and Tasks 006-044, 006-047 and 006-057. Request (8) is needed: I verified that the floor's `lease_withdrawal_required()` ignores `kinds` today (floor `20270208000000_lease_withdrawal.up.sql:71-76`). One stale sentence remains (Nit 1). |
| Nit 1: audit text | Resolved | The failure-table row now says "once per step and generation". |
| Nit 2: unreadable `fsync`, revoked start time | **Partly resolved** | The `fsync` read is gone. The revoked `pg_postmaster_start_time()` case is still unhandled (Nit 5). |
| Nit 3: `platform_operator_actions` details | Resolved (removed) | |

### Other round-7 findings in my domain

| Finding | Status |
|---|---|
| Design Major 1 and correctness Major 1 (the third trigger and the cap) | Answered by Decision 006-066. I checked that the column is per step row (scoped instances are `steps` rows: `crates/infra-postgres/src/execution.rs:6987-6993`), set only by a holder's hand-back after the token and binding matched, or by the pass. Residue: Minor 1. |
| Design Major 2; architecture Minors 7 and 8 (the breaker) | Resolved by withdrawal. |
| Design Major 3 and architecture Major 1 (liveness stops heartbeats) | Resolved. It is narrowed to capability calls. `ResolveSecrets`, `WriteBlob` and `ReadBlob` carry the lease token (`sdks/rust/proto/worker.proto:154`, `:169`, `:181`), so they are in reach. Heartbeats, reports, claims and hand-backs hand over nothing that the capability record protects. |
| Design Minors 4 to 6; architecture Minors 5 and 6 | Resolved. The command is gone, `fsync` is dropped, and "one writable server" is stated. Nit 4 is about the `fsync` reasoning. |
| Architecture Minor 1 and design Minor 1 (closed set of reasons) | Resolved by the error envelope. |
| Architecture Minor 2 (hand recording without the floor) | Resolved. Counted as `requirement_missing` and alerted. |
| Architecture Minor 4 (no structural check) | Resolved in the design by the `RequirementsHeld` witness type. |

---

## Minor

### Minor 1. `provisional_withdrawn_attempt` is defined as a lease mark, and the allowed forms let a statement clear it "with the lease", which brings the cycle back up to the cap (design reading)

**Where.**
- `stream-completion.md:421-423`: all marks are "written for a lease's generation" and count only while the step is `LEASED` or `RUNNING`. But the claim reads this column while the step is `READY`.
- `stream-completion.md:431`: the column sits in the marks table.
- `stream-completion.md:474-478`: an allowed form is "the engine's other statements keep them (`ELSE` the column) or clear them with the lease".
- Decision 006-066.

**What is wrong.** The withdrawals of Architecture decision 0098 that this build executes clear "the lease and its marks". These are a `NOT_SENT` withdrawal, and a `NOT_STARTED` withdrawal of an unmarked 1.8 lease. The static check accepts clearing this column there.

**Scenario.**
1. Attempt N has provisional lease L1. Its acknowledgement is lost, and L1 is withdrawn `NOT_ACKNOWLEDGED`, setting the column to N.
2. L2 is unflagged. The host's connection flaps before L2 reaches the transport, and L2 is withdrawn `NOT_SENT`, which clears the column.
3. L3 is flagged again. Its acknowledgement is lost and it is withdrawn, setting the column to N.
4. L4 is withdrawn `NOT_SENT`. The cap of four is reached.
5. L5 is flagged, because the claim does not read the cap. Its acknowledgement fails, and the pass ends it with `expire_attempt`: an unknown outcome, and a review for an `Activity`.

A flapping connection produces both halves of this together.

**Fix.**
- State that the column is attempt data, not lease data. Only the two withdrawals of a provisional lease write it (`= attempt`, from the row held). Every other statement keeps it, including the claim, every withdrawal and expiry, and every full-row write. It is outside the "counts" macro.
- Give it its own allowed form in the static check, with a negative test of a clearing form.
- Add a matrix case to Task 006-047 or Task 006-050: a provisional lease withdrawn, the next lease withdrawn `NOT_SENT`, and the third lease of the attempt still unflagged.

### Minor 2. With no relief left, a failure of the acknowledgement for every tenant holds a host slot for a whole give-up per attempt, for as long as it lasts (design reading, against verified timeouts)

**Where.**
- Decision 5 (`ADR-0099-…md:98`, "delayed by at most one give-up").
- Decision 006-066.
- The failure-table row "The acknowledgement failing at the server, for every tenant or for one".
- The "Bad" consequences, which do not name this cost.

**What the code has (verified).**
- Other calls have a 30 s budget (`crates/api-grpc/src/server.rs:82`, `crates/api-grpc/src/limits.rs:681-692`).
- The acknowledgement has a 3 s deadline, pool wait included, and statements of 1 s.

**Scenario.**
1. Tenant B's permitted load saturates engine E's request pool, so that a connection takes more than about 3 s to get. A database incident with primary-key statements above 1 s has the same effect.
2. Every acknowledgement on E ends `ACKNOWLEDGEMENT_DEADLINE`, while claims, heartbeats and reports succeed slowly.
3. Each host waits each flagged lease out to its give-up: 24 s at the default 30 s time-to-live (`apps/execution-engine/src/cli.rs:76`), and up to 120 s at long time-to-lives. Then it hands the lease back, and only the second lease runs.
4. Slots fill with waiting leases. For every tenant on E, the time per task becomes the degraded time of today plus one give-up. With three calls of about 3 s per task, that is about 10 s today against about 34 s. At a time-to-live of 126 s or more, it is about 130 s.

The guarantee "no attempt spent" holds. But the cost lasts as long as the cause, and one tenant can create the cause.

**Fix (smallest, with no engine-wide state and nothing a flood can hold).**
- A host gives a lease up at once, and hands it back, once every attempt of the call so far was answered with the envelope and a reason that means nothing was written. Those reasons are `ACKNOWLEDGEMENT_SHED`, `ACKNOWLEDGEMENT_DEADLINE` (the transaction rolled back before commit), a lock timeout and a statement timeout. Two such attempts in a row is enough.
- Keep waiting out "store unavailable", whose commit outcome is unknown.
- The withdrawal sets the column, and the next lease runs within seconds.
- Name the residual cost in "Bad".
- Assert in the bound test of Task 006-050 that, with every acknowledgement failing at the server, the step runs within two attempts plus one claim.

### Minor 3. "No answer" within the host's 6 s attempt still counts toward a decline, and with the 60 s floor gone, a slow engine declines every 1.8 host on it within one give-up (verified transport layering)

**Where.**
- `sdks/SPEC.md:383-386`.
- `stream-completion.md:659-666`.
- Decision 006-065 ("there is no time floor").

**What the code has (verified).**
- The server bounds only its handler: 3 s, plus one statement of 1 s, plus a 100 ms rollback, about 4.1 s from arrival.
- Time before the handler is outside that bound. This covers tonic's `concurrency_limit_per_connection`, which queues rather than sheds (`crates/api-grpc/src/server.rs:157`), body decoding, and runtime scheduling under CPU load.
- The stream keeps delivering frames, and a frame counts as "another call answered".

**Scenario.**
1. One tenant's load drives E's CPU so that acknowledgement answers arrive after 6 s, while frames and heartbeats still arrive.
2. Every attempt of a host's three in-flight frames ends with no answer, and each call is given up.
3. That makes three counted calls in about 24 s.
4. Every 1.8 host on E declines for 10 minutes, doubling up to an hour. This is the effect of round-6 Major 1, now triggered by latency instead of a status.

It fails toward today's meaning, hence Minor.

**Fix.**
- Count another call as evidence only if it came back within half the receive bound (3 s) of its own start. A path that swallows one call passes this test; a uniformly slow server does not.
- Start the server's deadline at admission (`Admit::call`), not at the handler.

### Minor 4. The restore step restarts PostgreSQL after the restore but never stops the engines before it (verified runbook)

**Where.**
- `docs/runbooks/disaster-recovery.md:92` (section 7, step 5: "restart PostgreSQL before the engines start").
- `:82` (the drill paragraph).
- `:47`: section 4 stops the writers, but only for point-in-time recovery.
- Gap 006-048.

**Scenario.**
1. Engines are running during a logical restore into the same server, or a virtual machine revert.
2. The restored rows carry marks of the unchanged start time, so they count.
3. Before the operator reaches step 5, the pass withdraws a provisional lease whose acknowledgement the restore lost.
4. The step runs a second time with the same attempt number and idempotency key. The journal says "nothing ran", and no review is opened.

Following the runbook exactly leaves this window open, so it is not only "the step skipped".

**Fix.**
- In step 5, and in a short paragraph in section 3 or 4: "before a restore that does not start the server, stop every engine (scale to zero); after it, restart PostgreSQL, check that `pg_postmaster_start_time()` changed, then start the engines."
- Add the same clause to the drill paragraph.
- Name "engines left running" in Gap 006-048.

---

## Nit

1. **A stale sentence.** `stream-completion.md:389` still says the entry "refuses while `lease-withdrawal-v1` is not recorded for engines". Decision 10, `:400` and Task 006-057 say "for both kinds".
2. **"The platform puts its error envelope on every status it makes" is not true of today's code** (`stream-completion.md:659`, `research.md:715`). `crates/api-grpc/src/service.rs:285` (`Status::deadline_exceeded`) and `:615` (`Status::internal`) carry no envelope. This is harmless for `AcknowledgeTasks`. Fix: either narrow the sentence to `AcknowledgeTasks` and the admission layer, or make Task 006-049's test a check that scans every `Status::` constructor in `crates/api-grpc`.
3. **An overclaim at the cap.** "No failure of the acknowledgement's path can spend an attempt" (`ADR-0099-…md:227`) is false at the cap. An attempt that reached four withdrawals through `NOT_SENT` or unmarked `NOT_STARTED` is still flagged (`stream-completion.md:470-471`; the failure-table row "the claim keeps marking"). Its failed acknowledgement then ends in `expire_attempt` or a spent attempt. Add "below the cap".
4. **The `fsync` reasoning.** "Only the loss of the operating system loses" a commit with `fsync` off (`research.md:684`, `stream-completion.md:457`). But a device write-back error is never reported to a PostgreSQL that does not call `fsync`, and the server keeps running with the same start time. Say that `fsync = off` is outside the guarantee (Gap 006-048), rather than covered by it.
5. **A revoked `pg_postmaster_start_time()`.** It would fail every 1.8 claim, not only the marking, and Task 006-044 checks only the pinned image. Fix: a start-up `has_function_privilege` probe. On false, the engine writes no reservation and no mark, with its own skip reason, and alerts.
6. **Request (8) changes the body of a floor migration.** Say whether it is done before `20270208000000_lease_withdrawal.up.sql` is applied anywhere (as request (6) says) or by a new `CREATE OR REPLACE` migration.

---

## Classes checked with no new finding

- **Abuse of the once-per-attempt bound across tenants.**
  - The column is on the step row.
  - It is set only after the token and binding match, or by the pass.
  - No tenant can set it on another tenant's step except by causing a failure for every tenant, which is Minor 2.
- **A worker unflagging its own leases.** A hostile worker can unflag only its own next lease, and gains nothing.
  - The unflagged lease's hand-back is already a withdrawal under Architecture decision 0098.
  - Running a lease before handing it back is Gap 006-042, unchanged.
- **Forging the envelope.**
  - Adding it makes only the host behind that path wait, and the bound limits that cost.
  - Stripping it makes only that host decline, which is today's meaning.
  - A path that can do either can already drop the call.
- **The liveness boundary.**
  - The cut-off at the start of the beat plus 25 s comes before the census drops the engine (`last_seen` plus 30 s). Both of those are on the database clock, so a clock offset cancels.
  - A silent floor engine's claims write no reservation.
- **Reading the requirements for both kinds.**
  - There is one row per capability with `kinds text[]` (`20270113000000_instance_registry.up.sql:49-57`), and activation refuses `KINDS_DIFFER` (`:310`, `:326`). So "both kinds" is one containment test, and the hand SQL cannot widen the kinds without removing the requirement first, which the replay guard blocks.
  - `FOR SHARE` runs with the definer's `UPDATE` right; `sagat_app` needs only `EXECUTE`.
- **Least privilege.**
  - The one new integer column is covered by the existing table grants.
  - `FOR NO KEY UPDATE OF steps` needs `UPDATE` on `steps`, which `sagat_app` has.
  - The only function is the requirements read, `SECURITY DEFINER` with a pinned `search_path`.
- **Tenant boundary, audit and logs.**
  - The new labels are a closed set (`reason` has five values).
  - `attempt_time_left_ms` is answered only after the token and binding match.
  - The cross-tenant existence check runs the same statement whether or not the run exists in another tenant, so its timing is no oracle.
  - The audit text is aligned.
- **Injection and parsing.** There is no dynamic SQL, and the only new input is an optional `int64`.

## Commands to run once the build exists

- The Task 006-047 or 006-050 matrix with a provisional withdrawal followed by a `NOT_SENT` withdrawal in the same attempt (Minor 1).
- The bound test of Task 006-050 with the pool held full, measuring the time to the second lease (Minor 2).
- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`, with acknowledgement answers delayed past 6 s while frames flow; it must not fall back (Minor 3).
- `infra/dr/drill.sh` with a logical restore run while the engines are up (Minor 4).
- `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_platform_foundation` for the privilege audit of `delivery_acknowledgement_required()`.

## Files

- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-withdrawn-a-fleet-requirement-may-require-another.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/status.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/service.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/rust/proto/worker.proto`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/apps/execution-engine/src/cli.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql`
