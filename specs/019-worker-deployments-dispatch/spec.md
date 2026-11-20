# Feature Specification: Worker deployments and dispatch

**Feature Branch**: `feature/019-worker-deployments-dispatch`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: Bring the dispatch features of Temporal (Worker Versioning, Task Queue priority and fairness, heartbeat details, asynchronous activity completion) and Windmill (dedicated workers) to the Sagat engine without a second authority for work: worker deployment versions with deployment and build identity, a capability manifest per version, per-run and per-step routing **pinned** or **auto-upgrade**, a ramp percentage, drain and safe retirement, upgrade at Continue-As-New (feature 014), interplay with protocol-minor negotiation (ADR-0015) and worker bindings and pools (features 009 and 010); priority keys and fairness keys with weights per run and step inside the existing tenant fairness; bounded, classified heartbeat checkpoint details recorded with heartbeats and given to the next attempt; asynchronous completion by task token bound to attempt, generation and fencing, with expiry and audit; push (serverless) dispatch in which an engine-side invoker calls a registered function endpoint (HTTPS, AWS Lambda, Cloud Run) with a signed, attempt-scoped lease under the egress guard, at least once, with no credential leaving the platform except the attempt capability; opt-in warm dedicated script workers per pinned artifact digest and tenant/namespace, analysed against ADR-0013; and worker insights (pollers, last heartbeat, versions, backlog and lag per pool and version) in the API and the Studio.

## Context and delivery boundary

