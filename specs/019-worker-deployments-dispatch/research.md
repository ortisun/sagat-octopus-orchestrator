# Feature 019 — Baseline, sources, decisions, gaps and owner questions

Baseline investigation: 2026-10-26, worktree `feature/018-032-parity-specs` cut from `origin/develop` at
`70f09435` (release 0.2.0). Nothing below is implemented; repository observations describe that revision.

## Repository evidence (verified paths)

- **Protocol and negotiation.** `crates/application/src/worker.rs` sets `PROTOCOL_MINOR = 6`
  (`ROOT_PROTOCOL_MINOR = 4`, `SCOPED_PROTOCOL_MINOR = 5`, `DATABASE_PROTOCOL_MINOR = 6`) and negotiates per
  ADR-0015; `protocol_ceiling` already gates a minor on what the store proves. The canonical schema
  `specs/001-platform-foundation/contracts/worker.proto` has `WorkerCapabilities{languages, activities, pools,
  max_inflight, protocol_version}` (fields 1–5), `TaskLease` fields 1–18, `HeartbeatRequest` 1–8,
  `HeartbeatResponse` 1–2, `FailStepRequest` 1–12, and no heartbeat payload.
- **Claim.** `crates/infra-postgres/src/workers.rs` (`claimable_query!`, `CLAIMABLE`) discovers candidates per
  advertised activity `ORDER BY st.ready_at LIMIT $4`, with activity concurrency quotas, compensation breakers
  and secret-pool refusal (`pool_refused!`). The claim lock order is run row → worker-drain fence →
  worker-session row → activity policy/quota locks (`docs/architecture/execution-semantics.md`). The runtime
  role cannot take row locks (`crates/infra-postgres/AGENTS.md`): fences use advisory locks or `SECURITY
  DEFINER` functions.
- **Heartbeat.** `WorkerService::heartbeat` runs under the run lock and the domain's
  `execution::heartbeat` (`crates/domain/src/execution/command.rs`), which emits `StepStarted` only on the first
  beat; there is no checkpoint storage.
- **Sessions, bindings, drains, pools.** `worker_sessions` (migration `20261009000100_execution`) stores
  advertised `pools` as self-declared labels; `worker_bindings` (`20261010000100_worker_bindings`) and
  `worker_drains` (`20261015000300_operator_actions`) exist; administrator-granted pools for secret delivery are
  `worker_binding_pools` (`20261109000000_secret_worker_pools`, G48 of feature 009). `listWorkers`,
  `drainWorker`, `undrainWorker` are in `specs/001-platform-foundation/contracts/openapi.yaml` (`x-implemented:
  '008'`) without last-poll or last-heartbeat times. The Studio shows them in
  `frontend/apps/studio/src/features/operations/WorkersPanel.tsx`.
- **Scheduler fairness and pools.** ADR-0020 shares a pass between tenants by weighted max-min
  (`crates/scheduler/src/fairness.rs`); operator pools (`SAGAT_WORKER_POOLS`) are measured by
  `crates/scheduler/src/monitor.rs` as `pool_ready_steps{pool}` and `pool_queue_lag_seconds{pool}`; G248 and
  G265 (feature 010) record their tenant-blindness.
- **Outbound egress from the engine.** `crates/scheduler/src/deliveries.rs` sends outbound webhooks from the
  execution engine through `crates/infra-connectors/src/webhook.rs` and the guarded client
  `crates/infra-egress/src/lib.rs` (resolver-checked addresses, no redirects, no environment proxies).
- **Sandbox.** ADR-0013 (container per build and per run; "one long-lived container per tenant" rejected for
  residue), ADR-0021, ADR-0024; runner in `apps/sandbox-runner/src/`, container driver
  `crates/infra-sandbox/src/docker.rs`. Measured on Apple M3 Max with Docker/runc
  (`tests/performance/reports/inline-code-sandbox.md`): warm start p50 104 ms (TypeScript) / 327 ms (Python);
  whole run (create, start, run, remove) p50 443 ms / 664 ms. The existing metric is
  `script_start_seconds{language,temperature}` (D110 of feature 005).
