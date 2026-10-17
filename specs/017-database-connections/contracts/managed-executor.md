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

Until T017-017 has pin-based credential-field scanning for every result transport, managed
`CompleteStep` and blob writes are refused before storage. Managed failures accept only fixed
`WORKER_ERROR` and `OUTCOME_UNKNOWN` codes; the transport already discards `safe_message`.
This deliberately leaves SQL execution incomplete at the capability checkpoint. The final
feature delivery must replace this gate with the tested scanner before enabling the runner.
