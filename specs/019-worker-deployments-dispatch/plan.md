# Implementation Plan: Worker deployments and dispatch

**Branch**: `feature/019-worker-deployments-dispatch` | **Date**: 2026-10-26 | **Spec**: [spec.md](spec.md)

Status: Approved for implementation (Owner decision 2026-11-11-1) — not started. Corrected 2026-11-24 against
`develop` at `2c94064f` and the work in flight ([research](research.md#corrections-and-decisions-of-2026-11-24)).

**Input**: [spec.md](spec.md), [research.md](research.md), [data-model.md](data-model.md),
[contracts/](contracts/), accepted (2026-11-24, delegate (OD-2026-11-24-4))
[ADR-0051](../../docs/adr/ADR-0051-dispatch-modes-are-variations-of-the-attempt-lease.md)
and [ADR-0052](../../docs/adr/ADR-0052-warm-script-containers-are-an-opt-in-exception-to-one-container-per-run.md).

## Summary

Every new dispatch capability is a variation of the attempt lease that the engine already grants in one canonical
claim transaction (`docs/architecture/worker-runtime-protocol.md`, "Final V1 decision"). Deployment routing adds an
eligibility predicate and a journaled pin to that claim; priority and fairness change the order of candidates it
considers; checkpoints ride on heartbeats; detach changes the expiry and holder of a lease and hands out a token
derived from it; push invocation is a claim made by an engine-side invoker instead of a gRPC session; warm script
containers change only where the runner executes an attempt it leased normally. No routing table, token store,
broker or process becomes an authority over who may report an attempt: the run row, the fence and the lease token
remain that authority. Each capability is switched on for the whole fleet by a requirement of the instance registry
(ADR-0082, D019-001), so no process of a rolling upgrade serves it without honouring it.

## Technical Context

**Language/Version**: Rust (pinned by `rust-toolchain.toml`); TypeScript/React Studio; six SDKs (TypeScript, Go,
Python, C#, Java, Rust; corrected 2026-11-24, the draft said five).

**Primary Dependencies**: existing Tokio, Axum, tonic, SQLx, `reqwest` (inside `crates/infra-egress`), Ed25519
signing through `ring` as `crates/application/src/script_signing.rs` already signs script artifacts. Corrected
2026-11-24: no new signing library is needed — `aws-sigv4` is already a dependency of `crates/infra-connectors`
(ADR-0025 brokers) and `jsonwebtoken` of the workspace, used there for Google service-account assertions; none
enters `crates/domain`.

**Storage**: PostgreSQL (authority). New tables and additive columns in [data-model.md](data-model.md).

**Testing**: `cargo test` targets `contract_rest_worker_deployments_dispatch`,
`contract_grpc_worker_deployments_dispatch`, `integration_worker_deployments_dispatch`,
`fault_worker_deployments_dispatch`, `e2e_worker_deployments_dispatch`, `perf_worker_deployments_dispatch` (new,
`tests/Cargo.toml`; corrected 2026-11-24 to the repository's split of contract targets, D019-013), the shared
`sdk_conformance` target, the `contract_rest_security_multitenancy` battery, the claim budget of
`integration_claim_transition_efficiency` (design 038), `integration_inline_code_sandbox`, Studio Vitest/Playwright,
per-SDK suites.

**Target Platform**: Linux containers (engine, control plane, sandbox runner); Kubernetes chart `infra/helm`.

**Project Type**: modular monolith (ADR-0001) with separate runner processes.

**Performance Goals**: none promised; benchmark criteria in NFR-019-005, run by
`tests/performance/worker-deployments-dispatch/` (new) on a declared environment.

**Constraints**: one transaction per transition; database clock; runtime role without row locks; finite ceilings;
no tenant/run/build/endpoint/key in metric labels.

**Scale/Scope**: 42 FR, 7 NFR, 14 SC, 8 user stories; three protocol minors; four permissions; six fleet
requirements; twelve new tables.

## Architecture and crate placement

| Concern | Domain (`crates/domain`) | Application (`crates/application`) | Adapters | Apps |
|---|---|---|---|---|
| Deployments, versions, routing, buckets, pins | new `src/worker_dispatch.rs` (D019-011): `DeploymentName`, `BuildId`, `VersionState`, `Routing`, `bucket(run, deployment)`, `route()`, behaviour precedence | new `src/deployments.rs` (management, repin, retire); claim hooks in `src/worker.rs` | `crates/infra-postgres/src/workers.rs` (claim predicate), new `src/deployments.rs`, migration; `crates/api-http/src/routes/` new `deployments.rs`; `crates/api-grpc/src/service.rs` (registration fields) | `apps/cli/src/` new `deployments.rs` |
| Priority, fairness | `src/worker_dispatch.rs`: `DispatchOptions`, effective attributes at activation; weighted max-min moved from `crates/scheduler/src/fairness.rs` | claim window selection in `src/worker.rs` | `CLAIMABLE` ordering and index in `workers.rs` | — |
| Checkpoints | `src/execution/command.rs` (`heartbeat` accepts details), events `step.*` digest fields | `src/worker.rs` heartbeat/fail; disclosure carrier in `src/payload_disclosure.rs` | new `crates/infra-postgres/src/checkpoints.rs`; `crates/infra-ir/src/` (`heartbeatSchema`); `crates/api-http/src/routes/runner.rs` (script checkpoints from the runner) | `apps/sandbox-runner/src/host.rs` (supervisor call) |
| Detach and task tokens | `src/execution/command.rs` (`detach`, detached expiry), `TaskToken` codec; `DETACHED_TIMEOUT` in `outcome_unknown_codes()` (`src/execution/event.rs`) | new `src/task_completion.rs` | `crates/api-http/src/routes/` new `task_completions.rs`; `crates/api-grpc/src/service.rs` (`DetachStep`, holder check); `crates/scheduler/src/passes.rs` (detached expiry) | — |
| Push dispatch | `src/worker_dispatch.rs`: `PushEndpoint`, response verdicts (pure, like `sagat_domain::outbound::judge_response`) | new `src/push.rs` (claim for endpoint, report, withdrawal of an unsent request) | new `crates/infra-connectors/src/push.rs` (guarded client, SigV4 through the existing `aws-sigv4`, ID tokens through `jsonwebtoken`, envelope canonicalization and Ed25519 signing through `ring`); new `crates/scheduler/src/push.rs` loop; `crates/infra-postgres/src/` new `push.rs` | `apps/execution-engine/src/{loops.rs,settings.rs}` |
| Warm containers | `src/scripts.rs`: `WarmPolicy` bounds and mode | `src/sandbox.rs` (policy service, runner views, slot checks) | `crates/infra-sandbox/src/docker.rs` and `protocol.rs` (prepared and reused serve loops, residue check); `crates/api-http/src/routes/{scripts.rs,runner.rs}` | `apps/sandbox-runner/src/{lib.rs,config.rs,host.rs}` |
| Insights | `src/worker_dispatch.rs`: `InsightSubject` | `src/operations.rs` (list workers, insights) | `crates/scheduler/src/monitor.rs` (sampling), `crates/infra-postgres/src/workers.rs` | — |
| Telemetry, audit, permissions, quotas | `src/audit.rs`, `src/identity.rs`, `src/quota.rs` | `src/quotas.rs` | `crates/telemetry/src/{metrics.rs,redact.rs}` | — |
| Fleet activation (D019-001) | — | `src/instances.rs` (the six capabilities and their kinds) | — | `apps/execution-engine/src/capabilities.rs`, `apps/control-plane/src/registry.rs` |

`scripts/check-architecture.py` keeps SigV4, JWT and outbound HTTP for push in `crates/infra-connectors` only. Shared
types cross module boundaries without mappers; wire types (`api-grpc`, `api-http`) map only where the wire shape
differs (task token string, envelope JSON). No serde_json value is a business type in the domain: checkpoint
details are opaque bytes validated by the adapter, like step outputs.

## Transaction boundaries

Each item lists exactly what commits together. Lock order of a pull claim: applicable chain lock (014) → run row →
shared worker-drain fence → session row → shared namespace dispatch-catalog fence → shared deployment fences
(deployment-id order) → activity policy/quota locks. Lock order of a push claim: chain lock → run row → shard fence
(`FOR SHARE`, after the run lock as the scheduler does today) → shared namespace dispatch-catalog fence → tenant,
namespace and endpoint push fences (in that order) → activity policy/quota locks. Between the two new fence kinds
the only order is **catalog → deployment**: a catalog change (patterns or push activities) takes the catalog fence
exclusively and then the exclusive fences of the affected deployments in id order; a routing change or the fenced
retirement takes the catalog fence shared and then its deployment's fence exclusively; claims, repins and
registrations take the catalog fence shared and then deployment fences shared, after their own locks, and never an
exclusive one. The order is therefore acyclic and a pattern change and a routing change of one deployment are
serialized (no write skew). Every new fence is a transaction-scoped advisory lock, because the runtime role takes no
row locks; the shard fence of a push claim is the existing ADR-0020 row check (`FOR SHARE` through the existing
privileged path), not a new lock. In ADR-0050's merged admission lock order (coordinator decision C4) the 019 fences
occupy **slot 4**, catalog fence before deployment fence, ahead of 017/021/024/025/030 catalog heads, secrets (slot 5),
run rows (slot 6), 022 semaphores (slot 7) and quota rows (slot 8). This feature's run admission takes no 019 fence
(child pin copies are read under the parent reservation); claims, repins and Detach are transitions on an existing
run and keep ADR-0050's rule for them: run row first, then the 019 fences, quota rows last.

Corrected 2026-11-24, what the claim items below also obey (research D019-003 to D019-008):

- **Claim cost (design 038).** Routing eligibility joins the predicate of the claim's run read (`RunStore::begin_claim`),
  so an ineligible candidate is dropped without a projection load. For a step outside every deployment the shared
  catalog fence is taken inside a statement the claim already runs, and no other 019 statement runs, so the recorded
  budget of `integration_claim_transition_efficiency` is unchanged; every other added statement is a budget line
  (NFR-019-007). The poll turn (FR-038-004) rotates the start point only among candidates of the same priority.
- **Provisional leases (protocol 1.8).** A claim for a session at 1.8 or later may mark the lease provisional as ADR-0099
  says; this feature changes nothing in that marking. `DetachStep` is a capability call (refused `TASK_NOT_ACKNOWLEDGED`
  on a provisional lease; on any other lease the detach transaction writes `lease_capability_generation`); heartbeat
  details on a provisional lease are not stored; details of a hand-back are ignored.
- **Unknown outcomes (feature 037).** A detached attempt that times out ends `DETACHED_TIMEOUT` and a lapsed push lease
  `LEASE_EXPIRED`; both go to reconciliation or review in the expiry transaction, exactly as a lapsed worker lease does.
- **Gates of feature 022.** A gated step is not `READY`, so it is no candidate; the claim reads no semaphore or window
  (ADR-0089 decision 1).
- **Fleet requirements.** Each transaction that starts using a capability (a routing change to a version other than
  `UNVERSIONED`, admission with dispatch options, a heartbeat with details, Detach, a push endpoint save, a warm policy
  save) first checks that its requirement is recorded (the registry's epoch read the process already carries, ADR-0082
  decision 4), and refuses `DISPATCH_ACTIVATION_PENDING` otherwise.

1. **Run admission** (existing transaction): run row with dispatch options and versioning behaviour,
   `run.started` carrying them, initial steps with effective attributes, child pin copies from the parent, quota,
   audit, outbox. A dispatch expression that fails refuses admission before anything is written.
2. **Step activation** (every existing writer): the domain computes effective priority, key, weight and behaviour;
   the store writes them on the step row in the transaction that makes it `READY`.
2a. **Registration with a deployment identity** — the existing binding advisory lock, then the shared catalog fence
   and the shared deployment fence (so the manifest is frozen against committed patterns); the per-deployment version
   creation rate (20 per hour) is checked; refuse a retired build or an unknown deployment; insert the version `INACTIVE` with its manifest (first
   registration, only when the identity's binding covers every pattern of the deployment, audited
   `deployment.version.registered`) or check the subset rule; then the existing session insert
   and quota check; commit. A retirement holding the exclusive fence is waited for, so a build retired before this
   commit is refused.
3. **Claim (pull)** — run lock, drain fence, session row, shared catalog fence, shared advisory fence
   `deployment:<tenant>:<id>` (an
   unversioned session whose activities span several deployments takes each fence in deployment-id order), quota
   locks; re-read the routing revision; check eligibility (pin, bucket route, unversioned rule); insert the
   pin and journal `run.deployment_pinned` when a pinned step has none; `READY → LEASED` with `step.leased`
   carrying version, dispatch mode and delivered checkpoint digest; outbox; commit. Discovery outside the
   transaction is only a hint.
4. **Routing change / ramp / drain** — shared catalog fence, then the exclusive deployment fence, CAS on
   `routing_revision`, capability check against the committed patterns, capability and
   poller validation (or audited override), version states, audit, outbox (`deployment.routing_changed` hint for
   the monitor), commit. Claims that held the shared fence committed first; later claims see the new revision.
5. **Retirement** is up to three transactions, so that no transaction takes a run or session lock after the
   exclusive fence: (a) optional repin of ≤1,000 open pinned runs, one run per transaction exactly as item 6;
   (b) the fenced retirement: exclusive fence, exact count of open pins, active and detached leases of the version
   (refused with `VERSION_NOT_DRAINED` if any), state `RETIRED`, audit, commit — from then on registration and the
   claim's eligibility check refuse the version; (c) closure of the version's sessions through the existing
   binding-revocation path, which keeps its own binding → session lock order; a crash between (b) and (c) leaves
   sessions that can claim nothing and are closed by the next retirement sweep of the monitor.
6. **Repin (single run)** — run lock, shared fence, pin update, `run.deployment_repinned`, audit, commit.
6a. **Catalog change** (deployment create/update/delete of patterns; push endpoint create/update of activities) —
   exclusive namespace dispatch-catalog fence, then the exclusive fences of the affected deployments in id order;
   overlap and deployment/push exclusivity checks; for added patterns the capability check of the deployment's
   current and ramping versions (refuse `MISSING_CAPABILITIES` unless `acceptGaps`, audited); for removed patterns
   refuse `PATTERN_IN_USE` while an open run is pinned through them to a version other than `UNVERSIONED` (counted
   from `run_deployment_pins` under both fences, so a pinned run can never fall outside its deployment); rows,
   audit, commit. Claims in flight committed before it or see the new catalog.
7. **Heartbeat with details** — when the step declares no secret: the existing heartbeat transaction (run lock,
   fence, renewal) plus the checkpoint upsert guarded by the order key `(attempt, holder epoch, sequence)` and the
   coalescing interval, with the retained-byte delta (ADR-0027); commit. When the step declares secrets: the
   heartbeat transaction renews the lease and commits first; the provider values are read outside any transaction;
   a second transaction rechecks the fence and secret versions (the `checked_secret_report` pattern of
   `crates/application/src/worker/secrets.rs`) and then stores or refuses the checkpoint. Scans run only for
   checkpoints that would be stored (after coalescing). Refusals record a bounded audit. No journal event per beat.
8. **Fail with details / lease expiry** — existing transactions; the final checkpoint is stored and the ending
   event carries its digest and size.
9. **Detach** — run lock, fence, node declares `asyncCompletion`, lease not provisional (else `TASK_NOT_ACKNOWLEDGED`,
   nothing written), `lease_capability_generation` written for the lease's generation; step marked detached, expiry =
   now + detached heartbeat timeout (bounded by the maximum detached deadline, which is never later than the attempt's
   deadline or the run deadline, D019-007), lease holder
   becomes `detached`, holder epoch 1, session in-flight released, `step.detached`, audit, commit. The `task_tokens`
   quota is exact: Detach and the push `202` path take the namespace task-token advisory lock and count detached
   attempts through the partial index `steps_detached_idx` before the transition. The token is computed from the
   committed lease and returned; replay returns the same token with `already_applied`. From this commit every gRPC
   call presenting that lease token is refused (`STALE_ATTEMPT`, reason `DETACHED`).
10. **Token verification** (first step of 10a and 11) — authenticate the principal (`task:complete`, current
    membership); decode the token; take the run lock; compare SHA-256(lease token) in constant time with the step's
    current lease hash and with the retained hashes of its ended attempts (`step_ended_leases`). No match, or a
    namespace/tenant mismatch: opaque `NOT_FOUND`. After a match, a machine identity must also hold a binding covering
    the step's activity (`FORBIDDEN`, audited) — checked only now so that the binding check reveals nothing about
    steps the caller cannot name with a real token.
    Match on an ended attempt: `CONFLICT`/`STALE_ATTEMPT`. Match on the current lease while a worker session holds it,
    or while the push invoker holds it for an attempt not allowed to complete asynchronously: `CONFLICT`/
    `NOT_DETACHED`. Only then are receipts read.
10a. **External heartbeat** — after 10: renewal (detached holder) or no renewal (push holder, whose invoker renews),
    optional checkpoint with `detailsSequence` in holder epoch 1 (as 7, two-phase when secrets are declared), commit.
11. **External completion / failure** — after 10, as `CompleteStep`/`FailStep`, including the two-phase secret scan:
    receipt keyed by (tenant, principal, token task id, idempotency key), output limits, journal, next steps,
    timers/subscriptions, outbox, audit (`task.completed_externally` / `task.failed_externally`), the ended lease's
    hash retained in `step_ended_leases`, commit. Accepted while the holder is `detached`, or `push` for an attempt
    allowed to complete asynchronously; first commit wins against the invoker's own report or `202` handling, which
    then finds the step ended and records nothing.
12. **Push claim** — run lock; shard fence (ADR-0020 `FOR SHARE` of the shard row, after the run lock) because the
    invoker acts for owned shards; shared catalog fence; tenant, namespace and endpoint push fences, counting active push
    leases through `steps_push_leases_idx` (exact ceilings); drain fence and routing are not applicable (push activities are outside deployments);
    lease, `step.leased` with `dispatchMode: push` and
    endpoint revision; outbox; commit. The HTTP request starts after commit. If the connection fails before the request
    is written, a withdrawal transaction under the run lock acts only on a step still `LEASED` with the issued generation
    and lease-token hash, appends `step.lease_withdrawn` (cause `NOT_SENT`) and returns the step to `READY` with its attempt
    set back by one (ADR-0098 decision 3), while `lease-withdrawal-v1` is recorded and the attempt has fewer than four
    withdrawals; otherwise the report path fails it `PUSH_UNAVAILABLE` (D019-008).
13. **Push report** — the invoker reports through the same application functions as a worker report, idempotency
    key `push-<task_id>`; the provider credential was resolved after the claim commit as the revision's secret
    binding, rechecked against the revision's `boundBy` principal (secret exists, its access list allows that
    principal, who is still a member with `push:manage`), else the attempt fails `PUSH_CREDENTIAL_UNAVAILABLE`
    (the ADR-0025 broker-credential precedent); lease renewals while in flight are ordinary heartbeat transactions; endpoint health and breaker
    counters are updated in the report transaction; expiry and detach end the push lease in their own transactions,
    so the indexed count needs no separate counter.
14. **Warm policy change** — policy row with revision, audit, commit. The runner reads policies over its REST
    surface; it never touches the database.
14a. **Warm slot lease** — a runner asks the control plane (runner REST surface) for a slot of a policy: namespace
    and tenant warm advisory locks, count of live slot leases against `script_warm_containers` /
    `tenant_script_warm_containers` and the policy's `max_warm`, insert the slot lease with expiry, commit; renewals
    extend it and return the current version digest of the declared secret set (also carried by every warm-eligible
    lease); a container whose slot lease is not live, or whose digest differs, MUST NOT serve a run and is evicted.
    Each execution report carries the slot id; the control plane checks it is live and matches tenant, namespace,
    artifact and secret set, and records `temperature` (`cold`, `prepared` or `reused`) with the execution and the attempt. Quotas are
    therefore exact across runners.
15. **Insights sample** — leased monitor round writes the namespace's snapshot rows; no run lock.
16. **Session activity** — conditional `UPDATE … WHERE last_x_at < now() - 5 s` in the poll and heartbeat
    transactions after their own work; it never blocks them on a row lock (advisory fence not needed: a lost
    coalesced write is harmless).

Provider I/O (OpenBao resolution of push credentials, HTTP invocation, ID-token minting) never happens inside a
transaction; credentials are resolved after the claim commit and dropped when the invocation ends.

## Cross-feature registries this design is entered in (coordinator decisions of 2026-10-27)

| Registry (owner) | 019 entries | Task |
|---|---|---|
| Reset rules for journal events (018, `specs/018-schedules-bulk-operations/contracts/journal-and-events.md`, C1) | `run.deployment_pinned`, `run.deployment_repinned` → `REMATERIALIZE`; `run.started` dispatch, `step.leased` details, `step.detached`, ending-event checkpoint details → `COPY` ([contracts](contracts/errors-and-events.md#journal-events-additive)); 018's table already lists these rows (checked 2026-11-24) | T019-068 |
| Merged admission lock order (ADR-0050, C4) | slot 4: 019 catalog fence → 019 deployment fence | T019-068 |
| Placement rule (ADR-0055, C3) | push envelopes are case (b) | T019-068 |
| FR-016-002 carriers (016 amendment, C6) | `heartbeatSchema` on `activity` and `script` | T019-067 |
| Encoded-field registry (031, C7) | expression position: `{expr}` fairness key; carrier: `heartbeatSchema` | T019-068 |
| Content stores (020, C8) | `step_checkpoints.details`, `runs.fairness_key` | T019-068 |
| Key inventory (032, C11) | push signing key (`SAGAT_PUSH_SIGNING_KEY` and retired public keys) | T019-068 |
| Time classes (028, C13) | detached-lease expiry and token validity `infrastructure`; maximum detached deadline `workflow_due` ([data model](data-model.md#time-classes-028)) | T019-068 |
| Batch actions (018, C14) | `REPIN` uses this feature's single-run repin; `Action::Repin` exists in `crates/domain/src/batch.rs` with only `run:operate`, and this feature adds `deployment:manage` and types its `repin` parameter (D019-014) | T019-025 |
| Instance registry (ADR-0082) | requirements `worker-deployments-v1`, `dispatch-priority-v1`, `heartbeat-checkpoints-v1`, `async-completion-v1`, `push-dispatch-v1`, `warm-scripts-v1` (D019-001) | T019-069 |
| Worker-protocol minors (`sdks/SPEC.md` §9, OD-2026-11-15-8) | the next three free minors (1.11 to 1.13 at the time of writing; design 044 took 1.10, Decision 044-255) | T019-003 |
| OCSF mapping (design 040, FR-040-001) | every new audit action | T019-009 |
| Script host calls (023, C17) | reserved `checkpoint` call | T019-032 |
| Permit order (022, C19) | 022 grants by this priority (1 first), then enqueue time | T019-042 |

## Authorization

| Action | Permission (new in bold) | Built-in roles | Notes |
|---|---|---|---|
| List workers, deployments, versions, insights, push endpoints, warm policies | `ops:read` / `script:read` | admin, developer, operator, viewer | counts, ages, names only |
| Create/update/delete deployment; set current; ramp; retire | **`deployment:manage`** | admin, operator | reason required, audited |
| Repin one run | **`deployment:manage`** + `run:operate` | admin, operator | audited with reason |
| Register as a version | `worker:execute` + binding | worker, admin | version grants no authority |
| Start with dispatch options | `run:start` | admin, developer, operator | |
| Detach | `worker:execute` + current lease + binding | worker | node must declare `asyncCompletion` |
| External heartbeat/complete/fail | **`task:complete`** + token (+ a binding covering the step's activity for a machine identity) | admin, worker | principal is the audited actor; an `async` push function needs its own such identity (D1058) |
| Manage push endpoints | **`push:manage`** + `secret:use` on referenced secrets | admin | the revision records `boundBy`; at each invocation the secret's ACL is rechecked against `boundBy`, who must still be a member with `push:manage` |
| Read push signing public keys | `namespace:read` | all members | public material |
| Manage warm policies | **`script:warm:manage`** | admin | residue acknowledgement |
| Read checkpoint values | existing disclosure (`payload:read-internal`, `payload:reveal-pii` grant) | per ADR-0033 | structure-only by default |

`Role::ADMIN_PERMISSIONS` gains the four new permissions (the catalog grows from 53 to 57; corrected 2026-11-24); `Operator` gains `deployment:manage`; `Worker` gains
`task:complete`. `Runner` and `DatabaseRunner` gain nothing: the runner's warm-slot routes require its existing
`script:execute`, scoped to its token's tenant. Foreign resources are opaque `NOT_FOUND`; a denied
call is audited with the existing bounded deduplication.

## Finite ceilings

| Resource | Default | Hard ceiling |
|---|---:|---:|
| Deployments per namespace (`worker_deployments` quota) | 20 | 100 |
| Activity patterns per deployment | 64 | 256 |
| Non-retired versions per deployment | 20 | 100 |
| Version records incl. tombstones per deployment | 1,000 | 1,000 |
| New versions created per deployment per hour | 20 | 100 |
| Build id length | 128 characters | 128 |
| Ramp | 0–10,000 basis points | — |
| Pins per run | deployments of the namespace | 100 |
| Runs repinned by one retirement | 1,000 | 1,000 |
| Priority levels | 1–5 (default 3) | 5 |
| Fairness key length / weight | 64 characters / 1 | 64 / 100 |
| Fairness discovery pages per poll | 4 | 16 |
| Checkpoint bytes per heartbeat (`heartbeat_detail_bytes` policy) | 16 KiB | 64 KiB |
| Stored checkpoint writes per attempt | 1 per second | 1 per second (final always) |
| Detached heartbeat timeout | 5 min | 24 h |
| Detached maximum duration | the attempt's deadline | 30 days (2,592,000,000 ms), never past the attempt's deadline or the run deadline |
| Detached attempts per namespace (`task_tokens` quota) | 10,000 | 100,000 |
| External completion output | 256 KiB inline | 256 KiB |
| Push endpoints per namespace (`push_endpoints` quota) | 20 | 100 |
| Activities per push endpoint | 64 | 256 |
| Revisions per push endpoint | 1,000 | 1,000 (never pruned; updates refused beyond) |
| Retained ended-lease hashes per step | one per ended attempt | the step's `maxAttempts` (≤100) |
| Push request timeout | 30 s | 15 min, never past attempt deadline |
| Push concurrency per endpoint | 8 | 256 |
| Push concurrency per namespace / tenant (`push_concurrency`, `tenant_push_concurrency`) | 32 / 128 | 256 / 1,024 |
| Push envelope bytes | 1 MiB | 8 MiB (`https`), 6 MB (`aws-lambda` sync), 1 MB (`aws-lambda` async) |
| Push response bytes | 256 KiB | 256 KiB |
| Push breaker | opens after 10 consecutive failures for 60 s | 100 failures / 10 min |
| Warm containers per namespace / tenant (`script_warm_containers`, `tenant_script_warm_containers`) | 0 / 0 | 8 / 32 |
| Warm uses per container (`reused` mode; a `prepared` container serves exactly one) | 100 | 1,000 |
| Warm idle / lifetime | 2 min / 10 min | 10 min / 1 h |
| Warm slot lease TTL (renewed by the runner) | 60 s | 300 s |
| Insights subjects returned | 200 | 500 |

Effective values are the minimum of request, namespace and deployment configuration. Zero is never unlimited: a
zero quota refuses new admission. Lowering a limit never retracts a committed lease, pin, detached attempt or warm
container in use; it applies at the next admission. Existing run, content, rate and session quotas still apply.
Ceilings are provisional until the benchmark of NFR-019-005 supports them; when it does not, the default is
published lower and the gap stays open.

## Failure and recovery matrix

| Failure | Expected state | Handling and test |
|---|---|---|
| Engine dies after pin insert, before commit | no pin, step `READY` | rollback; `fault/…/crash_points.rs` |
| Routing change races a claim | one order | shared/exclusive fence; 100-round race test |
| Version retired while a session polls | retirement refused while any lease is active; after it, the version is ineligible at every claim and its sessions are closed in step (c) | binding-revocation path; stale reports `STALE_ATTEMPT` |
| Pinned version has no live worker | steps `READY`, alert `STEP_UNCLAIMED`, insights flag | no automatic reroute; operator repins |
| Lost Detach response | lease already detached | repeat Detach → same token, `already_applied` |
| Detached timeout vs external completion | first commit wins | loser audited no-op; a winning timeout is `DETACHED_TIMEOUT`, an unknown outcome: reconciliation or review |
| Token of reclaimed attempt | `CONFLICT`/`STALE_ATTEMPT` only after its secret matches a retained ended-lease hash | secret first, then generation |
| Forged token (known ids, guessed secret) | opaque `NOT_FOUND`, no receipt read | constant-time hash comparison before anything else |
| Decoded token replayed on gRPC (`ResolveSecrets`, `ReadBlob`, `CompleteStep`…) | `STALE_ATTEMPT` (`DETACHED`/`PUSH_HELD`) | holder check on every lease-token call |
| Async push function completes before the invoker records its `202` | completion commits; the invoker's later handling is a no-op | push-held async-capable attempts accept external reports |
| Pattern change racing a routing change | serialized by catalog → deployment fences | race test in T019-020 |
| Out-of-order checkpoints | newest sequence kept | sequence guard |
| Push replica dies mid-request | lease lapses `LEASE_EXPIRED`, an unknown outcome | reconciliation or review (feature 037); a later attempt keeps the idempotency key; at least once |
| Push connection fails before the request is written | lease withdrawn, no attempt spent (cause `NOT_SENT`) | ADR-0098 withdrawal under its cap; otherwise `PUSH_UNAVAILABLE` |
| `DetachStep` on a provisional lease (protocol 1.8) | nothing changes | `FAILED_PRECONDITION`, `TASK_NOT_ACKNOWLEDGED`; counted as a violation by 1.8 |
| Engine of the previous build in a rolling upgrade | the capability's requirement is recorded only when every live engine has it | ADR-0082: a process without it is refused and serves nothing |
| Push timeout after send | `OUTCOME_UNKNOWN` | reconcile or manual review (003) |
| Push endpoint resolves to forbidden address | attempt fails `PUSH_DESTINATION_DENIED`, health `DENIED`, breaker holds claims | audited once per window |
| OpenBao unavailable for push credentials | retryable `PUSH_UNAVAILABLE` | no fallback, no credential cache on disk |
| Lost broker hint | step found by discovery | hints are optional (NATS) |
| Failed compensation of a push-dispatched step | manual review | existing saga semantics; compensation pinned to forward version |
| Warm container residue check fails | container evicted, run unaffected | eviction metric and audit |
| Secret rotated while warm containers hold it | evicted before next run: the lease's secret-version digest differs | `script_warm_evictions_total{reason="secret"}` |
| Push credential's binding principal leaves or loses access | `PUSH_CREDENTIAL_UNAVAILABLE` (retryable), health `FAILING` | administrator saves a new revision bound to themselves |
| Monitor lease lost | insights stale with old `sampledAt` | displayed lag |
| Restart of every engine | all state in PostgreSQL | restart e2e |

## Observability

**Spans** (attributes per ADR-0017 naming; identifiers on spans, never on metrics): `worker.claim` gains
`sagat.dispatch.mode`, `sagat.dispatch.behavior`, `sagat.dispatch.priority`; `deployment.routing.change`;
`deployment.version.retire`; `step.detach`; `task.external.report` (`sagat.outcome`); `push.invoke`
(`sagat.push.kind`, HTTP status class, `sagat.outcome`); `script.warm.acquire`.

**Metrics** (closed labels only):

- `dispatch_claims_total{mode="pull|push", behavior="pinned|auto_upgrade|unversioned|none", outcome}`
- `dispatch_wait_seconds{priority="1".."5"}` (ready to leased)
- `dispatch_fairness_pages_total`, `dispatch_fairness_exhausted_total`
- `deployment_routing_changes_total{change="set_current|ramp|drain|retire|repin"}`,
  `deployment_pins_created_total`
- `heartbeat_details_total{outcome="stored|coalesced|stale|refused_size|refused_secret"}`,
  `heartbeat_details_bytes` (histogram)
- `task_external_requests_total{op="heartbeat|complete|fail", outcome="applied|replayed|stale|not_detached|not_found|forbidden"}`,
  `detached_attempts` (gauge, monitor)
- `push_invocations_total{kind="https|aws_lambda", outcome="success|failure|rejected|unknown|denied|timeout"}`,
  `push_invocation_duration_seconds{kind,outcome}`, `push_inflight` (gauge), `push_breaker_open_total`
- `script_warm_acquisitions_total{outcome="hit|miss"}`, `script_warm_evictions_total{reason="residue|limit|uses|idle|lifetime|policy|trust_key|secret|slot_lost"}`,
  `script_warm_containers{state="idle|busy"}`; `script_start_seconds{language,temperature}` gains the values
  `prepared` and `reused`; `worker_leases_undelivered_total` (ADR-0098) counts push withdrawals under its existing labels

**Logs**: structured and redacted; task tokens (`sgtt1.` prefix), provider credentials, signatures, envelopes,
checkpoints and fairness keys (logged as digest) never appear. `crates/telemetry/src/redact.rs` learns the token
prefix; a canary test covers each new path.

**Audit actions** (same transaction as the decision): `deployment.created|updated|deleted`,
`deployment.version.registered`, `deployment.routing.changed`, `deployment.version.retired`,
`run.deployment_repinned`, `step.detached`, `task.completed_externally`, `task.failed_externally`,
`task.token.denied` (bounded), `push_endpoint.created|updated|paused|resumed|deleted`, `push.destination_denied`
(bounded), `heartbeat.details_refused` (bounded), `script.warm_policy.changed`, `script.warm.evicted` (reported by
the runner, bounded).

**Alerts**: new monitor kinds `VERSION_DRAIN_STALLED`, `PUSH_ENDPOINT_FAILING`, `UNVERSIONED_POLLERS_IDLE`
(additive AsyncAPI enumeration values, `docs/event-schema-evolution.md` rule 3) with runbooks.

## Migration and compatibility

- **Schema**: additive migrations (`worker_deployments`, `worker_deployment_patterns`, `worker_deployment_versions`,
  `run_deployment_pins`, `step_checkpoints`, `step_ended_leases`, `push_endpoints`, `push_endpoint_revisions`, `script_warm_policies`,
  `script_warm_slots`, `worker_session_activity`, `namespace_dispatch_insights`; nullable columns on `runs`, `steps`, `worker_sessions`;
  expression index `steps_ready_priority_idx` on `coalesce(priority, 3)`, used verbatim by the claim queries; no
  stored generated column, which would rewrite `steps` under an exclusive lock). Grants to `sagat_app` in the same files; downs refuse to drop rows older code
  cannot read; every variable-size column classified in `crates/infra-postgres/storage-classification.json`. The
  index is built `CONCURRENTLY` in its own migration; the runbook states lock time.
- **Protocol**: three consecutive minors (`1.N` deployment identity and dispatch facts, `1.N+1` checkpoints,
  `1.N+2` detach), each a superset of the previous, each advertised only after its slice's evidence passes and the
  store declares its path (`protocol_ceiling` pattern). Workers below `1.N` (1.0 to 1.9 at the time of writing) are served exactly as today and are unversioned. Minors are cumulative: a 019 host also carries the 1.8 and 1.9 host procedures.
- **REST**: optional properties and new operations only; `PlatformVersion.features` (D1073).
- **IR**: additive fields under ADR-0032; publication refuses a field whose capability the server does not enable.
- **Runs in flight**: old runs and steps have null dispatch columns, read as priority 3, no key, auto-upgrade,
  no pin; their behaviour is unchanged. A deployment created over activities with in-flight runs routes them to
  `UNVERSIONED` until an operator changes routing.
- **Fleet activation** (D019-001): each capability is used only after the operator records its requirement in the
  instance registry (ADR-0082); before that its surfaces refuse `DISPATCH_ACTIVATION_PENDING` and its minor is not
  offered, after it a process without the capability is refused and serves nothing.
- **Rollback**: disable the capabilities (no new deployments, detach, push, warm policies); readers keep decoding
  stored pins, checkpoints and detached attempts until they drain; an older engine is not a safe reader of runs
  with detached attempts (ADR-0032 point 8) and the runbook says so; while a requirement is recorded the registry
  refuses such an engine.

## Constitution Check

| Principle | Status | Evidence in this design |
|---|---|---|
| I. State and retry semantics | Pass | Pins, routing, checkpoints, detach, external and push reports commit in the run/claim transactions listed above; first commit wins; stale fences refused; terminal runs never reopen |
| II. Ports and adapters | Pass | pure routing, bucket, fairness and verdicts in `crates/domain`; SigV4/JWT/HTTP in `crates/infra-connectors`; no mapper without semantics |
| III. At-least-once effects | Pass | push and external completion keep idempotency keys; timeout after send is `OUTCOME_UNKNOWN`; a detached timeout (`DETACHED_TIMEOUT`) and a lapsed push lease (`LEASE_EXPIRED`) are unknown outcomes that go to reconciliation or review, never to a blind retry (feature 037); a checkpoint never authorizes a retry; compensation pinned to forward version |
| IV. Versioned and pinned compatibility | Pass | workflow versions unchanged; deployment pins orthogonal; additive minor, REST, IR and AsyncAPI; six-SDK conformance; each capability switched on by a fleet requirement (ADR-0082) |
| V. Isolation, tenant boundary, secrets | Justified | tenant-keyed tables, opaque `NOT_FOUND`, credentials per invocation; prepared warm containers keep one container per run; only the acknowledged `reused` mode relaxes per-run isolation (Complexity Tracking, ADR-0052) |
| VI. Observability and audit | Pass | spans, closed-label metrics, redaction, audit in the decision's transaction, displayed insights lag |
| VII. Inbox/outbox contract | Pass | hints remain optional; push invoker discovers by query; external calls deduplicated by receipts |
| VIII. Failure tested honestly | Pass | crash points, duplicates, timeout/completion race, lost hint, failed compensation, restart, tenant and quota tests planned in `tasks.md`; benchmarks are criteria |

Re-evaluated after design: no unjustified violation.

## Complexity Tracking

| Violation | Why needed | Simpler alternative rejected because |
|---|---|---|
| The `reused` warm mode reuses a container across runs (ADR-0052 supersedes ADR-0013 in part, coordinator decision C22) | runner capacity per run for a high rate of short scripts; the latency need is met by the `prepared` mode, which keeps one container per run | per-run containers remain the default and `prepared` is the default warm mode; in-process reuse rejected for residue (O1042); ADR-0052 states the risk and an acknowledgement is required |
| Engine process makes outbound calls to customer functions with provider credentials | push dispatch without customer workers; a fixed, platform-defined envelope to a tenant-configured destination is case (b) of ADR-0055's placement rule (coordinator decision C3), the ADR-0025 precedent | a separate push runner (like `apps/database-runner`) duplicates the outbound delivery path; kept as the escape hatch if the security review requires it (G1044) |
| Claim discovery gains routing and fairness predicates | routing must be decided in the claim (ADR-0051) | per-version queues would be a second authority; pre-assignment is forbidden by the worker protocol decision |

## Project Structure

### Documentation (this feature)

```text
specs/019-worker-deployments-dispatch/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── README.md
│   ├── openapi-fragment.yaml
│   ├── worker-protocol.md
│   ├── workflow-ir.fragment.schema.json
│   ├── push-envelope.md
│   └── errors-and-events.md
├── checklists/requirements.md
└── tasks.md
```

### Source code (real paths; new files marked)

```text
crates/domain/src/{worker_dispatch.rs (new),execution/command.rs,execution/event.rs,identity.rs,audit.rs,quota.rs,scripts.rs}
crates/application/src/{worker.rs,deployments.rs (new),task_completion.rs (new),push.rs (new),operations.rs,sandbox.rs,quotas.rs,payload_disclosure.rs,instances.rs,batch.rs,runs.rs,chain_progress.rs}
crates/infra-postgres/src/{workers.rs,deployments.rs (new),checkpoints.rs (new),push.rs (new)} + migrations/
crates/infra-connectors/src/push.rs (new); crates/infra-egress/src/lib.rs (reused)
crates/infra-ir/src/{convert.rs,schema.rs}; crates/infra-sandbox/src/{docker.rs,protocol.rs}
crates/scheduler/src/{fairness.rs,monitor.rs,passes.rs,push.rs (new)}; crates/telemetry/src/{metrics.rs,redact.rs}
crates/api-grpc/src/service.rs; crates/api-http/src/routes/{operations.rs,scripts.rs,runner.rs,deployments.rs (new),task_completions.rs (new),push.rs (new)}
apps/{execution-engine,control-plane,sandbox-runner,cli}/src/; runners/{go,java,dotnet}/ (script checkpoint host call)
frontend/apps/studio/src/features/{operations,scripts,runs}/
sdks/{typescript/src,go,python/sagat_octopus_orchestrator,dotnet/src/SagatOctopusOrchestrator.Sdk,java/sdk,rust/src}/
tests/{contract,integration,fault,e2e,performance}/worker-deployments-dispatch/ (new; the contract target split into rest/ and grpc/); tests/contract/sdk-conformance/scenarios/; tests/integration/claim-transition-efficiency/; tests/integration/inline-code-sandbox/
```

**Structure Decision**: extend existing modules where the semantics live; one new domain module (`worker_dispatch.rs`; `dispatch.rs` names the event dispatcher of feature 004)
holds the pure rules shared by the claim, the scheduler and the invoker.

## Delivery sequence

Corrected 2026-11-24: the order of delivery is the two-lane plan of [tasks.md](tasks.md#build-slices-2026-11-24). Lane A
builds the server (domain, application, store, gRPC, REST, scheduler, IR, engine and control plane) in twelve slices,
from the foundations through deployments and routing, inheritance, checkpoints, detach, priority, push, insights and
the warm control plane; lane B builds, in parallel and never in a crate lane A is changing, telemetry, the push adapter,
the six SDKs, the CLI, the Studio, the sandbox runner and the examples. The story order is kept within lane A: US01
and US02 (P1), US03 (P1), US04, US05, US06 and US07 (P2), US08 (P3).

Each slice is releasable behind its capability; a slice is not advertised (protocol, `features`, IR) until its
fault, security and six-SDK evidence passes and its fleet requirement can be recorded.

## Verification plan

- **Unit**: bucket function equality domain/SQL, routing table of behaviours, fairness properties (reuse the
  ten properties of `fairness.rs`), token codec, envelope canonicalization and signature, verdict mapping.
- **Integration (real PostgreSQL, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`)**: claim predicate, pins, fences,
  checkpoints, detach, external reports, push claim capacity, insights sampling, migrations up/down, privileges.
- **Contract**: OpenAPI/proto/IR/AsyncAPI lint and baselines; gRPC compatibility of every minor below `1.N`; REST battery.
- **Security**: cross-tenant tokens/endpoints/deployments; SSRF and metadata; canaries for tokens, credentials,
  checkpoints and keys; warm residue suite (reused mode) and single-use test (prepared mode); hostile push function
  responses (oversize, redirects, slow body).
- **Fleet**: each requirement unrecorded (surfaces refuse, minor not offered), recorded, and a mixed fleet with an engine
  of the previous build (no routed lease, no gRPC call served on a detached lease); `DetachStep` and details on a
  provisional lease of 1.8; the claim budget lines of `integration_claim_transition_efficiency`.
- **Fault**: crash at each commit point of the transaction list; duplicate Detach/completion/push response;
  timeout vs completion race; lost NATS hint; failed compensation of push and pinned steps; full restart.
- **E2E**: quickstart happy and failure demos with real engine, runner and a local HTTPS function.
- **Conformance**: `deployment-versioning`, `heartbeat-details`, `async-completion`, `dispatch-priority`,
  `push-envelope` scenarios in all six SDKs, goldens reviewed per ADR-0015.
- **Performance**: NFR-019-005 criteria with environment, configuration and failures reported in
  `tests/performance/reports/worker-deployments-dispatch.md` (new); no figure is published without it.