- **Permissions.** `crates/domain/src/identity.rs` has 48 permissions and seven built-in roles including
  `DatabaseRunner` (ADR-0036).
- **Errors.** `sdks/error-catalog.json` has 19 codes; new conditions refine them with `details.reason`.
- **IR.** `workflow.schema.json` has no dispatch, heartbeat or async declarations; `continueAsNew` has only `id`,
  `type`, `input`, `state`; extensions are additive under ADR-0032.

## External semantics consulted (2026-10-26)

- Temporal Worker Versioning — <https://docs.temporal.io/worker-versioning>: deployments and versions identified
  by deployment name and Build ID; Pinned and Auto-Upgrade behaviours; Current and Ramping versions with a
  0–100 % ramp in which the Workflow ID decides the group; statuses Inactive, Active, Draining, Drained (drainage is
  updated periodically); pinned versions carry across Continue-As-New; unversioned is the default current.
- Temporal priority and fairness — <https://docs.temporal.io/develop/task-queue-priority-fairness>: priority 1–5,
  default 3, 1 highest, strict within a partition; fairness keys as virtual queues chosen by weight within a
  priority tier, FIFO within a key; unkeyed tasks share an implicit key; weight overrides for up to 1,000 keys;
  inheritance from the caller and across Continue-As-New.
- Temporal heartbeats — <https://docs.temporal.io/encyclopedia/detecting-activity-failures>: heartbeat details are
  available to the next attempt; workers throttle heartbeats but not the final one on failure.
- Temporal asynchronous completion — <https://docs.temporal.io/activity-execution>: a task token is unique per
  attempt; completion from anywhere through the client; retries make old tokens invalid.
- Windmill dedicated workers — <https://www.windmill.dev/docs/core_concepts/dedicated_workers>: a worker dedicated
  to one script keeps a process alive in a loop between jobs; isolation between runs is not documented.
- AWS Lambda Invoke — <https://docs.aws.amazon.com/lambda/latest/api/API_Invoke.html>: `RequestResponse` (6 MB
  payload, 200) and `Event` (1 MB payload, 202, Lambda retries up to two more times and may deliver twice);
  `X-Amz-Function-Error` marks function errors; SigV4 authorization (`lambda:InvokeFunction`).
- Cloud Run service-to-service auth —
  <https://docs.cloud.google.com/run/docs/authenticating/service-to-service>: Google-signed ID token with the
  service URL as audience, `Authorization: Bearer`, `roles/run.invoker`, tokens expire after about one hour;
  external callers use a service-account key or workload identity federation.

These sources informed vocabulary and edge cases; no external behaviour is assumed without a test of ours.

## Decisions

- **D1040 — A deployment owns activity patterns, not a task queue.** Sagat has no task queues; a session
  advertises exact activities. A deployment owns binding-grammar patterns of one namespace; patterns of live
  deployments never overlap, so each activity has at most one deployment. Script (`script.`) and managed
  (`sagat.`) activities are excluded: they are pinned by digest or served by managed executors.
- **D1041 — A version is created by the first registration and starts `INACTIVE`.** This mirrors Temporal's
  inactive state and makes a worker deploy harmless until an operator routes to it. Only an identity whose binding
  covers every pattern of the deployment may create a version, so a narrowly bound worker cannot freeze a manifest
  or fill the version ceiling with predictable build ids. Its manifest is frozen so a
  later session cannot widen a version that is already current; sessions may advertise a subset.
- **D1042 — Routing is decided at the claim, not when a step becomes ready.** Many writers make steps `READY`
  (admission, completions, timers, signals, approvals); deciding there would spread routing into all of them and a
  routing change would have to retarget ready rows. The claim is one canonical transaction. The bucket is the first
  eight bytes of SHA-256(`run_id ‖ deployment_id`) read as an unsigned big-endian integer modulo 10,000 (two bytes
  would bias low buckets by about 7 %; eight bytes leave a bias below 10^-15), an `IMMUTABLE` SQL function and a domain function
  held equal by a test (the ADR-0020 `run_shard` pattern).
