# Security review, round 13: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Verdict: no Blocker, no Major, 3 Minor, 1 Nit.**

All three of my round-12 Minors are resolved, and I checked each against the base code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`. The round-13 changes add three smaller defects:
- **Minor 1:** the new reason `ACKNOWLEDGEMENT_STORE_UNREACHABLE` cannot be produced on the engine it was added for.
- **Minor 2:** the stale path's new journal read has no index behind it.
- **Minor 3:** the new second transport connection is not required to carry the main connection's TLS and credential configuration.

"Verified" means I read the code. "Design reading" means the defect is in the documents alone.

## Status of round 12

| Round-12 finding | Status | Reason (verified) |
|---|---|---|
| Minor 1: the beat read the epoch before taking the lock | **Resolved** | I checked the three orders. (a) The activation holds the row: the beat skips it and reports a change in progress. (b) The activation committed first: the locked read returns E+1. (c) The beat holds the share lock while the activation waits: the operator transaction runs in READ COMMITTED (`crates/infra-postgres/src/instances.rs:116-126`, `begin_safely`, no isolation override anywhere in the repository). After its wait, the activation's census is a later statement (`20270113000000_instance_registry.up.sql:323` locks, `:333-337` counts), so it sees the beat's committed `last_seen`. It counts the engine as live and refuses when the engine lacks the capability. No order shows the old epoch with no change in progress while a requirement is recorded. The test's stated expectation is wrong for order (c): Nit 1. |
| Minor 2: the second refusal lived only in the command | **Resolved** | The `BEFORE INSERT` trigger covers every path the runbook allows. The base guard already refuses every UPDATE of `platform_requirements` (`instance_registry.up.sql:105-108`), so a trigger on INSERT alone leaves no way around it. The hand path of runbook section 5 goes through `platform_requirement_lock` and the functions (`docs/runbooks/instance-registry.md:156-161`), so the trigger runs under the epoch lock. The residue is now stated in Gap 006-044. It is accurate: a process whose row is gone keeps serving through plain connection failures (`crates/application/src/instances.rs:27-30`), and its first successful beat registers again and closes on `CAPABILITY_MISSING` (`:659-667`). The third phase of the released-upgrade job covers that refusal. |
| Minor 3: the resend bypassed the backoff during sheds | **Resolved** | Attempts keep the backoff of §5 and any retry hint. The early attempt comes only after one receive bound, with no attempt shed, after up to 200 ms of jitter, and at most once per lease per receive bound (`sdks/SPEC.md:389-391`). Two SDK unit tests (Task 006-059) pin the attempt count during sheds and during fast failures. |
| Nit 1: the Gate row | Resolved | The row now has v2 and the case where only v1 is recorded. |
| Nit 2: what decides the heartbeat kind | Resolved as asked, but the chosen read is Minor 2 | |
| Nit 3 (optional) | Not taken | Accepted. |

Other reviewers' round-12 findings in my domain:
- **Architecture Minor 1, design Minor 6, correctness Nit 6:** resolved, as my Minor 2.
- **Architecture Minor 2 (the v2 removal guard narrowed):** resolved.
  - The codec reads only the attempt, the generation and the cause (`crates/infra-postgres/src/event_codec.rs:538-546`). So a build without the follow-up replays every Architecture decision 0098 cause, including a 1.8 `NOT_STARTED` withdrawal.
  - Removing v2 while `delivery-acknowledgement-v1` is recorded lets no engine in: an engine without `delivery-acknowledgement-v1` cannot register while it is recorded (`instance_registry.up.sql:199-205`).
- **Architecture Minor 5:** resolved by the third phase.
- **Architecture Minor 4:** resolved by naming the read, but the read is not indexed (Minor 2).
- **Correctness Major 1:** the new server reason does not reach the case it was added for (Minor 1).
- **Architecture Minor 3, design Minor 4:** a transport connection is now required, but its configuration is not (Minor 3).

---

## Minor

### Minor 1. `ACKNOWLEDGEMENT_STORE_UNREACHABLE` is never answered by an engine that cannot reach the database, because authentication reaches the database first and fails with the generic `UNAVAILABLE`, which the host counts toward the early give-up (verified)

**Where:**
- `stream-completion.md:605-608`, `:779`, `:789` and the failure row at `:899`.
- `sdks/SPEC.md:385-393`.
- The Fallback row and Task 006-058: "two engines … `ACKNOWLEDGEMENT_STORE_UNREACHABLE`, no give-up".
- Task 006-050 (`tasks.md:260-261`): "a pool held full for 5 s answered `UNAVAILABLE` (`ACKNOWLEDGEMENT_STORE_UNREACHABLE`)".

**What the code has:**
- Every worker call authenticates before its handler runs (`crates/api-grpc/src/service.rs:479-483`).
- `authenticate_bearer` makes two lookups, `find_tenant_by_slug` and `find_membership` (`crates/api-http/src/extract.rs:65`, `crates/application/src/identity.rs:252-256`).
- A store failure there becomes `ApiError::unavailable()`: `UNAVAILABLE` with the platform's envelope, code `Unavailable` and a retry hint of 5 s (`crates/api-http/src/error.rs:518-531`, `:1001`).
- §6.2 says "any other status, with the envelope or not, counts".

**Scenario:**
1. After a promotion, engine B's pool has not recovered and engine A's has.
2. The host's main connection lands on A, so its heartbeats are answered `OK`, which is evidence.
3. Its acknowledgement connection, a connection of its own since this round, lands on B.
4. The deciding attempt fails in `authenticate` with the generic `UNAVAILABLE`, so the lease is given up.
5. The hand-back reaches A. The mark belongs to the earlier server, so the hand-back is an ordinary failure, and the attempt is spent (a review for an `Activity` whose tries end before the promotion).

This is the case the new failure row says is covered. Task 006-050's test with the pool held full cannot pass as written either, because `authenticate` waits on the same pool before the handler starts. The direction is safe (nothing runs without `ACKNOWLEDGED`), so this is Minor.

**Fix:**
- On `AcknowledgeTasks` only, map a store failure during authentication (`AppError::Unavailable`, a pool acquisition timeout) to `UNAVAILABLE` with `ACKNOWLEDGEMENT_STORE_UNREACHABLE`.
- Do it only after the token has verified (`extract.rs:56`). An unverified token then still gets `UNAUTHENTICATED` without touching the database. The reason reveals no more than the generic `UNAVAILABLE` already does, so it creates no oracle.
- Write in `stream-completion.md` that the reason also covers the authentication stage.
- Make the two-engine test hold B's whole pool, not just the handler's connection.

### Minor 2. The stale path's "one indexed read" of the journal has no index for a root step, so a holder of a withdrawn token makes the shared primary scan the run's journal on every call (verified)

**Where:**
- `stream-completion.md:756-758`.
- `research.md:730-732`.
- Task 006-053: "the kind taken from the journal's latest withdrawal of that generation".

**What the code has:**
- `run_events` has its primary key `(tenant_id, run_id, seq)` (`20261009000100_execution.up.sql:101`) and `run_events_instance_idx (tenant_id, run_id, instance_id, seq) WHERE instance_id IS NOT NULL` (`20261027000000_scoped_instance_identity.up.sql:45-47`).
- Nothing indexes `event_type` or `node`.
- For a root step, "the latest `step.lease_withdrawn` of this node and generation" walks the run's primary-key range backwards from its end. It fetches heap tuples to filter on the event type until it reaches the withdrawal.
- `withdrawn_lease_token_hash` is "replaced by the next" withdrawal only (`stream-completion.md:491`), so a withdrawn token stays matchable for the life of the step.

**Scenario:**
1. A host of tenant T (hostile, or one breaking the protocol, Gap 006-042) keeps the withdrawn token of a root step.
2. The run then appends about 100,000 events, for example a collection under it.
3. The host sends `Heartbeat` with that token at its subject and tenant budget.
4. Each call costs a scan of about 100,000 tuples on the shared primary, and the answer is the ordinary stale answer.

The host gains load on other tenants' database, not their data: the read stays inside T. "Already rare" holds for hosts that keep the protocol, not for this one. No cost line counts the read.

**Fix (either one):**
- Write the cause in the same assignment as the hash: a boolean `withdrawn_on_hand_back`, listed in the static check's allowed forms, with "eight columns" corrected to nine. This was the architecture reviewer's first option.
- Or add a partial index `ON run_events (tenant_id, run_id, node, seq) WHERE event_type = 'step.lease_withdrawn'`, created concurrently and written only by withdrawals. Read it with `ORDER BY seq DESC LIMIT 1`, and add an `EXPLAIN` test that the plan uses it.

Either way, correct "one indexed read".

### Minor 3. The acknowledgement's own transport connection is not required to be built like the main one, so an SDK can send bearer and lease tokens with weaker TLS, or without the user's interceptors (design reading; construction verified in two SDKs)

**Where:** `sdks/SPEC.md:365-370`, `stream-completion.md:600-603`, Task 006-059 to Task 006-064.

**What the code has:**
- Each SDK builds its channel from options. In Go: TLS from a cloned `tlsConfig`, plaintext chosen by the URL scheme, and the user's `WithDialOptions` appended (`sdks/go/worker/host.go:377-406`).
- In .NET: `Tls.CreateHandler`, `Tls.WithServerName` and the user's interceptors (`sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/WorkerRpc.cs:22-43`).

**What is wrong:**
- The design says what to make ("a `GrpcChannel` on a handler of its own", "a `ClientConn` of its own", "an `Endpoint` connected apart") but not that it must be built by the same construction.
- A second construction written separately can drop:
  - the private certificate authority, the client certificate or the server-name override: the call fails closed and the host falls back;
  - the user's dialer or per-call credentials;
  - the interceptors: the user's redaction and audit hooks never see these calls;
  - or it can choose plaintext where the main channel uses TLS. Every acknowledgement's bearer token and lease tokens then travel in clear.
- No test would catch this. The token canary checks logs, not the wire.

**Fix:**
- One sentence in §6.2: the acknowledgement connection is made by the same function as the main connection, with the same target, transport credentials, TLS settings, server name, dial options and interceptors. It differs only in its subchannel pool, handler instance or connection.
- A unit test per SDK against a TLS server with a private certificate authority that requires a client certificate: the acknowledgement connection succeeds and presents the same certificate, and the user's interceptor sees the call.

---

## Nit

1. **Task 006-046's barrier expectation is wrong for one order** (`tasks.md:220-221`).
   - The text says "a lapsed engine keeps refusing in both".
   - In the order where the beat holds the share lock while the activation waits, the beat correctly returns the unchanged epoch with no change in progress, and the engine opens its gate: nothing is recorded yet.
   - What protects that order is that the activation, after its wait, counts the engine as live and is refused.
   - Fix: assert `INSTANCES_LACK_CAPABILITY` naming the engine (or, for a capability it has, that its next beat applies E+1). Write in Decision 006-059 (7) that the protection depends on the activation running in READ COMMITTED with its census in a statement after the lock, and pin that with a comment in the function.

## Suspicion, not verified

- **Beats holding off the epoch lock** (carried over). Each beat still holds `FOR SHARE` until it commits, so the effect is availability only.
  - Command, once built: the 1,000-instance scalability profile, timing `control-plane capability-activate` under `pgbench` load.

## Classes checked with no new finding

- **Tenant boundary:**
  - The stale path's read stays in the caller's tenant, and the answer is the same whichever kind is counted.
  - The longer timing of a matched withdrawn hash is visible only to the holder of that token.
  - Merged frames come from one session, and a session has one namespace (`RegisterWorkerRequest`, `worker.proto:78`), so a merged call never mixes tenants or namespaces. It also never trips the one-namespace refusal.
- **Authorization and admission:**
  - `ACKNOWLEDGEMENT_STORE_UNREACHABLE` is reached only after authentication, and with the Minor 1 fix only after the token verifies.
  - The reserve stays per connection in `CappedConnectInfo`, with no map keyed by the peer.
  - A second connection per host doubles that host's places, but any client can already open connections, and the tenant concurrency slot (`extract.rs:74-80`) is unchanged.
- **Least privilege:**
  - `sagat_app` still cannot lock the epoch row (it has `SELECT` only, `instance_registry.up.sql:404`).
  - The trigger is the owner's. Its `public.`-qualified names should follow the base guards.
- **Secrets and logs:** nothing new carries a payload. The new reason is a closed label (`store`).
- **Injection and parsing:** no dynamic SQL. Merged calls are capped at the server's 64 leases. Redials are bounded by attempts, which keep the backoff, and by the fallback after three counted calls.
- **Races:**
  - The narrowed v2 guard has the same lock order as the v1 guard: a withdrawal holds v2 `FOR SHARE`, so the removal waits for it and its guard then sees the event.
  - The trigger runs under the epoch lock on every path the runbook allows.

## Commands to run once built

- **Minor 1:** in `integration_durable_execution`, two engines, with B's pool held unreachable (a proxy blackholing B's database port). `AcknowledgeTasks` sent to B must answer `ACKNOWLEDGEMENT_STORE_UNREACHABLE`, not reason `UNAVAILABLE`, and no lease may be given up.
- **Minor 2:** `EXPLAIN (ANALYZE, BUFFERS)` of the stale-path read, as `sagat_app`, on a run with 100,000 events appended after a root step's withdrawal.
- **Minor 3:** the per-SDK test against a TLS server that requires a client certificate.

## Files

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261009000100_execution.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261027000000_scoped_instance_identity.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/event_codec.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/identity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-http/src/extract.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-http/src/error.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/service.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/go/worker/host.go
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/WorkerRpc.cs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/instance-registry.md
