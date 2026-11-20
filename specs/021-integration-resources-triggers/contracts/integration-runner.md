# Integration runner and managed stream protocol (draft contract)

Draft for FR-021-008, FR-021-015, FR-021-016, FR-021-021, FR-021-022; revised 2026-10-27 after
the architecture and security reviews. Additive to
`specs/001-platform-foundation/contracts/worker.proto` in the next free minor after 1.6
(provisionally **1.7**, allocated at merge, D1151). Not implemented.

## Roles and claiming

- Machine role `integration-runner`: `worker:execute`, `integration:execute`, `connector:stream`
  (for `MQTT`, `GCP_PUBSUB`, `AZURE_SERVICE_BUS`, `WEBSOCKET` streams only). Registers only the
  reserved activities `sagat.integration.http`, `.graphql`, `.s3`, `.smtp`, `.slack`, and still needs
  a current namespace/activity binding. A wildcard binding of an ordinary worker, a sandbox runner
  or a database runner never matches `sagat.integration.*`.
- `ResolveResource` serves only operations whose pinned type version names the caller's role.
- Machine role `database-runner` gains `connector:stream` for `POSTGRES_CDC` streams only.
- One tenant per runner token: a runner's token and every namespace in its configuration belong to
  one tenant (as for sandbox runners, ADR-0013); a multi-tenant deployment runs one runner
  deployment per tenant.
- A worker below the new minor is never offered these tasks or streams; protocol negotiation never
  replaces the role, binding, lease and session checks (ADR-0036 decision 1).
- `TaskLease.input_json` is `{}` for managed integration tasks; the request is evaluated by the
  server inside `ResolveResource`.

## Additive messages (excerpt)

```proto
// Added in 1.7 (provisional). Integration runner only.
rpc ResolveResource(ResolveSecretsRequest) returns (ResolvedResource);

message ResolvedResource {
  string tenant_id = 1;              // telemetry correlation only
  string namespace_id = 2;
  string resource_name = 3;
  uint32 resource_revision = 4;
  string type_key = 5;               // http_api | graphql | s3_bucket | smtp | slack_bot
  uint32 type_version = 6;
  string settings_json = 7;          // pinned non-secret settings
  repeated ResolvedSlot slots = 8;   // only the slots this operation needs
  string operation = 9;              // e.g. "http.request"
  string request_json = 10;          // server-evaluated literal request (no expressions left)
  string limits_json = 11;           // effective = min(request, namespace, deployment)
  string effect_key = 12;            // stable step idempotency key (or rendered template)
  string role = 13;                  // forward | compensate | reconcile
  string target_effect_key = 14;     // compensate/reconcile: the forward step's key
  repeated string readable_blob_ids = 15; // blobs this operation may read (S3 put, SMTP attachments)
  int64 lease_deadline_epoch_ms = 16;
  string egress_policy_json = 17;    // namespace egress policy snapshot; the runner intersects it
                                     // with its deployment policy file before resolving any name
}
message ResolvedSlot { string name = 1; bytes value = 2; string version = 3; } // value zeroized after use

// Added in 1.7 (provisional). connector:stream holders; the kind decides the role (CDC: database
// runner; MQTT, Pub/Sub, Service Bus, WebSocket: integration runner).
rpc ClaimConnectorStream(ClaimConnectorStreamRequest) returns (ConnectorStreamLease);
rpc RenewConnectorStream(ConnectorStreamFence) returns (ConnectorStreamLease);
rpc AdmitConnectorEvents(AdmitConnectorEventsRequest) returns (AdmitConnectorEventsResponse);
rpc ReleaseConnectorStream(ConnectorStreamFence) returns (StepCommandResponse);

message ConnectorStreamFence { string connector_id = 1; string stream = 2; uint64 generation = 3; string lease_token = 4; }
message ConnectorStreamLease {
  ConnectorStreamFence fence = 1;
  int64 expires_at_epoch_ms = 2;
  string settings_json = 3;          // host, port, database, tls, publication, slot, tables
  bytes credential = 4;              // source credential, two-phase resolved; set only by Claim,
                                     // always empty in a Renew response
  string credential_version = 5;
  int64 checkpoint = 6;              // stored next_offset (CDC LSN; unused by kinds whose broker
                                     // keeps the position)
  string limits_json = 7;
}
message AdmitConnectorEventsRequest {
  ConnectorStreamFence fence = 1;
  repeated ConnectorEvent events = 2;       // ≤ 1,000 and ≤ 4 MiB (hard)
  repeated ConnectorDeadLetter dead_letters = 3;
  int64 expected_checkpoint_lsn = 4;        // CDC only; 0 for other kinds
  int64 next_checkpoint_lsn = 5;            // 0 when the batch ends inside a transaction
  int64 server_wal_end_lsn = 6;             // for lag
  string idempotency_key = 7;               // batch receipt
}
message AdmitConnectorEventsResponse { repeated string outcomes = 1; int64 committed_checkpoint_lsn = 2; bool backlogged = 3; }
```

`ConnectorEvent` carries event id, type, subject, correlation key (with a data-derived flag),
occurred-at, data bytes, optional `event.message` metadata and, for CDC, the commit LSN that the
inbox purge guard compares with the stored checkpoint;
`ConnectorDeadLetter` carries position, closed reason and bounded metadata.