Today a worker identity is bound to activity patterns of a namespace (FR-009-002), registers a session that
advertises exact activity names, negotiates a minor of protocol 1 (ADR-0015, today 1.6) and claims `READY` steps
oldest first through one canonical claim transaction (`docs/architecture/execution-semantics.md`, "Worker-session
claim admission"). Any session of any build that advertises an activity may take its next step. An operator can
drain a worker *identity* (FR-008-004) but cannot route new work to a new build gradually, keep a long run on the
build that started it, or retire a build only when nothing needs it. A step's lease lives 30 s by default and
dies with its worker; a worker cannot hand an attempt to an external system that completes it hours later. Steps
are taken oldest first within a namespace, so one customer's backlog delays every other customer of the same
tenant. Every script run starts and removes its own container (ADR-0013), which costs a measured 443 ms (TypeScript)
and 664 ms (Python) p50 per run on the reference machine of `tests/performance/reports/inline-code-sandbox.md`.

This feature adds those capabilities as **variations of the attempt lease**, never as a second authority:
routing, push invocation and external completion are all decided in the canonical claim/report transactions,
fenced by attempt, generation and lease token (ADR-0051, proposed). Warm script containers are an explicit,
opt-in exception to "one container per run" with its consequences stated (ADR-0052, proposed, which supersedes
ADR-0013 in part).

The workflow engine interprets a declarative IR; it does not replay workflow code (ADR-0003). Temporal's
Worker Versioning exists largely to keep *workflow code replay* deterministic; that reason does not apply here.
In Sagat a deployment version is the build of the **activity implementations** that serve steps: pinning keeps a
run's steps on activity code with the contract the run started with, and ramping limits the blast radius of a
new build. The workflow version pin (constitution IV, FR-002-008) is unchanged and independent.

**Not in this feature:** concurrency limits by key, step result caches and standalone activities (022); run list
query language and search attributes used to select runs by version (020); batch operations such as repinning
thousands of runs (018); Workflow Streams that show progress to *clients* (026: heartbeat details here go only to
the next attempt and to authorized inspection); additional script languages (023); custom roles (029);
multi-region routing of push endpoints and task tokens (032).

## User Scenarios & Testing *(mandatory)*

### US01 — Roll out a new worker build gradually and retire the old one safely (Priority: P1)

A platform team deploys build `2026.10.2` of the `payments` workers next to `2026.10.1`. The new build receives
nothing until an operator ramps 10 % of runs to it, makes it current, and later retires the old build once no run
needs it.

**Why this priority**: without versioned routing every deploy is all-or-nothing for in-flight runs; US02 and US07
build on it.

**Independent Test**: two worker processes of different builds; register, ramp, set current, drain, retire; the
journal shows which version served each attempt and none was served by a version the routing did not select at its
claim.

**Acceptance Scenarios**:

1. **Given** deployment `payments` owning `payments.*` with current `2026.10.1`, **When** a `2026.10.2` worker
   registers, **Then** version `2026.10.2` exists `INACTIVE` with the manifest it advertised and is leased nothing.
2. **Given** `2026.10.2` ramping at 10 %, **When** 1,000 auto-upgrade runs start, **Then** each run is routed by a
   deterministic bucket of its run id, stays on its version while the ramp is unchanged, and the share served by
   `2026.10.2` equals the share of buckets below the ramp.
3. **Given** a stale expected routing revision, or a version whose manifest misses an owned activity, **When** it is
   set current, **Then** the change is refused (`ROUTING_REVISION_CONFLICT`, `MISSING_CAPABILITIES`) unless, for the
   second, the operator explicitly accepts the gap with an audited reason.
4. **Given** `2026.10.1` is neither current nor ramping and has open pinned runs, **When** it is retired, **Then** the
   request is refused (`VERSION_NOT_DRAINED`) with counts; once drained, retirement succeeds, its sessions are closed
   and a later `2026.10.1` registration is refused (`VERSION_RETIRED`).

**Failure scenario:** a routing change commits while a claim is in flight; the claim either commits before the change
or sees the new revision, so no lease goes to a version the committed routing did not select.

---

### US02 — Keep a run on the build that started it, and upgrade at Continue-As-New (Priority: P1)

The `settlement` workflow is **pinned** because its activities change contract between builds: its runs finish on
the build that served their first step, while new runs follow the current version; a long-lived chain moves to the
current build at Continue-As-New when the author asks.

**Why this priority**: pinning makes an incompatible activity change safe to deploy with runs in flight.

**Independent Test**: change the current version between steps of pinned and auto-upgrade runs and of a chain, and
check every later lease, retry, compensation, child and successor.

**Acceptance Scenarios**:

1. **Given** a pinned run first served by `2026.10.1`, **When** `2026.10.2` becomes current, **Then** every later
   step of that run in the deployment, including retries, compensations and reconciliations, goes to `2026.10.1`.
2. **Given** an auto-upgrade run, or an `autoUpgrade` step inside a pinned run, **When** the current version changes
   before its claim, **Then** that step goes to the version routed at its claim and the run's pin is unchanged.
3. **Given** a pinned parent, **When** it starts a child, **Then** the child inherits pins and dispatch options
   unless the `child` node overrides them.
4. **Given** a pinned segment whose `continueAsNew` declares `deployment: current`, **When** the handoff commits,
   **Then** the successor has no pins and the same workflow version (FR-014-012); with the default `inherit` it has
   the predecessor's pins, copied in the handoff transaction.
5. **Given** an operator repins one run to `2026.10.2` with a reason, **When** it commits, **Then** the attempt in
   flight finishes under its fence, the next claim goes to `2026.10.2`, and journal and audit record who and why.

**Failure scenario:** retirement of a version with open pinned runs is refused unless a repin target is given; a
pinned run is never stranded silently.

---

### US03 — Report progress and resume from the last checkpoint (Priority: P1)

A worker processing a 2 GB file records `{offset: 1288490188}` with its heartbeats; after it is killed, the next
attempt receives that checkpoint and continues from the offset.

**Why this priority**: long activities are the most expensive to retry; US04 and US06 reuse checkpoints.

**Independent Test**: record three checkpoints, kill the worker, let the lease expire; the next attempt reads the
last committed checkpoint and its attempt number; an oversized or secret-bearing checkpoint is refused without losing
the lease.

**Acceptance Scenarios**:

1. **Given** a worker at minor `1.N+1` or later, **When** it attaches details to a heartbeat, **Then** they are stored
   in the heartbeat's transaction, replacing the previous checkpoint only if their sequence is newer.
2. **Given** an attempt that ended after recording a checkpoint, **When** the next attempt is leased, **Then** its
   lease carries the checkpoint and the attempt that recorded it, and the journal records digest and size, never the
   value.
3. **Given** details over the ceiling or containing a secret value handed to the attempt, **When** sent, **Then** the
   lease is renewed, the details are not stored, and the answer says `REFUSED_SIZE` or `REFUSED_SECRET`.
4. **Given** a `heartbeatSchema` with classified fields, **When** an operator inspects the step, **Then** values are
   disclosed only per ADR-0033 (undeclared ⇒ `pii`); a script step can record and read checkpoints through its shim.

**Failure scenario:** heartbeats with details arrive out of order; the older is ignored by its sequence.

---

### US04 — Hand an attempt to an external system that completes it later (Priority: P2)

An activity submits a document to a review system and returns; hours later the review system heartbeats and reports
the outcome through the platform's API with the task token it was given.

**Why this priority**: long external work without holding a worker; `waitEvent` already covers part of it.

**Independent Test**: an activity declaring asynchronous completion detaches, an external client heartbeats and
completes with the token, the run continues; a token of a reclaimed attempt is refused.

**Acceptance Scenarios**:

1. **Given** a node declaring `asyncCompletion`, **When** its worker detaches, **Then** it receives a token bound to
   attempt and generation, the lease expiry becomes the declared detached heartbeat timeout, and the session's slot is
   released; a node without the declaration is refused (`ASYNC_COMPLETION_NOT_DECLARED`).
2. **Given** a detached attempt, **When** a principal holding `task:complete` completes it with the token and an
   idempotency key, **Then** the step completes in one transaction exactly as a worker completion; the same key and
   body replays, another body is refused (`IDEMPOTENCY_KEY_REUSED`).
3. **Given** the detached timeout expiring while a completion arrives, **When** both race, **Then** the first commit
   wins and the loser is an audited no-op (`STALE_ATTEMPT`).
4. **Given** a forged, malformed or foreign token, **When** used, **Then** the answer is the opaque `NOT_FOUND`; only a
   token whose secret matches an ended attempt of this step gets `CONFLICT`/`STALE_ATTEMPT`; a token replayed on any
   gRPC worker call is refused (`DETACHED`); the token never appears in logs, spans, audit or responses.

---

### US05 — Serve urgent work first and share capacity fairly between customers (Priority: P2)

One namespace serves all customers of a tenant: a burst of 50,000 runs from customer A must not delay customer B, and
fraud checks go before nightly reports.

**Why this priority**: common production need; tenant fairness of scheduler passes (ADR-0020) already exists, this
refines inside a namespace.

**Independent Test**: one session, a priority-3 backlog of keys A and B, priority-1 steps arriving later; check claim
order.

**Acceptance Scenarios**:

1. **Given** ready steps of priorities 1, 3 and 5, **When** a worker polls, **Then** it is leased 1 before 3 before 5
   regardless of age.
2. **Given** keys A (weight 1) and B (weight 3), each with 1,000 ready steps, **When** 400 claims are made, **Then** B
   receives three times A's share, oldest first within each key.
3. **Given** run options `priority`, `fairnessKey`, `fairnessWeight`, **When** steps become ready, **Then** each step
   carries the node's priority if declared, else the run's; children and successors inherit unless overridden.
4. **Given** priority and fairness, **When** steps are claimed, **Then** quotas, bindings, routing, secret pools (G48
   of feature 009), breakers and scheduler-pass tenant fairness apply unchanged, and nothing crosses a namespace.

---

### US06 — Run activities on serverless functions without running workers (Priority: P2)

A team registers its AWS Lambda function as the push endpoint of `thumbnails.render`; the engine invokes it with a
signed envelope per attempt, and the function answers with the result or acknowledges and completes later.

**Why this priority**: removes long-lived workers for sporadic activities; reuses US03 and US04.

**Independent Test**: against a local HTTPS function and a Lambda-compatible emulator: synchronous completion, `202`
plus token completion, lost response, metadata-address refusal.

**Acceptance Scenarios**:

1. **Given** a push endpoint serving `thumbnails.render`, **When** a step becomes ready, **Then** the replica owning
   the run's shard claims it through the canonical claim, invokes outside any transaction and reports under that lease.
2. **Given** the envelope, **When** the function verifies it, **Then** an Ed25519 signature with a published key covers
   body, timestamp, tenant, namespace and endpoint, and the function's verifier rejects an envelope addressed to
   another tenant; it carries no credential except, for an attempt allowed to complete asynchronously, its task token.
3. **Given** `429`/`5xx`, other `4xx`, or a timeout after sending, **When** the attempt ends, **Then** it is retryable
   honouring `Retry-After`, non-retryable `PUSH_REJECTED`, or `OUTCOME_UNKNOWN` with feature 003 reconciliation.
4. **Given** a target resolving to a private, loopback, link-local or metadata address, **When** saved or invoked,
   **Then** the egress guard refuses it, it is audited and the endpoint's health says so.
5. **Given** `aws-sigv4` or `google-oidc` auth, **When** invoked, **Then** the credential is resolved from OpenBao per
   invocation and only a signature or an audience-bound short-lived ID token leaves the platform.
6. **Given** a step that declares secrets or carries blobs, **When** it is ready, **Then** it is not push-eligible.

---

### US07 — See who serves the namespace and where work waits (Priority: P2)

Operations → Workers shows, per activity, deployment version, push endpoint and advertised pool: pollers, last poll
and heartbeat, in-flight and detached attempts, backlog, oldest ready age and drainage.

**Why this priority**: needed to decide ramps, drains and retirements and to find unserved work; read-only.

**Independent Test**: create a known backlog and sessions, wait one monitor round, compare API and Studio with
database ground truth.

**Acceptance Scenarios**:

1. **Given** sessions of two versions and a backlog, **When** an `ops:read` holder reads insights, **Then** counts and
   ages per subject come with `sampledAt` and no payload, input or key value.
2. **Given** an unversioned session bound to an activity routed to a build, **When** insights are read, **Then** it is
   flagged as receiving no routed work.
3. **Given** another tenant, **When** it asks, **Then** the answer is opaque `NOT_FOUND`; with no data the Studio
   shows an empty state, never sample values (FR-007-010), with a table view and WCAG AA (FR-007-008).

---

### US08 — Opt in to warm script containers for a latency-sensitive script (Priority: P3)

A namespace administrator enables a warm pool for one script artifact and accepts in writing that later runs of it
may observe what earlier runs leave in the container.

**Why this priority**: a latency optimization with an explicit isolation cost; nothing else depends on it.

**Independent Test**: a pool of two containers runs the artifact 50 times; each run has its own lease and process,
planted residue (file, global, child process, environment) does not reach the next run, a secret revocation evicts
the containers, and warm and cold start are measured on a stated machine.

**Acceptance Scenarios**:

1. **Given** no warm policy, **When** a script runs, **Then** behaviour is exactly ADR-0013.
2. **Given** a policy saved without the residue acknowledgement, **When** saved, **Then** it is refused
   (`WARM_RESIDUE_NOT_ACKNOWLEDGED`).
3. **Given** an accepted policy, **When** a step of that artifact and secret set runs, **Then** it runs in a warm
   container of the same tenant, namespace, artifact and secret set, in a fresh process, with cold-run semantics.
4. **Given** a warm container that was handed a secret value, **When** that secret is rotated, revoked or unusable,
   **Then** the container is evicted before its next run (the warm lease carries the declared secrets' version digest),
   each run's output, log and checkpoints are scanned for every value the container has held, and test runs never use
   warm containers.

### Edge Cases

- A deployment's patterns overlap another deployment's patterns, or cover the reserved `script.` or `sagat.`
  activity prefixes: refused at create/update.
- A worker advertises activities outside its deployment's patterns, or more activities than the version's frozen
  manifest: refused at registration (`ACTIVITY_OUTSIDE_DEPLOYMENT`, `DEPLOYMENT_VERSION_MISMATCH`).
- Ramp set to 0 % or 100 %, or the ramping version equal to the current one: 0 % keeps the ramping version
  without traffic, 100 % sends every bucket to it, and an equal version is refused.
- A pinned run's version has no live session: its steps stay `READY`, the existing `STEP_UNCLAIMED` alert fires
  and insights name the version; nothing is rerouted automatically.
- A routing change, a repin and a claim of the same run race; a session is replaced while its version is retired.
- Children and successors of pinned runs in deployments the child's activities do not use: pins of unrelated
  deployments are inherited but never consulted.
- A run of a workflow version published before this feature: it has no dispatch options and behaves as today
  (priority 3, no fairness key, auto-upgrade).
- Heartbeat details on a protocol minor older than `1.N+1`: never sent; the next attempt receives none.
- A checkpoint recorded after the worker already reported the attempt: ignored like any late heartbeat.
- Detach repeated after a lost response: same lease, same token, `already_applied`.
- An asynchronous push function completes through REST before the invoker has recorded its `202`: accepted under the
  same fence; the invoker's later handling finds the step ended.
- A push endpoint's binding principal leaves the namespace: invocations fail `PUSH_CREDENTIAL_UNAVAILABLE` until an
  administrator saves a revision bound to themselves.
- A detached attempt whose run is cancelled: the next external heartbeat answers `cancelRequested`; a completion
  after the run stopped is stale.
- A push function answers after the invoker's replica died: the late answer is lost; the attempt expires and is
  retried with the same idempotency key; a late async completion with the old token is stale.
- An AWS Lambda `Event` invocation is retried by Lambda itself up to two more times
  ([AWS Invoke API](https://docs.aws.amazon.com/lambda/latest/api/API_Invoke.html)): duplicates arrive with the
  same idempotency key and only the first completion of the current attempt commits.
- A push endpoint's DNS changes to a forbidden address between save and invocation: refused at invocation.
- A warm container's script leaves a child process or fills its scratch area: the residue check fails and the
  container is evicted; the run that left it is not failed for it.
- Quotas at zero (deployments, push endpoints, warm containers, live task tokens): refused before any work; zero is
  never unlimited.

## Requirements *(mandatory)*

### Functional Requirements

**Worker deployments and versions**

- **FR-019-001 — Deployment resource:** The system MUST provide namespace-scoped worker deployments with a unique
  name and a set of activity patterns in the binding grammar of `sagat_domain::binding` (exact name, `prefix.*`,
  not `*`). Patterns of two live deployments of a namespace MUST NOT overlap, MUST NOT cover an activity a push
  endpoint serves, and MUST NOT cover reserved script (`script.`) or managed (`sagat.`) activities. Pattern changes
  MUST take the namespace dispatch-catalog fence exclusively and then the exclusive fences of the affected
  deployments in id order (claims, registrations and routing changes take the catalog fence shared before any
  deployment fence, so catalog → deployment is the only order), MUST run the capability check of FR-019-003 against
  the current and ramping versions, MUST refuse removing a pattern while an open run is pinned through it to a
  version other than `UNVERSIONED` (`PATTERN_IN_USE`, counted under both fences), and are audited with optimistic
  concurrency; delete is refused while a non-retired version other than `UNVERSIONED` or an open pin exists.
- **FR-019-002 — Version identity and registration:** A worker at minor `1.N` or later MAY register
  with a deployment name and a build id. The first registration of an unknown build of an existing deployment
  MUST create its version in state `INACTIVE` with a frozen capability manifest (the advertised activities within
  the deployment's patterns, languages and accepted minor), and only when the identity's binding covers every
  pattern of the deployment; otherwise it is refused (`BINDING_DOES_NOT_COVER_DEPLOYMENT`). Registration takes the
  catalog fence shared, so a manifest is frozen against the committed patterns; new versions of one deployment are
  rate-limited. Later sessions of the version MUST advertise a subset
  of the manifest; a superset, an activity outside the patterns, an unknown deployment or a retired build MUST be
  refused at registration. A version never grants authority: the identity's binding and `worker:execute` remain
  required.
- **FR-019-003 — Routing state:** Each deployment MUST have one current version, which may be the deployment's
  `UNVERSIONED` pseudo-version, and at most one ramping version with a ramp in basis points (0–10,000). Changes
  MUST use an expected routing revision (CAS), take the catalog fence shared and then the deployment's exclusive
  fence (so a concurrent pattern change cannot leave the current version without an owned activity), require
  `deployment:manage`, record a reason, and be audited in the same transaction. Setting a version current or ramping MUST be refused when its manifest misses an activity the
  deployment owns or when it has no live session, unless the operator explicitly accepts that gap, which is
  audited.
- **FR-019-004 — Routing at the claim:** The routing decision MUST be made in the canonical claim transaction,
  never by pre-assignment. A run's bucket for a deployment MUST be a deterministic function of the run id and the
  deployment id, implemented identically in the domain and in SQL. A claim and a routing change MUST linearize:
  every committed lease was eligible under the routing revision in force at its commit.
- **FR-019-005 — Versioning behaviours:** A step routed through a deployment MUST be either `pinned` or
  `autoUpgrade`. The effective behaviour MUST be resolved, in order, from the node's declaration, the run's start
  option, the workflow version's default and the namespace default (`autoUpgrade` unless the owner decides
  otherwise, O1040). A pinned step MUST use the run's pin for that deployment, creating it at the step's first
  claim from the current routing and journaling it in that claim's transaction. An auto-upgrade step MUST use the
  routing in force at its claim. Compensation and reconciliation tasks MUST use the behaviour and pin of their
  forward step.
- **FR-019-006 — Children and chains:** A child run MUST inherit its parent's pins and dispatch options unless
  its `child` node overrides them. A Continue-As-New successor MUST inherit the predecessor's pins and dispatch
  options in the handoff transaction by default; a `continueAsNew` node MAY declare `deployment: current`, which
  starts the successor without pins. The successor's workflow version and chain semantics remain those of
  FR-014-012 and FR-014-013.
- **FR-019-007 — Drain and drainage:** A version that has ever been routed (current or ramping, including a ramp
  that is removed) and stops being current or ramping MUST become `DRAINING` and receive no new pins, while
  existing pins keep being served; only a version never routed returns to or stays `INACTIVE`. Drainage MUST report open pinned runs, active and
  detached leases and live sessions; it MAY be sampled for display, but retirement MUST check it exactly in its
  own transaction. A version with none becomes `DRAINED`.
- **FR-019-008 — Retirement:** Retiring a version MUST require `DRAINED`, or an explicit repin target for its
  open pinned runs up to the bounded batch ceiling, and MUST be refused while the version has an active or
  detached lease. A retired version MUST be ineligible at every later claim; its sessions MUST be closed through
  the binding-revocation path, later registrations of that build refused, and the version kept as an audited
  tombstone.
- **FR-019-009 — Repin:** An operator holding `deployment:manage` and `run:operate` MUST be able to move one open
  run's pin to another non-retired version of the same deployment (a build id or `UNVERSIONED`, named by a tagged
  value, never a bare string that could be a build id), with a reason. The attempt
  in flight MUST finish under its own fence; the change MUST be journaled (`run.deployment_repinned`) and audited in
  one transaction. Repinning many runs is 018's `REPIN` batch action, gated on this feature and requiring the same
  two permissions (coordinator decision C14); this feature specifies only the single-run operation.
- **FR-019-010 — Unversioned compatibility:** A session that registers without a deployment, or at a minor older
  than `1.N`, MUST be unversioned. Activities that belong to no deployment MUST be claimed exactly as
  today. Activities of a deployment MUST be leased to unversioned sessions only when the step's route or pin is the
  `UNVERSIONED` pseudo-version. Registration of such sessions MUST NOT be refused for that reason; insights MUST
  flag them when they receive no routed work.
- **FR-019-011 — Existing controls stay authoritative:** Deployments MUST NOT change bindings, identity drains
  (FR-008-004), secret worker pools, activity concurrency quotas, compensation breakers, scheduler shard fencing or
  the tenant fairness of scheduler passes (ADR-0020). The operator-named pools of feature 010 and their metrics MUST
  keep their meaning.

**Priority and fairness**

- **FR-019-012 — Dispatch options:** Runs MUST accept optional `priority` (1–5, 1 highest, default 3),
  `fairnessKey` (1–64 characters of `[A-Za-z0-9._:/-]`) and `fairnessWeight` (1–100, default 1) at start, and
  workflow versions MAY declare defaults (the key as a tagged `{literal}` or `{expr}` over the run input,
  evaluated at admission). Nodes MAY override priority. Values MUST be validated before admission, persisted immutably with
  the run, and inherited by children and successors unless overridden; the journal records only the key's digest.
  A key is classified (ADR-0033): an `{expr}` key takes the level of the input field it reads (`pii` when
  undeclared), and publication MUST refuse an `{expr}` whose effective level is above `internal` or that reads a
  location encoded under feature 031 (the encoded-location check of FR-031-006, code `ENCODING_READ_BY_FAIRNESS_KEY`;
  the `{expr}` key is an inspection position in 031's encoded-field registry, `specs/031-client-side-encryption/contracts/encoded-field-registry.md`, coordinator
  decision C7);
  literal and start-option keys are
  `internal`. Keys are stored as retained content (purged with the run's payloads).
- **FR-019-013 — Claim order:** Within the candidates one session may take, claims MUST be ordered strictly by
  priority, then shared between fairness keys of the same priority by weighted max-min fairness (work-conserving,
  unkeyed work sharing one implicit key), then oldest first within a key. Polling and streaming (ADR-0029) and push
  invocation MUST use the same order. Fairness discovery MUST be bounded per poll.
- **FR-019-014 — Boundaries of priority:** Priority and fairness MUST only reorder work a session is already
  entitled to take. They MUST NOT bypass quotas, bindings, routing, secret pools or breakers, and a key or
  priority MUST never move work across namespaces or tenants. Fairness keys MUST NOT appear in metric labels; logs,
  journal and insights carry only a per-tenant keyed digest (HMAC, not a plain hash of a low-entropy value), and the
  raw key is shown only to holders of `payload:read-internal`. When feature 022 is present, its concurrency-key
  permits are granted by this priority (priority 1 first) and then by enqueue time within a key (coordinator decision
  C19; 022 D1602); priority never overrides a permit, a quota or a fence.

**Heartbeat checkpoints**

- **FR-019-015 — Recording:** From minor `1.N+1` a worker MUST be able to attach JSON details to a
  heartbeat and to its final failure report. Details MUST be bounded by the effective ceiling, stored in the
  heartbeat's transaction as the latest checkpoint of that step instance only when their order key
  `(attempt, holder epoch, sequence)` is greater than the stored one (a detach starts holder epoch 1, whose external
  heartbeats carry their own sequence), and stored at most once per coalescing interval per attempt, except the
  final details of a failure report or a detach, which are always stored when valid. When the step declares secrets
  the secret scan MUST follow the existing two-phase report pattern (lease renewed first, provider read outside any
  transaction, fences rechecked before storing). A refused checkpoint MUST NOT prevent the lease renewal; the
  answer states the refusal.
- **FR-019-016 — Delivery:** A new attempt's lease MUST carry the latest committed checkpoint of earlier attempts
  of the same step instance and the attempt that recorded it. Compensation and reconciliation leases MUST carry
  the forward instance's latest checkpoint. The journal MUST record the digest and size of the checkpoint an
  attempt ended with and of the checkpoint a lease delivered, never the value. A rerun or a new run never inherits
  checkpoints.
- **FR-019-017 — Classification and secrets:** Activity and script nodes MAY declare a `heartbeatSchema`;
  checkpoint values are classified per ADR-0033 (undeclared ⇒ `pii`) and disclosed only through the existing
  payload disclosure surfaces. A checkpoint containing a secret value handed to the attempt MUST be refused.
  Checkpoints count against retained-content quotas (ADR-0027), are purged with the run's content, and never reach logs,
  spans, metrics or audit. Encoding follows 031's encoded-field registry (`specs/031-client-side-encryption/contracts/encoded-field-registry.md`, rows for 019;
  coordinator decision C7): `heartbeatSchema` of `activity` is a 031 carrier (kinds K and S — checkpoint and final
  failure details are a write path, and a delivered checkpoint is in the next lease's decode plan); `heartbeatSchema` of
  `script` cannot declare encoding (`ENCODING_NOT_PRODUCIBLE`); the checkpoint of an attempt whose payload plan is
  non-empty and whose node has no encoded `heartbeatSchema` MUST be refused with 031's `ENCODING_REQUIRED`. As for
  every refused checkpoint, the lease renewal still commits. Checkpoints are a content store
  registered in 020's content-store registry (coordinator decision C8) and follow its purge and erasure rules.
- **FR-019-018 — Script checkpoints:** The sandbox runner MUST expose checkpoint recording and the previous
  checkpoint to scripts through the reserved, capability-gated `checkpoint` host call that feature 023 defines for
  every script language (`specs/023-script-languages/contracts/shim-protocol.md`, "Reserved host calls";
  coordinator decision C17); 023 owns the message shape and gate, this feature owns its meaning (ordering,
  coalescing, storage, delivery, classification, `REFUSED_*` statuses).

**Asynchronous completion by task token**

- **FR-019-019 — Declaration:** An activity node MAY declare `asyncCompletion` with a detached heartbeat timeout
  and a maximum detached duration, both bounded and never beyond the step timeout or run deadline. Publication
  MUST refuse it when the server does not enable the capability. Their instants follow 028's time-class catalog
  (coordinator decision C13): the detached-lease expiry, and with it the token's validity, is `infrastructure` time;
  the maximum detached deadline is `workflow_due` time.
- **FR-019-020 — Detach:** The holder of a current lease of such a step MUST be able to detach the attempt. Detach
  MUST, in one transaction, mark the lease detached, set its expiry to the detached heartbeat timeout, release the
  session's in-flight slot, journal `step.detached` and audit it, and return a task token that encodes the
  attempt identity, generation and lease secret. Repeating Detach under the same lease MUST return the same
  token. No new stored secret is created. Every worker-protocol call that takes a lease token (`Heartbeat`,
  `CompleteStep`, `FailStep`, `ResolveSecrets`, `ResolveDatabase`, `ReadBlob`, `WriteBlob`, `DetachStep`) MUST
  refuse a lease whose holder is `detached` or `push` (`STALE_ATTEMPT`, reason `DETACHED` or `PUSH_HELD`), so a
  token can never be replayed on gRPC to read secrets or blobs or to report outside the REST receipt and audit.
- **FR-019-021 — External heartbeat:** An authenticated principal holding `task:complete` in the namespace (and,
  for a machine identity, a binding that covers the step's activity) MUST be able to heartbeat an attempt whose
  holder is `detached`, or `push` when the attempt may complete asynchronously, with its token, renewing its expiry within the maximum detached duration,
  optionally with checkpoint details (FR-019-015), and learn whether cancellation was requested.
- **FR-019-022 — External completion and failure:** The same principal MUST be able to complete (inline output
  within the existing 256 KiB limit) or fail (code, retryable, safe message, including the reserved
  `OUTCOME_UNKNOWN`) such an attempt with its token and a required idempotency key; a push-held attempt allowed to
  complete asynchronously is accepted before the invoker has recorded its `202` (a fast function or a Lambda
  `Event` invocation), under the same fence, first commit wins. The transaction MUST be the same as a worker
  report's: fence, receipt, output scan, journal, next steps, timers, outbox and audit. Receipts are keyed by
  principal, task and idempotency key and are read only after the token's secret has been verified. Same key and
  body replays; same key and another body is refused.
- **FR-019-023 — Expiry and races:** A detached attempt whose heartbeat timeout or maximum duration passes MUST
  fail with `DETACHED_TIMEOUT` under the step's retry policy and a new generation; its token becomes stale. A
  completion racing expiry, cancellation or another report MUST be decided by the first commit; the loser is an
  audited no-op. The token's secret MUST be verified first, against the current lease or the retained lease-token
  hashes of this step's ended attempts; any mismatch (forged, malformed, foreign) MUST be the opaque `NOT_FOUND`,
  and `STALE_ATTEMPT` or `NOT_DETACHED` MUST be returned only after a secret match, so the API is not an oracle for
  run existence, attempt or generation.

**Push dispatch**

- **FR-019-024 — Push endpoints:** The system MUST provide namespace-scoped push endpoints with immutable
  revisions: kind `https` or `aws-lambda`, target, served activities, authentication `signature-only`,
  `aws-sigv4` or `google-oidc` (credentials as OpenBao secret references), mode `sync` or `async`, request
  timeout, concurrency and pause. Saving MUST validate the destination against the namespace egress policy
  (for `aws-lambda`, the region MUST match `^[a-z]{2}(-[a-z]+)+-\d$`, equal the ARN's region, and the derived host
  is checked); management requires `push:manage` and current `secret:use` on the referenced secrets, and is audited.
  Each revision records the principal that bound its secret (`boundBy`), following the namespace secret binding of
  outbound broker subscriptions (ADR-0025).
- **FR-019-025 — Invoker:** An engine-side invoker MUST claim push-eligible steps of the runs of the shards its
  replica owns through the canonical claim transaction, with the endpoint's capacity counted in that transaction,
  invoke the endpoint with no database transaction or lock held, renew the lease while the request is in flight,
  and report under the same lease. Steps that declare secrets or carry blob input or output are not push-eligible.
  A push invocation is a delivery of a fixed, platform-defined envelope to a tenant-configured destination, sent by
  the engine through the guarded egress client holding only that destination's credential — case (b) of the
  placement rule held by ADR-0055 (coordinator decision C3), as ADR-0025 deliveries already are; tenant-authored
  request content is never sent from the engine.
- **FR-019-026 — Envelope:** Each invocation MUST carry a canonical JSON envelope signed with Ed25519 under a
  published key id, binding timestamp, tenant, namespace, endpoint, task id, attempt and generation. It MUST carry
  the idempotency key, deadline, step document, checkpoint and trace context. The attempt's task token is the only
  credential it may carry, and only when the attempt may complete asynchronously (endpoint mode `async` and the node
  declares `asyncCompletion`); using it still requires the function's own platform identity holding `task:complete`
  (D1058). The push signing key is listed in 032's key inventory with its replication and rotation (coordinator
  decision C11). Public keys MUST be readable by authenticated members. All five SDKs MUST ship an envelope verifier whose
  expected tenant, namespace and endpoint are **required** parameters, that also rejects an envelope whose
  `task.deadline` has passed, and that passes the shared vectors, including rejection of a validly signed envelope of
  another tenant.
- **FR-019-027 — Responses:** A `2xx` result envelope completes the step; an error envelope fails it with its
  stated retryability; `202` detaches the attempt only when the endpoint mode is `async` and the node declares
  `asyncCompletion` (otherwise it is a non-retryable failure); `429` and `5xx` are retryable and honour
  `Retry-After`; other `4xx` are `PUSH_REJECTED`; a timeout or broken connection after the request was sent is
  `OUTCOME_UNKNOWN`. Redirects are never followed and response bodies are bounded.
- **FR-019-028 — Credentials and egress:** The engine MUST resolve a revision's provider credential per invocation
  as that revision's secret binding, rechecking at each invocation that the secret exists and that its access list
  still allows the revision's `boundBy` principal, who must still be a member holding `push:manage`; otherwise the
  attempt fails retryable `PUSH_CREDENTIAL_UNAVAILABLE`, the endpoint health becomes `FAILING`
  (`CREDENTIAL_UNAVAILABLE`), and an administrator repairs it by saving a new revision bound to themselves. The
  credential is used only to sign the request (SigV4) or to mint a Google ID token whose audience is derived from the
  target's canonical origin (any other audience is refused at save), through the guarded client with the token
  endpoint pinned to `https://oauth2.googleapis.com/token` (never a key file's `token_uri`), valid at most one hour;
  credentials never enter the envelope, logs, journal or audit. Every invocation passes the
  guarded egress client; private, loopback, link-local and metadata destinations are refused, except through one
  explicitly named development-only exception that production configuration and images refuse at startup (tested).
  Static bearer credentials and private-network endpoints are not supported in this feature.
- **FR-019-029 — Backpressure and health:** Concurrency MUST be bounded exactly per endpoint, namespace and tenant
  (tenant, namespace and endpoint push fences taken in that order in the claim, active push leases counted under
  them).
  Consecutive failures MUST open an endpoint breaker that holds new claims for a bounded time, and endpoint health
  (`HEALTHY`, `FAILING`, `DENIED`, `PAUSED`) MUST be visible. Failed attempts follow the step's retry policy; there
  is no separate dead-letter store for push.

**Warm script containers**

- **FR-019-030 — Warm policy:** A namespace administrator holding `script:warm:manage` MAY enable a warm pool for
  one artifact digest and declared secret set, with minimum and maximum warm containers, maximum uses, idle time
  and lifetime, only with an explicit residue acknowledgement that is audited. Warm container quotas default to
  zero (disabled) and are enforced exactly through durable warm-slot leases that runners acquire and renew over
  their REST surface; a container without a live slot lease MUST NOT serve a run.
- **FR-019-031 — Warm execution:** A warm container MUST use the hardened profile of ADR-0013 plus: `/dev/shm`
  mounted read-only with size 0, a seccomp profile that also denies `ptrace`, `process_vm_readv` and
  `process_vm_writev`, and a resident idle process running under a uid different from the scripts'. It MUST serve one
  invocation at a time, each started as a fresh process by an engine `exec` into the container (the runner already
  holds the engine), with its own lease, input channel, limits and deadline. Between invocations the runner MUST kill
  the invocation's processes, wipe `/tmp`, and verify that `/tmp` is empty, that no SysV shared memory, semaphore or
  message queue exists (`/proc/sysvipc/*`), that `/dev/mqueue` is empty, and that the process list equals the
  baseline (same resident process, executable and start time); any failure evicts the container. A container MUST
  never serve another tenant, namespace, artifact digest or declared secret set.
- **FR-019-032 — Eviction and secret hygiene:** A warm container MUST be evicted when a residue check fails, a
  limit is breached, its use, idle or lifetime bound is reached, the policy changes, the runner's trust keys
  change, or any secret value it was handed is rotated, revoked or no longer usable by the publisher; the warm-slot
  renewal and every warm lease carry the current version digest of the declared secret set, and the runner evicts on
  a mismatch before starting the process. Output, log and checkpoint scanning of each run MUST cover every secret
  value the container has been handed. The runner MUST report the warm slot id with each execution; the control
  plane MUST check that the slot is live and matches tenant, namespace, artifact digest and secret set, and records
  cold or warm per attempt. Test runs MUST NOT use warm containers.
- **FR-019-033 — Fallback and parity:** When no warm container is available the runner MUST run the step cold.
  Lease, retry, timeout, cancellation and result semantics MUST be identical in both paths.

**Worker insights**

- **FR-019-034 — Session activity:** The platform MUST record the last poll and last heartbeat time per session
  with bounded write amplification, and `listWorkers` MUST additionally report deployment, build id, accepted
  protocol version and these times.
- **FR-019-035 — Insights API:** An `ops:read` holder MUST be able to read, per namespace, for each activity,
  deployment version, push endpoint and advertised pool: live pollers, last poll and heartbeat ages, in-flight and
  detached attempts, ready backlog, oldest ready age, open pinned runs and drainage, with the sample time. Values
  are counts, ages and names only; the response is bounded.
- **FR-019-036 — Studio:** The Studio MUST show deployments and versions with routing actions behind confirmation,
  push endpoints with health, warm policies, insights, and per-run dispatch facts (priority, pins, the version and
  dispatch mode of each attempt, detached state, checkpoint structure under disclosure). It MUST meet FR-007-008
  and FR-007-010 and use design tokens only.

**Cross-cutting**

- **FR-019-037 — Protocol and SDKs:** Worker-protocol additions MUST be additive minors of protocol 1, one per wire
  slice (deployment identity and dispatch facts; checkpoints; detach), each a superset of the previous and
  advertised only after its slice's evidence passes, negotiated per ADR-0015; the server MUST keep serving every
  older minor unchanged. The five SDKs MUST expose
  deployment identity, checkpoint recording and reading, detach, the asynchronous completion client, the dispatch
  start options and the envelope verifier, and pass the shared conformance scenarios.
- **FR-019-038 — Observability and audit:** Every new path MUST emit OTel spans and fixed-label metrics (no
  tenant, run, deployment, build, endpoint or key in labels), redacted logs (task tokens are credentials), and
  audit records written in the decision's transaction for every management action, routing change, repin,
  retirement, detach, external completion, push refusal and warm policy change.
- **FR-019-039 — Errors, tenant boundary and limits:** New failures MUST use the existing error codes with stable
  `details.reason` values; foreign resources MUST be opaque `NOT_FOUND`; every new resource MUST have a default and
  a hard ceiling, effective values are the minimum of request, namespace and deployment, zero is not unlimited, and
  admission fails before work.
- **FR-019-040 — Contracts and documentation:** OpenAPI, worker protobuf, workflow IR schema and AsyncAPI changes
  MUST be additive and merged into the canonical contracts with their baselines; runbooks, the CLI and the
  reference documentation MUST be updated in the same delivery.

### Amendment to feature 016

This feature adds one classification carrier to FR-016-002: an optional `heartbeatSchema` on `activity` and `script`
nodes, governing checkpoint details (FR-019-017) with the same annotation, inheritance and `pii` default as the
existing carriers (declaring 031 encoding on it is allowed only on `activity`, per 031's registry). FR-016-002 currently lists the carriers as "exactly" the workflow `inputSchema`, the workflow
`outputSchema` and the node `outputSchema` of `activity`, `script`, `waitEvent` and `child`, so this is an amendment of
feature 016, not a silent extension. Per coordinator decision C6 the first of the parity features to be implemented
opens the single amendment of FR-016-002 and classification rules version 2 for the consolidated carrier list kept in
`SPECIFICATION-INDEX.md`; this feature contributes `heartbeatSchema` to that list and does not claim rules version 2
on anyone's behalf. A version published under rules version 0 or 1 has no checkpoint carrier, so its checkpoints are
`pii` throughout. Task T019-067 carries the amendment.

### Non-functional requirements

- **NFR-019-001 (durability):** A committed pin, routing change, checkpoint, detach, external completion or push
  result survives process and broker loss; a duplicate request or redelivered hint never creates a second lease,
  pin or completion. External effects of push functions and async completers stay at least once.
- **NFR-019-002 (security):** Zero cross-tenant or cross-namespace access through deployments, tokens, endpoints,
  insights or warm containers; zero task-token, provider-credential or secret canary leakage to logs, spans,
  metrics, audit or responses. Warm residue is a stated, accepted risk only where a policy acknowledges it.
- **NFR-019-003 (compatibility):** Workers of minors 1.0–1.6, existing workflow versions and runs, and REST clients
  that send none of the new fields keep their behaviour; old runs behave as priority 3, no key, auto-upgrade.
- **NFR-019-004 (observability):** Every decision has persisted correlation (journal or audit) and bounded OTel
  evidence; telemetry export failure never changes a commit.
- **NFR-019-005 (performance, benchmark criteria only):** Measure on a declared environment the claim latency
  and discovery cost with and without priority, fairness keys and a ramp; the push invoker's overhead and
  in-flight bound; checkpoint write cost per heartbeat; warm versus cold script start. No throughput or latency
  figure is promised before it is measured.
- **NFR-019-006 (resource bounds):** Every resource in the plan's finite ceilings table is enforced at
  admission; a policy reduction never retracts work already leased, pinned or detached.

### Key Entities

- **Worker deployment:** namespace-scoped name, owned activity patterns, routing (current, ramping, ramp, revision).
- **Deployment version:** a build id of a deployment with a frozen capability manifest and a state
  (`INACTIVE`, `RAMPING`, `CURRENT`, `DRAINING`, `DRAINED`, `RETIRED`); the `UNVERSIONED` pseudo-version.
- **Deployment pin:** a run's chosen version for one deployment, created at a claim, inherited or repinned.
- **Dispatch options:** priority, fairness key and weight of a run and the effective values of each step.
- **Checkpoint:** the latest heartbeat details of one step instance, with sequence, digest, size and attempt.
- **Detached attempt and task token:** an attempt released from its worker, completed or heartbeated externally
  with a token derived from its lease.
- **Push endpoint (revision):** a registered function target with kind, auth, mode, limits and health.
- **Warm policy and warm container:** an opt-in pool for one artifact and secret set, and its bounded containers.
- **Insights sample:** counts and ages per subject of a namespace with the time they were sampled.

## Success Criteria *(mandatory)*

### Measurable Outcomes

All criteria are future evidence; none is met by this document.

- **SC-019-001:** In an end-to-end test with two builds, across 1,000 runs and every routing step (inactive, 10 %
  ramp, current, drain, retire), zero attempts are leased to a version the routing did not select at the claim's
  commit, and the ramp share equals the share of buckets below the ramp (exact, computed from the run ids),
  while bindings, drains, secret pools and quotas refuse exactly as before. (FR-019-001..004, 007, 008, 011)
- **SC-019-002:** Pinned runs complete every step, retry, compensation and child on their start version across
  three routing changes; auto-upgrade runs take the routed version at each claim; Continue-As-New with `inherit`
  and `current` produces the expected successor pins in 100 % of 200 handoffs. (FR-019-005, 006, 009)
- **SC-019-003:** Fault injection at every commit point (pin creation, routing change, retirement, repin,
  checkpoint write, detach, external completion, push claim and push report) produces no duplicate lease, pin or
  completion and loses no acknowledged change. (NFR-019-001)
- **SC-019-004:** 100 rounds of a routing change racing claims of the same runs, and 100 rounds of a detached
  timeout racing an external completion, each end with exactly one committed winner and an audited loser.
  (FR-019-004, 023)
- **SC-019-005:** After a killed worker, the next attempt receives exactly the last committed checkpoint in 100 %
  of 100 trials, including checkpoints of a later attempt replacing an earlier attempt's and external heartbeats
  after a detach; oversized and secret-bearing checkpoints are refused while the lease is renewed; a canary value
  never appears in logs, spans, metrics or audit. (FR-019-015..018)
- **SC-019-006:** A task token completes only its own current attempt; a forged, foreign or malformed token always
  gets `NOT_FOUND` (no 404/409 difference reveals existence, attempt or generation); a reclaimed token gets
  `STALE_ATTEMPT` only after its secret matched; a decoded token is refused by every gRPC worker call; an
  asynchronous push completion that commits before the invoker's `202` handling is applied once — all in 100 % of
  the security and fault battery cases. (FR-019-019..023, 027)
- **SC-019-007:** In a deterministic claim test, every priority-1 step is leased before any priority-3 step of the
  same session, and over 1,000 claims each fairness key's share is within ±5 % of its weighted max-min share;
  the scheduler-pass tenant fairness results of feature 010 are unchanged. (FR-019-011..014)
- **SC-019-008:** Push happy, async, retryable, rejected, unknown-outcome and egress-denied scenarios pass against a
  local HTTPS function and a Lambda-compatible emulator; the five SDK verifiers accept and reject the same
  envelope vectors, including validly signed envelopes of another tenant and another endpoint; two endpoints never
  exceed the tenant push ceiling together in 100 race rounds; no provider credential appears outside the outbound request. Real AWS and Google Cloud runs are
  reported separately as deployment-gated evidence. (FR-019-024..029)
- **SC-019-009:** The warm-container security suite (file in `/tmp`, write to `/dev/shm`, SysV shared memory,
  semaphore and message queue, POSIX message queue, global, child and detached processes, signal or `ptrace` of the
  resident process, environment, scratch fill, and secret eviction on a version-digest mismatch) shows no residue
  reaching the next invocation in the supported mode; warm and cold start
  are measured on a stated machine and reported with their failures. (FR-019-030..033)
- **SC-019-010:** Insights values equal database ground truth within one monitor round in 100 % of sampled checks,
  contain no payload, and pass the cross-tenant battery and a Studio accessibility check. (FR-019-034..036)
- **SC-019-011:** The five SDKs pass the new conformance scenarios with byte-identical workflow documents and equal
  normalized observations; workers at minors 1.0–1.6 pass the existing scenarios unchanged and are served as
  unversioned sessions. (FR-019-010, FR-019-037, NFR-019-003)
- **SC-019-012:** Every new quota and ceiling refuses admission at its limit and at zero before any work, and every
  new metric has only the documented closed labels. (FR-019-038, 039, NFR-019-006)
- **SC-019-013:** The REST cross-tenant battery covers every new operation, the canonical contracts pass their
  lint and baseline checks, and the happy and failure demos of `quickstart.md` run reproducibly. (FR-019-039, 040)

## Assumptions

- Worker protocol minors: the three wire slices take three consecutive minors, written `1.N` (deployment identity
  and dispatch facts), `1.N+1` (checkpoints) and `1.N+2` (detach); `1.N` is 1.7 unless another feature merges a
  minor first (G1048).
- The engine's `listWorkers`, identity drain and operator alerts of feature 008 remain the base of insights.
- Push endpoints are reachable over the public network from the engine under the namespace egress policy; private
  networks need a separate managed executor (G1044).
- OpenBao (ADR-0035) holds the provider credentials of push endpoints; workload identity federation is a gap.
- Warm containers are supported for the languages the runner builds today; 023 extends them per language.
- Time decisions use the database clock (ADR-0020); the engine's injectable clock (feature 028) is used in tests.

## Dependencies

- **Existing features:** 002 (leases, fencing, retry, timeouts), 003 (compensation, reconciliation,
  `OUTCOME_UNKNOWN`), 005 and ADR-0013/0021/0024 (runner and containers), 006 and ADR-0015/0029 (protocol,
  SDKs, conformance, streaming), 007 (Studio), 008 (operations, alerts, drains), 009 (bindings, secret pools,
  quotas, egress), 010 and ADR-0020 (shards, fairness, pools), 014 and ADR-0030/0032 (chains, additive IR), 016 and
  ADR-0033 (classification), 017 and ADR-0035 (OpenBao, managed executors).
- **New features (018–032), each cited instead of re-specified (coordinator decisions of 2026-10-27):**
  - 018: the `REPIN` batch action for many runs (C14), the reset-rule registry in which this feature's journal events
    are declared (C1), and ADR-0050's merged admission lock order, in whose slot 4 the 019 catalog fence precedes the
    019 deployment fence (C4);
  - 020: selecting runs by version or key in queries, and the content-store registry in which checkpoints and raw
    fairness keys are registered (C8);
  - 021: ADR-0055's placement rule, case (b) of which covers push envelopes (C3);
  - 022: concurrency limits by key (complementary), whose permits are granted by this feature's priority (C19);
  - 023: the reserved `checkpoint` host call for every script language (C17), and warm pools in new languages;
  - 026: client-facing progress (distinct from checkpoints);
  - 028: the time-class catalog (C13) and the time-skipping harness used by timeout tests;
  - 029: custom roles over the new permissions;
  - 031: the encoded-field registry and the encoded-location check of FR-031-006, covering `{expr}` fairness keys and
    the `heartbeatSchema` carrier (C7); envelopes carry encoded values opaquely;
  - 032: regional placement of push endpoints and tokens, and the key inventory that lists the push signing key (C11).

## Out of scope

Workflow code replay concepts (patching, `getVersion`, sticky queues, local activities); concurrency limits by
key (022); bulk repin (018's `REPIN` batch action); static bearer credentials for push endpoints; push to private networks; push endpoints
as deployment versions (Lambda aliases); asynchronous completion with blob outputs; fairness weight overrides per
key; in-process reuse in warm containers (O1042); autoscaling of push functions or warm pools by the platform.

## Owner questions

Recorded with recommended defaults in [research](research.md#owner-questions): O1040 (default versioning
behaviour), O1041 (token-only completion without an authenticated principal), O1042 (in-process reuse in warm
containers). Proposed ADRs: [ADR-0051](../../docs/adr/ADR-0051-dispatch-modes-are-variations-of-the-attempt-lease.md)
and [ADR-0052](../../docs/adr/ADR-0052-warm-script-containers-are-an-opt-in-exception-to-one-container-per-run.md).
