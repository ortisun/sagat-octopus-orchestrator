# Managed database executor capability

Implementation contract for T017-013..017; executed evidence is in [verification](../verification.md).

- The `database-runner` machine role grants worker execution and the dedicated database capability.
  Human administrators, ordinary workers and sandbox runners cannot claim or resolve a managed
  database step, including through a wildcard binding. Database runners advertise only supported
  managed database activities and still need a current namespace/activity binding.
- Protocol 1.6 `ResolveDatabase` uses the existing task id, run/node, attempt, generation
  and lease token. A managed lease requires the dedicated executor protocol revision. Registration,
  claiming and capability use check current authority; a stale in-memory role does not suffice.
- The managed owner check retains the exact matching session SHARE guard in the owning run
  transaction and samples database time after acquiring it. A session that expires during an
  unchanged-row wait supplies no authority, including for heartbeat renewal. This is validity
  at the post-lock sample, not continuous validity until commit. Binding, dedicated current
  role, lease, namespace and protocol checks remain required; the check does not renew sessions.
- Resolution first opens the run transaction, checks the lease, executor/session ownership,
  connection pin/current status and current publisher/initiator ACLs, and reads provider metadata.
  It closes that transaction before any OpenBao I/O. After reading the value, it opens a new run
  transaction, repeats every fence, compares the selected credential metadata and commits the
  resolution audit before returning anything. Identical resolutions of the same attempt/generation/credential
  share the existing bounded `secret.resolved` audit record; counters measure every call. Cancellation, expiry, revocation or rotation between
  the phases discards the value. No provider call holds a run/catalog/membership lock.
- A successful first resolution pins one immutable OpenBao operation/version for that attempt.
  The pin contains metadata only and preserves output-scanning evidence after logical rotation or
  deletion. A repeated resolution in the same attempt uses that version, still subject to current
  authority. It does not cache plaintext. An expired lease cannot resolve a credential. Cleanup must
  retain provider objects while an active attempt's pin needs them, even if the logical secret has
  rotated past its retained-version window; deletion of logical authority remains immediate.
- The response contains only the selected connection settings, typed operation with expressions
  evaluated into literals by the server, effective finite
  limits, credential bytes/version and lease deadline. It contains no OpenBao routing/token, Sagat
  storage URL or signing key. The runner drops credentials when the attempt ends. Diagnostics and
  telemetry never include SQL, values, credentials or driver/provider messages.
- Managed claim concurrency is bounded per connection, namespace and tenant in the lease
  transaction. The target driver also enforces connect/query deadlines, pool and result ceilings.
  These are internal gates; no user-authored retry or activity name can bypass them.
- The SQL statement executes only in the isolated database runner. Existing worker heartbeat,
  cancellation, report idempotency and fencing remain authoritative. An ambiguous write must use
  `OUTCOME_UNKNOWN` and the existing manual-review transition, with no automatic retry.

## Result acceptance

Managed inline and blob results require the configured retained-credential codec and typed result
validator. Blob bytes are scanned before staging and verified/rescanned before atomic completion.
Default instances without those codecs fail closed. Production engine composition installs both;
the isolated runner emits only the closed `DatabaseFailure` vocabulary described below. Transport
always discards caller-supplied `safe_message`. Current policy tightening cannot turn an uncertain
write into an automatic SQL retry.

PostgreSQL adapter details and current runtime gates: [target adapter](postgres-executor.md).

## Audited result disclosure

`GET /namespaces/{namespaceId}/runs/{runId}/database-results/{nodeId}` requires both
`run:read` and `database:result:read` (Admin by default). It selects only a succeeded
managed database step in that exact tenant/namespace/run, with a retained output and
an admitted connection binding. Ordinary activities and missing/foreign outputs are
opaque 404s, including an ordinary activity whose JSON happens to resemble a database
result. The caller cannot select a digest, object key, arbitrary JSON path or bulk export.

The store selects committed provenance in a short transaction. Inline content is
hashed; external content must match the committed attempt's catalog descriptor and
is completely read through verified EOF, bounded by 4 MiB. Both forms pass the typed
result validator. No OpenBao request is necessary for content already scanned at
completion. A second short transaction rechecks current membership, scope and output
digest, then appends `database.result_read` before disclosure. Every successful read
has its own audit; repeat denials use the existing bounded deduplication policy.
An audit failure returns no result bytes. The authorization decision linearizes at
that final transaction; revocation cannot retract bytes already delivered.

HTTP responses are JSON with `Cache-Control: private, no-store` and `nosniff`.
Reads share finite blob-transfer capacity. A permit remains owned by the response
bytes through Hyper's buffers. An absolute deadline (minimum of 30s, configured
transfer duration and token lifetime) covers preparation, object read, audit and
response delivery. A native-server connection abort handle enforces it independently
of body polling, including a client that keeps making slow progress. Normal buffer
release cancels the timer. Missing native server integration fails closed. A delivery
cutoff can occur after the audit has committed; an audit records authorized disclosure,
not proof that a client received every byte. Generic payload inspection stays structural.

