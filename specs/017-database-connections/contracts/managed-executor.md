# Managed database executor capability

Implementation contract for T017-013..017 (pending until the recorded tests pass).

- The `database-runner` machine role grants worker execution and the dedicated database capability.
  Human administrators, ordinary workers and sandbox runners cannot claim or resolve a managed
  database step, including through a wildcard binding. Database runners advertise only supported
  managed database activities and still need a current namespace/activity binding.
- Protocol 1.5 `ResolveDatabase` uses the existing task id, run/node, attempt, generation
  and lease token. A managed lease requires the dedicated executor protocol revision. Registration,
  claiming and capability use check current authority; a stale in-memory role does not suffice.
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

The protocol and credential-pin migration are implemented with real PostgreSQL/OpenBao and gRPC
fence evidence. Driver SQL execution, configurable namespace/deployment ceilings, physical process
isolation, result canaries and provider cleanup remain separate delivery gates (T017-014..025).
This capability contract is not evidence that the SQL executor is already shipped.

### Checkpoint gate

Managed inline and blob results require the configured retained-credential codec and typed result
validator. Blob bytes are scanned before staging and verified/rescanned before atomic completion.
Default instances without those codecs still fail closed. Production composition roots and the
isolated runner remain pending. Managed failures currently accept only fixed `WORKER_ERROR` and
`OUTCOME_UNKNOWN` codes; the transport discards `safe_message`. The runner delivery will add its
closed safe failure vocabulary and integration evidence before enabling SQL execution.

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