- **D1043 — Ramp and auto-upgrade use the per-run bucket.** A run sees one version while a ramp is unchanged,
  like Temporal's workflow-id grouping. A ramp change moves only buckets crossing the threshold.
- **D1044 — Pins are created at the first claim of a pinned step and journaled** (`run.deployment_pinned`) in that
  transaction, so replay reconstructs pins. Pins live in a tenant-keyed table; repins journal
  `run.deployment_repinned`.
- **D1045 — Behaviour precedence:** node declaration › start option › workflow-version default › namespace default.
  The namespace default is `autoUpgrade` pending O1040.
- **D1046 — Inheritance.** Children inherit pins and dispatch options at admission; Continue-As-New inherits by
  default and `continueAsNew.deployment: current` drops pins. The workflow version is never changed (FR-014-012).
- **D1047 — `UNVERSIONED` is a pseudo-version row per deployment.** A new deployment's current is `UNVERSIONED`,
  so creating a deployment changes nothing. Pins may reference it. Sessions of minors below `1.N` are unversioned
  and are not refused (ADR-0015 keeps older forms); insights flag their starvation.
- **D1048 — Drainage is sampled for display and exact at retirement.** The monitor samples counts per version;
  retirement counts open pins and active and detached leases under the deployment's exclusive fence; live sessions are
  closed afterwards through the binding-revocation path.
- **D1049 — Repin is a single-run audited override to another version (a build id or `UNVERSIONED`, tagged);**
  retirement may repin at most 1,000 open runs, one run per transaction, before its fenced step. More needs 018 batch
  operations (G1055). Repin to "auto-upgrade" is not offered: the next pinned claim would pin again.
- **D1050 — Priority is strict, 1–5, default 3; fairness key 1–64 characters, weight 1–100;** a workflow default key
  is a tagged `{literal}` or `{expr}` so `input.customerId` is never ambiguous. *Revised 2026-10-27 (review):* a key
  is classified — an `{expr}` key takes its source field's ADR-0033 level (`pii` when undeclared) and publication
  refuses one above `internal` or one that reads a location encoded under 031 (FR-031-006 check,
  `ENCODING_READ_BY_FAIRNESS_KEY`); literal and
  start-option keys are `internal`; the raw key is retained content shown only to `payload:read-internal`; logs,
  journal, steps and insights carry an HMAC digest under a per-tenant key, not a plain hash. Integers keep the
  wire and SQL simple. A key's weight in a poll window is the maximum weight among its candidates (deterministic;
  Temporal leaves conflicting weights unspecified). No per-key override table (G1042).
- **D1051 — Fairness reuses ADR-0020's weighted max-min algorithm.** The pure function moves from
  `crates/scheduler/src/fairness.rs` into the domain so the claim (application) and the scheduler share one
  implementation; scheduler behaviour is unchanged. Discovery reads at most 4 pages per poll by default (ceiling 16),
  leaving out keys that already show their share; with no keys present it is one query, as today.
- **D1052 — Compensation and reconciliation inherit the forward step's dispatch attributes and pin.** Undoing
  work must reach code that understands the forward contract; priority elevation is not introduced.
- **D1053 — Effective dispatch attributes are computed by the domain at activation** from the immutable definition
  and the run's options journaled in `run.started`, and persisted on the step row for the claim index; replay
  derives the same values.