## Runner process configuration (T017-015)

The runner is a separate Rust process with a gRPC worker identity and protocol 1.6 (1.5 is scoped execution identity, D600).
It holds no platform SQL pool, scheduler store, local keyring or OpenBao token.
Each configured namespace has its own registered session. A resolved capability
includes namespace UUID and logical connection name so the runner can verify the
scope and reconstruct the canonical operation without invented references.
The tenant UUID is included for correlated telemetry. Managed task polls contain
`input_json={}` and no input blob reference: the server evaluates parameters under
the capability. Generic `ReadBlob` is refused for managed leases.

An operator-owned bounded policy file declares deployment CIDRs/ports, platform
state exclusions, deployment limits and narrower per-namespace UUID/slug scopes
and limits. Empty scopes deny access. Policies are re-read before each attempt;
removed/changed namespaces cannot inherit another entry. New registered namespaces
require restart. Changes cannot retract credentials or undo an already dispatched
customer operation. The same bounds are enforced independently by deployment
network policy (T017-023); a file alone is not proof of process/network isolation.

`SAGAT_DATABASE_RUNNER_CONFIG` selects a closed JSON document (256 KiB maximum).
`grpcUrl` must use HTTPS; numeric loopback HTTP requires explicit development mode.
`tokenFile` is a regular file of at most 8 KiB, re-read before each RPC for workload
token rotation. `caFile`, if supplied, is at most 256 KiB. Redirects and credential
URLs are not supported. The worker name is `name/namespaceSlug`, at most 120 bytes;
give every live replica a distinct stable name. Up to 32 namespaces are configured.
Namespace UUID and slug must match; duplicate entries are rejected.

`maxConcurrent` is 1..64 per process. `pollIntervalMs` is 100..5000 and `rpcTimeoutMs`
100..10000. Each poll has at most four receive credits; a persistent cursor fairly
rotates namespaces when capacity fills. Deployment and namespace CIDR lists and
port lists each have at most 256 entries. Deployment denies include operator-supplied
platform-state addresses (nonempty outside development); hard reserved/metadata
denials remain in the target adapter. Each of the nine limits is explicitly supplied,
positive, and no greater than the domain ceiling. Effective limits are the component
minimum of server capability, deployment policy and namespace policy. A shared
per-process namespace/connection counter enforces the effective `poolSize` across
attempts through socket cleanup; saturation fails before SQL dispatch. Distributed
server admission intersects persisted connection limits with current operator/tenant/namespace
quota policies. Narrower runner policies independently constrain the delivered capability.

The runner registers only `sagat.database.postgresql` with protocol 1.6. Heartbeats
start before policy/provider I/O; loss, cancellation or lease expiry cancels target
execution. A target statement cannot outlive the capability's returned lease deadline
or its query deadline; renewing the worker lease does not extend that captured SQL
deadline. Each attempt dispatches SQL once. Terminal reports reuse an unchanged key,
content and digest for up to 30 seconds, bounded by the latest valid lease. Retryable
transport failures never trigger another SQL dispatch. Blob uploads use 64 KiB chunks
and are not retried after an ambiguous upload response; unused objects follow upload
TTL cleanup. A write with lost result/upload is `OUTCOME_UNKNOWN`; an ambiguous
completion acknowledgment is handled through receipt replay or lease-expiry recovery.

Managed failures accept only the closed `DatabaseFailure` codes: configuration,
statement/destination denial, connection, TLS, authentication, privileged role,
capacity, timeout, cancellation, SQL failure, unsupported type, result limit,
capability/credential/result unavailability, `OUTCOME_UNKNOWN`, and `WORKER_ERROR`.
They carry no provider/driver message and request no automatic retry. SIGTERM/SIGINT
cancels target work and allows up to 30 seconds for reporting before aborting tasks.
Cancellation is not proof that a dispatched write was rolled back.

The runner exports W3C-correlated OTLP attempt spans, fixed outcome counters and
attempt durations. `/metrics` listens at `SAGAT_DATABASE_RUNNER_METRICS_ADDR`
(default `127.0.0.1:9092`); bind it only on the monitoring network. No SQL, parameter,
credential, output, bearer token or session token is an observability field.
The panic hook emits a fixed incident message without the panic payload.


## Managed report receipt replay guards

An accepted report replays under current dedicated role and binding plus its durable attempt
ownership and request fingerprint. Retain binding authority before acquiring the namespace
Secrets quota/current-membership guards. Replaying that receipt does not require a live session
or repeat provider I/O. No new execution, pin, resolution audit or outbox entry is created.
Live capability and missing-receipt report validation retain their separate lease/session rules.