## ResolveResource (two phases)

1. Transaction A: lease token, attempt, generation, session SHARE guard with DB clock sampled
   after acquisition, current role and binding, run's pinned binding for this node and role,
   current resource status, current publisher/initiator `resource:use` (and `resource:effect` for
   effect operations), slot ACLs, provider metadata; evaluate the request expressions over the run
   data and pinned variables; nothing committed. Close.
2. OpenBao reads for each needed slot; no transaction open; bounded deadline.
3. Transaction B: repeat every fence and authority check, compare the selected secret versions,
   insert `step_resource_credentials` rows and the bounded `secret.resolved` audit, commit, return.
   Cancellation, expiry, revocation, rotation or a pause between the phases discards the values and
   returns `CAPABILITY_UNAVAILABLE` or `RESOURCE_UNAVAILABLE`.

A repeated call in the same attempt reuses the pinned versions (still under current authority) and
never caches plaintext. OAuth2 tokens are obtained by the runner after this call and live only in
the attempt.

## Execution and reporting

- Heartbeats start before any network I/O; loss of the lease, cancellation or deadline cancels the
  request. A request is dispatched at most once per attempt; transport retries never redispatch.
- Before reporting, the runner scans the output for every slot value (raw, JSON-escaped, base64,
  base64url, percent-encoded) and fails with `SECRET_IN_OUTPUT` on a match. The server repeats the
  scan for inline output.
- Blob outputs (`s3.getObject`, HTTP blob mode) use `WriteBlob` in 64 KiB chunks under the lease;
  `ReadBlob` is permitted only for `readable_blob_ids`; everything else is refused.
- A refused destination is reported as `RESOURCE_DESTINATION_DENIED` (operations) or ends a stream
  with health `CONNECTOR_DESTINATION_DENIED`, with its closed denial category (never
  the name or address); the server records `egress.denied` in the failure transaction.
- `FailStep` uses the closed failure codes of [error-reasons.md](error-reasons.md); `safe_message`
  from the runner is discarded by the transport (017 rule). An ambiguous effect reports
  `OUTCOME_UNKNOWN`; the server applies the existing reconciliation or manual-review transition.
- Compensation and reconciliation are derived steps (`task_kind` `COMPENSATION`/`RECONCILIATION`)
  of the same reserved activity, resolved with `role` and `target_effect_key`; a reconcile answer
  maps to `{"effect": "applied" | "not_applied" | "unknown"}` exactly as for activities.

## Managed streams

- `ClaimConnectorStream` checks the kind-to-role rule, the binding, connector status `ACTIVE`, the
  publisher's current secret ACL, then resolves the credential in two phases like `ResolveResource`
  and pins its version on the lease row.
- `RenewConnectorStream` (every lease third) and every `AdmitConnectorEvents` recheck status, ACL and
  that the pinned credential version is still the active authorized one. A refusal ends the lease
  (`RESOURCE_UNAVAILABLE`); the runner closes the source connection and discards the credential; a
  later claim starts over. Renewal never resends a credential (security S2, architecture S6).
- The runner opens every source socket through the guarded resolver (resolve once, check every
  address, connect to a checked one) and runs TLS, WebSocket upgrade or MQTT over that stream; a
  client library that cannot take a pre-connected stream is not used (G1125).
- Acknowledgement to the source happens only after a committed `AdmitConnectorEvents` reply.

## AdmitConnectorEvents

One transaction: stream lease row locked and compared (holder, generation, DB-clock expiry);
connector status, secret ACL and pinned credential version rechecked; batch
receipt by `idempotency_key` (same key and content replays, changed content conflicts); inbox rows
(ADR-0012 dedupe per event); dead letters (bounded per connector); checkpoint CAS
`expected_checkpoint_lsn → next_checkpoint_lsn` when non-zero; `lag_bytes` and health. A stale
generation returns `STALE_ATTEMPT` and commits nothing. A full backlog commits nothing and returns
`backlogged = true`; the runner stops reading and retries later. The reply is sent only after the
commit is durable under the cell's replication mode: in a `sync_quorum` cell (feature 032) on the
quorum standby, so no source acknowledgement (CDC flush, PUBACK, acknowledge, complete) precedes it. The runner reports the flush LSN
to the customer server only after a committed response whose `committed_checkpoint_lsn` it
received.

## Runner process configuration

`SAGAT_INTEGRATION_RUNNER_CONFIG`: closed JSON (≤ 256 KiB) with `grpcUrl` (https), `tokenFile`,
optional `caFile`, worker `name`, up to 32 namespaces (UUID + slug), `maxConcurrent` 1..64,
`pollIntervalMs` 100..5000, `rpcTimeoutMs` 100..10000, deployment egress allow/deny lists (≤ 256
each, platform-state denies mandatory outside development) and per-namespace narrower limits.
Metrics on `SAGAT_INTEGRATION_RUNNER_METRICS_ADDR` (default `127.0.0.1:9093`). The image is nonroot,
read-only root filesystem, no capabilities, finite CPU/memory/PIDs; its NetworkPolicy permits DNS,
the control plane gRPC endpoint, approved egress and telemetry only.