- **D1054 — Checkpoints are latest-wins rows, not journal events.** *Revised 2026-10-27:* for a node whose input or
  output declares 031 encoded locations, encoding follows 031's encoded-field registry (`specs/031-client-side-encryption/contracts/encoded-field-registry.md`):
  `heartbeatSchema` of `activity` is a carrier (K, S), of `script` `ENCODING_NOT_PRODUCIBLE`, and the checkpoint of an
  attempt with a non-empty payload plan and no encoded `heartbeatSchema` is refused with 031's `ENCODING_REQUIRED`
  (it replaced this feature's earlier `REFUSED_ENCODING`), because the server cannot tell ciphertext from a decrypted
  value in a checkpoint. The lease renewal still commits. Journaling every beat would multiply journal
  growth by the heartbeat rate. The journal records the digest and size at attempt end and at lease delivery,
  which is enough to explain what the next attempt received.
- **D1055 — A refused checkpoint never fails the renewal.** Losing a lease because progress data was too large
  would turn a reporting error into duplicate work; the answer carries a status instead.
- **D1056 — Checkpoints are coalesced to one stored write per second per attempt,** except the final details of
  `FailStep` and `Detach`, which are always stored (Temporal does not throttle the final heartbeat on failure).
- **D1057 — A task token is an encoding of the lease, not a new secret.** It carries namespace id, task id,
  attempt, generation, instance path digest and the 256-bit lease token, prefixed `sgtt1.`. The server stores no
  new secret (only the existing lease-token hash), a lost Detach response is recovered by repeating it, and any
  reclaim (new generation) invalidates it. It is a credential: redaction learns its prefix. *Revised 2026-10-27
  (review):* the secret is verified first, in constant time, against the current lease hash or the retained hashes of
  the step's ended attempts (`step_ended_leases`, written only for leases that were detached or push-held); any
  mismatch is the opaque `NOT_FOUND`, and `STALE_ATTEMPT`/`NOT_DETACHED` follow only a secret match, so forged tokens
  with known ids learn nothing. Receipts are keyed by principal and read after the check. Every gRPC call that takes a
  lease token refuses `detached` and `push` holders (`DETACHED`, `PUSH_HELD`), so a decoded token cannot reach
  `ResolveSecrets`, `ReadBlob` or a gRPC report.
- **D1058 — External calls need an authenticated principal holding `task:complete` and the token.** Auth fails
  closed and every completion has an accountable actor; a token-only path is O1041. Consequently a push function in
  `async` mode (including Lambda `Event`) MUST hold its own platform machine identity, configured by its owner like
  any worker credential; the envelope carries the token only for attempts allowed to complete asynchronously.
  *Revised 2026-10-27:* a machine identity must also hold a binding covering the step's activity, so `task:complete`
  on the Worker role does not widen FR-009-002. External reports are accepted while the holder is `push` for an
  attempt allowed to complete asynchronously (a fast function or Lambda `Event` may complete before the invoker
  records the `202`); first commit wins and the invoker's later handling is a no-op.
- **D1059 — The push invoker runs in the execution engine,** sharded by the runs its replica owns, like outbound
  deliveries, through the guarded egress client. A separate process (like `apps/database-runner`) would be needed
  only for private networks, which are out of scope (G1044).
- **D1060 — Envelopes are signed with Ed25519 under a deployment key with key ids** (`SAGAT_PUSH_SIGNING_KEY` plus
  retired public keys). A function needs only public keys; nothing secret is handed to the customer. Because one key
  signs every tenant's envelopes and any tenant can aim an endpoint at any public URL, the verifier's expected tenant,
  namespace and endpoint are required parameters, and the vectors include a validly signed foreign-tenant envelope
  that must be rejected. Per-tenant key ids remain a possible hardening.
- **D1061 — Provider authentication is SigV4 or Google ID tokens minted per invocation from OpenBao references.**
  Static bearer tokens would be long-lived credentials leaving the platform and are excluded. *Revised 2026-10-27
  (review):* the engine resolves the credential as the endpoint revision's secret binding and rechecks the secret's
  access list at every invocation against the revision's `boundBy` principal, who must still be a member with
  `push:manage` (the outbound broker-subscription binding of ADR-0025 is the precedent); otherwise
  `PUSH_CREDENTIAL_UNAVAILABLE`, retryable, until an administrator saves a revision bound to themselves. The Google
  audience is derived from the target's canonical origin and no other is accepted, and tokens are minted through the
  guarded client against the pinned `https://oauth2.googleapis.com/token`, never a key file's `token_uri`, so an ID
  token is never sent to a host other than the service it is for. Lambda regions are validated and equal to the
  ARN's region, and the derived host passes the egress check.
- **D1062 — Steps that declare secrets or carry blobs are not push-eligible.** A function has no session to call
  `ResolveSecrets` or `ReadBlob`, and sending values in the envelope would be credentials leaving the platform.
- **D1063 — A timeout or broken connection after sending is `OUTCOME_UNKNOWN`,** reusing feature 003's
  reconciliation and manual-review semantics; it is never retried blindly for `highRisk` steps.
- **D1064 — Push capacity is exact.** The push claim takes, after the run lock and the shard fence (the order the
  scheduler already uses), the tenant, namespace and endpoint push advisory locks in that order, and counts active push
  leases through a partial index under them (bounded by the ceilings, at most 1,024 rows per tenant), so two claims for different endpoints cannot both
  pass the tenant ceiling. Push claims of one tenant are therefore serialized at that lock; the cost is measured.
- **D1065 — Warm containers run a fresh process per invocation, keyed by tenant, namespace, artifact digest and
  declared secret set,** and require a residue acknowledgement. *Revised 2026-10-27 (review):* each process is
  started by an engine `exec` into the container; the resident idle process runs under a uid different from the
  scripts'; the warm seccomp profile also denies `ptrace` and `process_vm_*`; `/dev/shm` is read-only with size 0;
  between runs the runner wipes and verifies `/tmp`, verifies no SysV IPC object (`/proc/sysvipc/*`) and an empty
  `/dev/mqueue`, and verifies the process list against the baseline (same resident executable and start time);
  keyring syscalls are already denied by the engine's default seccomp profile and the suite asserts it. A warm container saves container creation and
  removal; the 005 report gives a 443 ms whole cold run against a 104 ms start (TypeScript) but does not split
  running from removal, so the saving is a measurement of this feature (NFR-019-005), not a claim. In-process
  reuse would also save interpreter start but keeps module state between runs (O1042).
- **D1066 — Output scanning in a warm container covers every value it was handed;** any change of a handed secret
  evicts it. *Revised 2026-10-27:* scanning also covers checkpoints; the trigger for "a change" is explicit — slot
  renewals and warm-eligible leases carry the version digest of the declared secret set and the runner evicts on a
  mismatch before starting a process; the runner reports the slot id with each execution and the control plane
  checks it and records cold or warm per attempt, so the opt-in is verifiable server-side. Warm quotas are exact because a container serves only while its runner holds a durable warm-slot lease
  obtained and renewed over the runner REST surface; the control plane counts live slot leases per namespace and
  tenant in the acquiring transaction (the runner still never touches the database).
- **D1067 — Insights are sampled by the leased monitor into a bounded snapshot per namespace,** returned with
  `sampledAt`; on-demand aggregate scans per request were rejected for cost and abuse.
- **D1068 — Last poll and heartbeat times live in a separate session-activity row updated at most every 5 s,** so
  the session row locked by claims is not written on every beat.
- **D1069 — One protocol minor per wire slice:** `1.N` deployment identity and dispatch facts, `1.N+1` checkpoints,
  `1.N+2` detach; each a superset of the previous, advertised only when its slice's evidence passes and the store
  declares the path (the `protocol_ceiling` pattern). A single minor would either wait for every slice or change
  meaning between releases, which ADR-0015 rejected. Numbers are assigned at integration (G1048).
- **D1070 — Four new permissions:** `deployment:manage` (admin, operator), `task:complete` (admin, worker),
  `push:manage` (admin), `script:warm:manage` (admin). The catalog grows from 48 to 52; feature 029's custom roles
  consume them.
- **D1071 — No new `Error.code`.** Reasons and durable step failure codes are listed in
  [contracts/errors-and-events.md](contracts/errors-and-events.md).
- **D1072 — Deployment management is REST, CLI and Studio, not SDK clients.** CI pipelines use the CLI; SDKs
  receive the worker-side identity, checkpoints, detach, async completion client, start options and verifier.
- **D1073 — `PlatformVersion` gains an optional `features` list** so SDKs send dispatch options only to servers
  that accept them (an older server refuses unknown properties with `BAD_REQUEST`, which stays fail-closed).
- **D1074 — Migration timestamps are allocated at implementation** after the newest migration on `develop`
  (today `20261220000000_reveal_grants_role_change`); this draft names migrations by slug only.
- **D1075 — One namespace dispatch-catalog fence.** Deployment pattern changes and push-endpoint activity changes
  take one namespace advisory fence exclusively; every claim takes it shared before the deployment fences. A pattern
  added while a claim is in flight therefore cannot let a lease commit that the committed catalog forbids, and the
  deployment/push exclusivity check cannot be passed by two concurrent saves. *Revised 2026-10-27 (review):* the
  only order between the two fence kinds is catalog → deployment. A catalog change takes the exclusive catalog fence
  and then the exclusive fences of the affected deployments; a routing change and the fenced retirement take the
  catalog fence shared before their exclusive deployment fence; registration takes both shared. This removes the
  write skew between a pattern change and a routing change, and `PATTERN_IN_USE` is decided on open pins counted
  under both fences, so a pinned run never falls outside its deployment.
- **D1076 — Checkpoint order key is `(attempt, holder epoch, sequence)`.** A later attempt always replaces an earlier
  one; a detach starts holder epoch 1 so external heartbeats (`detailsSequence` in REST) order among themselves; final
  details take the maximum sequence of their epoch.
- **D1077 — A development-only push egress exception** (one named configuration, e.g. a CIDR list for the demo
  function) is refused at startup unless the development profile is explicit, and the production image test proves
  it absent — the precedent of the development-only exception of feature 017's database runner.
- **D1078 — The priority index is an expression index on `coalesce(priority, 3)` built `CONCURRENTLY`;** a stored
  generated column would rewrite `steps` under an exclusive lock.
- **D1079 — `RETIRED` version tombstones are never purged** while their deployment exists, so a retired build can
  never register again; the 1,000-record bound refuses new versions instead (`QUOTA_EXCEEDED`). *Revised
  2026-10-27:* only versions that were ever routed are tombstoned; a never-routed version is deleted when retired
  (nothing can reference it), and version creation is rate-limited per deployment, so a bound credential cannot
  exhaust the bound permanently.
- **D1610 — Reset rule of the deployment pin is `REMATERIALIZE`** (coordinator decision C1, 018's registry): a reset
  re-creates the pin with the source's version so a pinned process keeps its build; if that version is `RETIRED`
  the new run is a new process and pins at its first claim from the current routing (`source: reset`), which is the
  same choice a `continueAsNew` with `deployment: current` makes. Other 019 events are `COPY`; checkpoints are never
  copied.

## Gaps

- **G1040 — Discovery amplification.** A session of a ramping or draining version filters candidates after the
  `(activity, priority, ready_at)` index scan; a 1 % ramp may read about a hundred candidates per hit. Bounded by
  the scan budget; measured in the benchmark before ceilings are accepted.
- **G1041 — Strict priority can starve lower levels indefinitely.** No aging; `dispatch_wait_seconds{priority}`
  makes it visible.
- **G1042 — No per-key fairness weight overrides.**
- **G1043 — Push endpoints are not deployment versions** (for example Lambda aliases as builds).
- **G1044 — No push to private networks;** it would need a managed executor with its own network boundary.
- **G1045 — Asynchronous completion and push results are inline only (≤256 KiB);** blob outputs are not supported.
- **G1046 — Warm residue the runner cannot enumerate** — genuinely untestable kernel and runtime state shared within
  one container (page cache contents, kernel or runtime vulnerabilities, G98). Every enumerable channel (`/tmp`,
  `/dev/shm`, `/dev/mqueue`, SysV IPC, processes, keyrings) is wiped or verified and in the suite.
- **G1047 — Static provider keys in OpenBao;** workload identity federation for AWS and Google is not built.
- **G1048 — Protocol minor numbers** (three) collide with other 018–032 drafts: at hand-over the drafts of 020, 021,
  025 and 026 in this worktree also propose "1.7" for their own additions (as 014 and 017 collided, ADR-0036). The
  coordinator must allocate one ordered sequence; this feature's text uses `1.N`–`1.N+2` for that reason.
- **G1049 — Fairness keys may still contain personal data** a start-option caller or a literal author chose (they
  are classified `internal`, not inspected); raw keys are shown only to `payload:read-internal` holders.
- **G1050 — Insights lag by up to one monitor round;** exact counts exist only at retirement.
- **G1051 — Unversioned sessions can sit idle** when their deployment routes to a build; surfaced, not refused.
- **G1052 — Lambda `Event` retries are invisible** to the attempt count; duplicates carry the same key.
- **G1053 — Only the latest checkpoint is kept;** there is no checkpoint history.
- **G1054 — Detached attempts do not count against `max_inflight`;** they are bounded by the namespace task quota.
- **G1055 — Repin of more than 1,000 runs** at retirement needs 018 batch operations.
- **G1056 — One platform push signing key for every tenant** (security review M3). Declined for now: the required
  expected-tenant/namespace/endpoint check of every verifier is the control; per-tenant key ids are possible defence in
  depth.
- **G1057 — Digest key rotation changes fairness-key digests;** grouping of in-flight steps by key is disturbed for
  the rotation window (the share is approximate during it).
- **G1058 — Lambda `Event` retries arrive outside the ±300 s verifier window** and are rejected; functions should set
  Lambda's own async retries to zero and rely on the platform's attempt retries (documented in the runbook).
- **G1059 — The warm seccomp profile is a second profile to maintain** next to the engine default, and the different
  resident uid needs an image change; both are release gates of US08.
- **G1060 — A push endpoint stops when its binding principal leaves or loses access** until an administrator saves a
  revision bound to themselves; health and an alert say so.
- **G1610 — Priority direction wording with 022.** 022's D1602 grants permits by "priority descending"; on this
  feature's scale (1 is the highest) that must mean priority value 1 first. This spec states "priority 1 first"; 022's
  text should use the same words to avoid an inversion at implementation.

## Owner questions

- **O1040 — Default versioning behaviour of a run routed through a deployment.** Options: `autoUpgrade` (default;
  safe because the engine does not replay workflow code), `pinned` (safest for incompatible activity changes, but
  drains last longer), or no default (authors must declare). **Recommended: `autoUpgrade`,** with `pinned`
  declared per workflow or step.
- **O1041 — Token-only external completion.** Allow a serverless function or external system without a platform
  identity to complete with the task token alone? **Recommended: no;** require an authenticated principal with
  `task:complete` (D1058). A future opt-in per endpoint would need its own threat review.
- **O1042 — In-process reuse in warm containers.** Allow a mode that keeps the interpreter and module state between
  runs (Windmill-style)? **Recommended: not in this feature;** fresh process per invocation only (ADR-0052).

## Independent review (2026-10-26)

A read-only architecture review of this set (agent `architecture-reviewer`, not a person) reported twelve findings;
all were addressed in the documents before handover: cross-tenant envelope acceptance (D1060), the identity needed
for asynchronous push completion (D1058), stranding of runs pinned to a removed ramp (FR-019-007), checkpoint
ordering across attempts and external holders (D1076), unfenced pattern changes and the deployment/push race
(D1075), the secret scan inside the heartbeat transaction (FR-019-015, two-phase pattern of
`crates/application/src/worker/secrets.rs`), the push lock order and inexact tenant concurrency (D1064), the
`steps` rewrite of a stored generated column (D1078), one minor for several slices (D1069), unenforceable warm quotas
and the loopback demo (D1066, D1077), ambiguous tagged values and a biased bucket (D1042, D1049, D1050), and several
cross-file contradictions (D1048, D1049, D1079, `run.started` journals only the key digest). A human review is still
required.

## Second review (2026-10-27, coordinator-commissioned architecture and security reviews)

Every Blocking and Should-fix finding for 019 was applied; minor findings were applied except where noted.

| Finding | Disposition |
|---|---|
| Arch 019-B1 warm residue channels, fresh-process mechanism, resident uid | Fixed: FR-019-031, SC-019-009, D1065, ADR-0052 decision 3, G1046 narrowed, T019-058 |
| Arch 019-S1 pattern vs routing write skew, `PATTERN_IN_USE` race | Fixed: catalog → deployment order (D1075, plan transactions 2a/4/6a, FR-019-001/003), race test T019-020 |
| Arch 019-S2 / sec M5 token usable on gRPC; binding for machine callers | Fixed: holder check on every lease-token call (FR-019-020), binding required (FR-019-021, D1058), battery T019-038 |
| Arch 019-S3 / sec S2 token oracle, receipt before secret | Fixed: secret first, `step_ended_leases`, principal-keyed receipts (FR-019-023, D1057, plan item 10, contracts) |
| Arch 019-S4 async push completion race | Fixed: push-held async attempts accept external reports (FR-019-021/022, D1058), fault test T019-049 |
| Arch 019-S5 / sec S4 fairness key classification and digest | Fixed: D1050, FR-019-012/014, data model (`content`, level, HMAC) |
| Arch 019-S6 / sec S5 warm scan of checkpoints, verifiable opt-in, eviction trigger | Fixed: FR-019-032, D1066, plan 14a, `script_executions` columns |
| Arch 019-S7 / sec S1 push credential authority, Google audience | Fixed: `boundBy`, derived audience, pinned token endpoint (FR-019-024/028, D1061) |
| Sec M1 resident process takeover | Fixed with B1 (uid, seccomp, baseline check) |
| Sec M2 Lambda region | Fixed: FR-019-024, data model, OpenAPI |
| Sec M3 per-tenant signing keys | Declined (G1056): the required verifier identity check is the control |
| Sec M4 tombstone exhaustion | Fixed: never-routed versions deleted, creation rate limit (D1079, plan ceilings) |
| Sec M6 revision retention vs trigger | Fixed: revisions never pruned, ≤1,000 per endpoint |
| Sec M7 Lambda `Event` retries vs window | Documented (G1058), runbook task T019-061 |
| Sec M8 / 031 S2 fairness `{expr}` and checkpoints vs encoded fields | Fixed on the 019 side: FR-019-012 and FR-019-017 call the FR-031-006 check (031 adds the consumers) |
| Arch minors (fence wording, pins `closed_at`, exact `task_tokens` count, registration fence, US08 wording, verifier deadline, ADR-0013 pointer task, eviction reasons, runner permission) | Fixed: plan, data model, spec, contracts, T019-001 |

## Cross-feature consistency (coordinator decisions, 2026-10-27)

Applied to this feature: C1 (reset rules declared, D1610), C3 (push envelopes are case (b) of ADR-0055's placement
rule), C4 (slot 4 of ADR-0050's lock order), C6 ("Amendment to feature 016" for `heartbeatSchema`, T019-067), C7
(031's encoded-field registry), C8 (020's content-store registry), C11 (032's key inventory), C13 (028's time
classes), C14 (018's `REPIN` batch action), C16 (`PUSH_DESTINATION_DENIED`, `run.deployment_repinned`, snake_case
journal detail keys), C17 (023's reserved `checkpoint` host call), C19 (022 permit order, G1610), C22 (ADR-0052
supersedes ADR-0013 in part). Registry entries are made in the owners' files at implementation (T019-068).
