**Security review, round 7: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)**

**Verdict: no Blocker, 1 Major, 5 Minor, 3 Nit.**

Round-6 Major 1 is partly resolved. The host no longer counts shed or deadline answers, so a tenant's burst no longer brings a decline of 10 to 60 minutes. But the breaker that replaced that path has two problems. Anyone, unauthenticated, can hold it closed (new Major 1), and one tenant can open it for everyone (new Minor 1). The server's other transient answers still carry no reason (new Minor 2). Minors 1 and 4 and Nits 1 to 3 are resolved. Minors 2 and 3 are partly resolved, and Nit 4 is still open.

"Verified" means I read the cited code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`, or in `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases` where I say so. "Design reading" means the defect is in the documents alone.

---

## Status of round 6

### My report

| Round-6 finding | Status | Reason |
|---|---|---|
| Major 1: another tenant's load switches the protection off fleet-wide | **Partly resolved** | Shed and deadline answers carry a reason and never count for the host, so the 10-to-60-minute tail is gone. The breaker's effect ends with the cause; with a 60 s window plus 60 s to close, that is up to about 120 s after the load, not 60 s. **Residues:** the breaker can be held closed by an unauthenticated flood (Major 1) or opened by one tenant's own lock waits (Minor 1). Statement timeouts and an unavailable store still have no reason, so a slow database still declines every host (Minor 2). |
| Minor 1: capability record lost on the same timeline | Resolved | Decision 8, Decision 006-062 (a), and the `synchronous_commit = off` case of Task 006-052. |
| Minor 2: a restore in place, engines left running | **Partly resolved** | Every restore that starts another server is now covered by the start time without an operator; that was the main scenario. **Residue:** for a logical restore into a running server, nothing stops the engines, and the new refusal of `end-lease-marks` fails in exactly that case (Minor 3). |
| Minor 3: a floor engine silent at the census | **Partly resolved** | The rule is stated for this build and asked of the floor build as request (7). But (7) is not among the preconditions, and the floor branch still keeps serving. There is also a boundary gap and a release-0.2 residue (Minor 4). |
| Minor 4: no record for `end-lease-marks` | Resolved | `platform_operator_actions` is append-only, unreadable by the runtime role and written in the same transaction; the operator text is bounded; the update is restricted to `LEASED`/`RUNNING`; the step is unconditional. The refusal check is Minor 3; column details are Nit 3. |
| Nit 1: `SECURITY DEFINER`, lock order | Resolved | `stream-completion.md:396-400`, Task 006-044. With Architecture decision 0100 withdrawn the removal order is only a runbook instruction; a reverse-order hand-SQL removal can deadlock with a withdrawal, which PostgreSQL detects, and Task 006-051 covers it. |
| Nit 2: the timeline function | Resolved (removed) | The same class of problem now applies to the `fsync` and start-time reads (Nit 2). |
| Nit 3: mixed fleet names a control plane | Resolved | Task 006-066 now uses a floor engine. |
| Nit 4: audit text | **Not resolved** | Nit 1. |

### Other round-6 findings in my domain

| Finding | Status |
|---|---|
| Correctness Major 1 (a host with full slots never falls back) | Answered by counting a hand-back's `OK` answer. This has no reach beyond the host's own session. |
| Architecture Major 1, design Major 1 (start time) | Adopted. Residue: Minor 3. |
| Architecture Minor 1 (`end-lease-marks`: table, live check, bound) | Table and bound resolved. The live check uses a different condition from the one asked for ("seen within 30 s"), and that condition is broken (Minor 3). |
| Architecture Minor 5 (capability record) | Resolved. |
| Architecture Minor 6, correctness Minor 7 (connection discarded at the deadline; a timeout of 0) | Resolved: rollback and return, discard only after 100 ms, never below 1 ms, not begun under 50 ms. No backend is left unbounded. |
| Design Minors 1 and 2 ("answered" undefined) | Defined on the host side. Undefined on the server side (Major 1). |
| Architecture Minors 2 and 3, design Minor 5 (Architecture decision 0100) | Withdrawn and replaced by "every withdrawal reads both". Residue: which kinds it reads (Minor 5). |
| Correctness Minor 1 (report fence relaxed for an acknowledged hand-back) | Checked: `verify` and `ensure_bound` still run first (floor branch `crates/application/src/worker.rs:2913-2915`). Only the cancel, stop and expiry conditions are relaxed, and the outcome is a failure that spends the attempt, so a holder of a stale token gains nothing. |

---

## Major

### Major 1. An unauthenticated flood can hold the breaker closed during a slowdown. Flagged leases then cycle to the cap and spend attempts across every tenant (design reading, against verified admission and authentication code)

**Where.**
- `stream-completion.md:404-410`.
- Architecture decision 0099, decision 10 (`ADR-0099-…md:141-145`).
- Decision 006-065 (`research.md:699`).
- Task 006-047 (the breaker's unit test).

**What the code has (verified).**
- Admission runs before authentication (`crates/api-grpc/src/limits.rs:4-6`, `668-679`).
- A garbage bearer fails at `verifier().verify(token)`, before any database lookup and before any tenant budget (`crates/api-http/src/extract.rs:52-65`).
- One connection carries 128 streams (`crates/api-grpc/src/server.rs:85`).
- Sheds happen before the handler. So the breaker must count at the transport layer to see them, and there it sees every answered call.

**What is wrong.** The ratio is over "the `AcknowledgeTasks` calls it answered". Nothing excludes calls answered `UNAUTHENTICATED`, `INVALID_ARGUMENT`, or `NOT_HELD` with no matched row.

**Scenario.**
1. Engine E's pool is saturated or the database is slow. This can be an incident, or tenant B's own heavy but permitted traffic on the shared database. About 60% of real acknowledgements end with `ACKNOWLEDGEMENT_DEADLINE`, while heartbeats, which have the full unary timeout, are answered.
2. A client with no credentials sends `AcknowledgeTasks` with garbage tokens at twice the real volume. At a failure share f, it needs (5f − 1) times the real volume: 2 at 60%, 4 at 100%. Each call is admitted while the band is not full, refused in microseconds, and never touches the pool.
3. The ratio stays under 20%, the breaker never opens, and E keeps flagging.
4. Hosts wait the reasoned answers out, because by design they never decline on them. Each flagged lease is given up at about 24 s, handed back, withdrawn and claimed again with the flag. After four withdrawals the hand-back is an ordinary failure and the attempt is spent. Retry policies run out. A lost hand-back becomes `expire_attempt`, which for an `Activity` opens a review.

**What the attacker obtains.**
- No data.
- A slowdown that today only slows tasks becomes failed steps and reviews for every tenant on E.
- It costs a few thousand cheap calls a minute.
- It disables the mechanism that answers round-6 Major 1 and round-5 correctness Major 2, and the host has no other way to stop.

**Fix (a definition and a test).**
- Numerator: admission sheds of the class, from any caller (they happen only while the band is full, which sheds everyone), plus the engine-wide transient endings (Minor 2) of calls that passed `worker:execute` and matched at least one row.
- Denominator: the same population plus such calls answered `OK`.
- Exclude `UNAUTHENTICATED`, `PERMISSION_DENIED`, `INVALID_ARGUMENT`, tenant-budget `RESOURCE_EXHAUSTED` and calls that matched no row.
- Unit test in Task 006-047: with the pool held, a flood of garbage-token calls ten times the real volume, and a flood of `NOT_HELD` calls, each still lets the breaker open.

---

## Minor

### Minor 1. One tenant's waits on its own locks open the breaker for every tenant on the engine, and the "breaker open" alert can be avoided by pausing (design reading, against verified lock keys)

**Where.** `stream-completion.md:404-408` (the numerator includes "a lock or statement timeout") and `773-777` (the alert fires only on "open for more than 10 minutes").

**What the code has (verified).**
- The locks the acknowledgement takes are keyed per tenant, namespace and subject: `binding:{tenant}:{namespace}:{subject}` shared (`crates/infra-postgres/src/bindings.rs:21-22`, `crates/infra-postgres/src/secrets.rs:225-226`).
- Replacing or revoking that binding takes the same key exclusively (`bindings.rs:86-92`), through the tenant's own administrator.

**Scenario.**
1. Tenant B's administrator repeatedly replaces its own worker subject's binding while B's run transactions hold the key shared.
2. B's acknowledgements queue behind the exclusive waiter and end at the 1 s lock timeout.
3. B runs enough flagged leases that its failing calls exceed 20% of E's calls. On a quiet engine, 20 calls a minute is enough.
4. E stops flagging for all tenants while B keeps this up, plus up to about 120 s.
5. Nine minutes open and two closed, repeated, never fires `SagatDeliveryAcknowledgementInactive`. `SagatAcknowledgementsFailing` (non-`OK` above 5%) does fire, but it does not say the protection is off.

**What the attacker obtains.** Other tenants' lost frames lapse into reviews while it lasts. It fails toward today's meaning.

**Fix.**
- Sort each transient ending into engine-wide or tenant-local:
  - engine-wide: admission shed, pool wait, statement timeout, store unavailable;
  - tenant-local: lock timeouts and deadlocks on the binding, secrets and membership locks; the acknowledgement never waits for a step row.
- Only engine-wide endings carry a server reason and count in the engine's breaker.
- Tenant-local endings stay unreasoned, so the tenant's own hosts decline on their own path. That is self-limiting.
- Alert on the share of time the breaker is open, for example `avg_over_time(worker_acknowledgement_breaker_open[1h]) > 0.25`, or on `worker_lease_marks_skipped_total{reason="acknowledgement_failing"}` as a share of claims.

### Minor 2. Statement timeouts and an unavailable store carry no reason, so a slow database still makes every host on an engine decline (verified error mapping)

**Where.**
- `sdks/SPEC.md:373-378`, which lists only `ACKNOWLEDGEMENT_SHED` and `ACKNOWLEDGEMENT_DEADLINE`.
- `stream-completion.md:537-539` (item 7) and `652` ("the store unavailable" counted as a server reason, with no token defined).
- Decision 006-065.

**What the code has (verified).**
- An unavailable store maps to `ApiError::unavailable_after` with no reason (`crates/api-http/src/error.rs:516-533`).
- A statement timeout reaches the adapter as `StoreError::Unavailable` with `sqlstate:57014` only in its internal detail (`crates/application/src/error.rs:135`).

**Scenario.**
1. Primary-key statements exceed 1 s. Each acknowledgement statement's timeout is the smaller of 1 s and the time left, so this fires before the 3 s deadline.
2. Heartbeats, which have the full timeout, are answered.
3. Every 1.8 host on E sees `UNAVAILABLE` with no server reason three times over 60 s, while other calls are answered.
4. Each host declines for 10 minutes, doubling up to an hour.

That is round-6 Major 1's effect, triggered by an incident that a noisy tenant can create on the shared database.

**Fix.**
- Give the engine-wide transient endings of the call (Minor 1's list) one server reason, a new token or `ACKNOWLEDGEMENT_DEADLINE` reused.
- Name it in `sdks/SPEC.md` §6.2 and item 5.
- Conformance case: "two minutes of statement timeouts for this call alone": no fallback, and the breaker opens.

### Minor 3. `end-lease-marks` refuses in exactly the case it exists for, and its `--expect-live 0` turns the check off (verified against the registry)

**Where.**
- `stream-completion.md:452-460`.
- Task 006-054.
- `docs/runbooks/disaster-recovery.md:92`.

**What the code has (verified).**
- Every beat rewrites `serving_instances.last_seen` (`20270113000000_instance_registry.up.sql:34`, `260`), and rows stay for up to a day (`:265`).
- The census everywhere else is `last_seen > clock_timestamp() - interval '30 seconds'` (`:336`; floor `20270208000000_lease_withdrawal.up.sql:44`).
- In `capability-activate`, `--expect-live N` must equal the census count (`apps/control-plane/src/lease_withdrawal.rs:200`).

**What is wrong.**
- The command is needed only for a logical restore into a server that keeps running. That server's start time predates the dump and the restore, so every registry row beaten since is "newer than the server's start time", whether restored from the dump or left by engines stopped minutes ago. The command always refuses.
- `--expect-live 0` then becomes the routine path, and with it the check does nothing. In the sibling command the same flag asserts that zero engines are live.
- Section 7 still has no step that stops the engines before a logical restore.

**Scenario.**
1. The operator restores a dump into the running server with the engines up.
2. The pass withdraws a provisional lease whose acknowledgement the restore lost. The step runs a second time with the same attempt number and idempotency key, and no review is opened.
3. Step 5 refuses because of the stale rows. The operator adds `--expect-live 0`.
4. The record's "live engines found" lists stale rows. Nothing shows that engines ran.

**Fix.**
- Refuse, inside the function's transaction, while the census (seen within 30 s) lists an engine.
- Give `--expect-live N` the sibling's meaning: it must equal the census, with no override.
- Record the instance ids the census found, up to 16.
- Add to section 7, and to a short logical-restore paragraph in section 3 or 4: "before a logical restore into a running server, scale every engine to zero".

### Minor 4. The liveness rule is not a precondition for the floor build, and its cut-off is measured later than the census's (verified)

**Where.**
- `tasks.md:139-144`: precondition (a) names requests (1) and (3) to (6), not (7).
- Decision 006-059 (7) (`research.md:632`).
- Decision 006-064 (`research.md:686`).
- `stream-completion.md:411-414`.

**What the code has (verified).**
- The floor branch still keeps serving after a plain connection failure (floor `apps/control-plane/src/registry.rs:140-141`; `crates/application/src/instances.rs:27-30`).
- The gate's silence is measured from `heard()`, called after the beat returns (`instances.rs:647-648`), with a strict `>` (`:425-426`). The census instead counts `last_seen`, stamped at the update (migration `:260`).
- A process that registers into a full table serves without being counted (`instances.rs:23-25`; migration `:221-222`).
- Release 0.2 binaries do not register at all (`research.md:235-237`).

**What is wrong.**
- (a) Without (7) in the floor build, Decision 006-064's claim is false for the pair of releases that ship together.
- (b) A beat whose commit waits on a synchronous standby (bounded only by the registry pool's statement timeout) moves the engine's cut-off past the census's. Every 1.8 claim writes the reservation whether or not its engine has learned of the activation. So a capability served in that gap, to a lease claimed just after the activation, can still be withdrawn on hand-back.
- (c) For a 0.2 engine, the guarantee rests on the runbook's rule, which Decision 006-064 does not say.
- (d) An uncounted process (Gap G2000, which needs the runtime role) serves for one beat after an activation.

**Fix.**
- Make (7) a hard precondition in (a).
- Measure the silence from when the last successful beat was sent, and stop at the window less a margin (25 s), compared with `>=`.
- Name (c) and (d) in Gap 006-044.

### Minor 5. The design reads `lease-withdrawal-v1` "recorded for engines", where the floor requires both kinds, so hand SQL can produce an event with no control-plane floor (verified against the floor build)

**Where.**
- `stream-completion.md:387-389`, `396-400`.
- Decision 006-047 (`research.md:505-512`).
- Task 006-057.
- Decision 10.

**What the code has (verified).**
- The floor writes the event only when `lease-withdrawal-v1` is recorded for both kinds (Decision 006-004, `research.md:228-232`).
- Its activation entry records `InstanceKind::ALL` (floor `apps/control-plane/src/capability_activation.rs:41-44`).
- `platform_requirement_activate` takes any kinds array (migration `:312`). That is the hand SQL of `instance-registry.md` section 5.

**What is wrong.**
- If an operator records `lease-withdrawal-v1` by hand for engines only, this build marks and writes `step.lease_withdrawn`, while a registering control plane that cannot replay the event is admitted.
- So the sentence "no order … by the hand SQL … can write an event without its replay floor" is false.
- In the other direction, recording `delivery-acknowledgement-v1` by hand for both kinds closes every control plane's gate, because control planes do not list it.

**Fix.**
- Read `lease-withdrawal-v1` as recorded for both kinds, in `delivery_acknowledgement_required()`, in the marking condition and in the activation entry. Keep `delivery-acknowledgement-v1` for engines only.
- Add both cases to the "Gate" test row.
- Name the kinds of each requirement in `instance-registry.md` section 5.

---

## Nit

1. **Round-6 Nit 4 is still open.** `stream-completion.md:727` says "audited once per step and withdrawn token", while Decision 006-053 and Task 006-053 say "once per step and generation".
2. **An unreadable value cannot fail safe as written.** "An unreadable value counts as off" (`stream-completion.md:449`) is not what `current_setting('fsync')` does: it raises an error rather than returning null. And because every 1.8 claim stamps `pg_postmaster_start_time()`, a `REVOKE EXECUTE` on it would fail every 1.8 claim, not only marking. Neither happens on stock PostgreSQL. Fix: a start-up probe (`has_function_privilege`, `pg_settings`) after which the engine writes no reservation and no mark, with a skip reason.
3. **`platform_operator_actions` details.**
   - A closed check on `action`.
   - `database_user` bounded to 128, as in the history table.
   - The engine list bounded to 16 ids.
   - `--dry-run` writes no row.
   - The log line shows the operator through the same sanitiser `capability-activate` uses.
   - Say whether the `down` keeps or refuses a non-empty table. Keeping it preserves the evidence.

---

## Classes checked with no new finding

- **A hostile worker forging the server's reasons.** A reason only changes how the host reads an answer. A hostile worker controls only its own host and its own session row, keyed by tenant, subject and worker name (`crates/infra-postgres/src/workers.rs:334-343`). It could register as declining anyway.
  - A network path that adds a reason only stops its own hosts from declining.
  - Suspicion, topology only: a balancer that pins a host's acknowledgements to one engine and its stream to another leaves the flagging engine's breaker blind. The design names it (`stream-completion.md:409-410`).
- **The hand-back answer as evidence.** It needs the lease's token and binding, and it affects only that host's own decline.
- **The liveness rule's availability and whether an attacker can trigger it.**
  - The beat has its own one-connection pool (`apps/control-plane/src/registry.rs:53-61`, used by `apps/execution-engine/src/lib.rs:187-196`), so worker load cannot starve it.
  - The rule acts only after 30 s of connection failure on that pool, when readiness already fails (`instances.rs:37-38`), and it reopens one beat later.
  - I found no trigger reachable through the worker API.
- **`platform_operator_actions` privileges.** As the history table: revoked from `PUBLIC` and the runtime role, `ENABLE ALWAYS` append-only and no-truncate guards (migration `:157-168`, `:385-400`). It holds no tenant data and no token, and the operator name is checked as a `Subject` (floor `apps/control-plane/src/capability_activation.rs:199`).
- **`pg_postmaster_start_time()` as the runtime role.** It is a catalog function executable by `PUBLIC`, compared by equality, and never reaches a tenant: the answer carries only `lease_time_left_ms`, and the gauges have no tenant label. A crash the postmaster recovers from is covered by the durable commit.
- **Rollback.** The order is switch off, confirm, drain provisional leases, remove, roll back, switch back. No window lets a floor engine serve a lease that this build then withdraws.
  - Leases claimed before a re-activation do not count.
  - `required_since` is stamped under the epoch lock that registration also takes (migration `:197`, `:323`, `:344`).
- **Tenant boundary.** Shed and deadline answers are the same for every caller, and the breaker's metrics have no tenant label. The flag a tenant sees reveals only the engine's overall state (accepted).
- **Injection and parsing.** No dynamic SQL; the requirement read is a fixed function; operator text is bound and checked.

## Commands to run once the build exists

- The breaker's unit test of Task 006-047, with a garbage-token flood and a `NOT_HELD` flood while the pool is held (Major 1), and a tenant-local lock-timeout case (Minor 1).
- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`, with "two minutes of statement timeouts for this call alone" (Minor 2).
- `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_platform_foundation`: the privilege audit of `platform_operator_actions`.
- The command test of Task 006-054 after a logical restore with stale registry rows (Minor 3).
- Task 006-066's silent-floor-engine case, once (7) is in the floor build (Minor 4).
- The Gate row with `lease-withdrawal-v1` recorded by hand for engines only (Minor 5).

## Files

- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-http/src/extract.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-http/src/error.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/error.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/instances.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/bindings.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/secrets.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/apps/control-plane/src/registry.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/apps/control-plane/src/lease_withdrawal.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/control-plane/src/registry.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/control-plane/src/capability_activation.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql`
