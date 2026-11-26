# OpenTelemetry for the three signals, and OpenBao as the key and secret authority

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

Two operator-facing foundations that release 0.3.0 completes (Owner decision 2026-11-24-1, roadmap row 15):

- **OpenTelemetry.** Traces, metrics and logs of every platform process reach an OpenTelemetry Collector, which the
  operator points at any backend (Grafana, Datadog, Honeycomb, Elastic, a vendor's OTLP endpoint). Today only traces do:
  metrics are a Prometheus page and logs are JSON lines on standard output. After this design the Collector scrapes the
  metrics, the processes send their logs over OTLP with the trace and span of the work they describe, and the
  production Collector configuration ships in the repository with authentication and TLS.
- **OpenBao.** OpenBao becomes the authority for every key the platform holds and for the credentials that need
  external custody. Today one key ring in an environment variable (`SAGAT_SECRETS_KEYS`) seals every stored secret, an
  Ed25519 key in another (`SAGAT_SCRIPT_SIGNING_KEY`) signs scripts, a 32-byte file (`SAGAT_OPENBAO_INTENT_KEY_FILE`)
  authenticates database provisioning receipts with no rotation, and only feature 017's database credentials live in
  OpenBao. After this design, with OpenBao configured, those keys live in OpenBao's Transit engine, never leave it,
  rotate without a restart, and are separated per tenant by derivation; each kind of credential that needs external
  custody has a mount, a token and a policy of its own (017's database credentials, 047's OAuth tokens); and the client
  keys of feature 031 are kept out of the platform's reach by a rule the policies enforce.

The two halves share one design because they share one deployment story (a Collector and an OpenBao beside the
platform, both in the repository's compose files and Helm chart) and one rule: telemetry must never carry what OpenBao
guards. They are built by two lanes that touch different crates (Detailed Implementation Plan).

The decisions are [Architecture decision 0151](../adr/ADR-0151-every-signal-reaches-the-collector-and-the-platform-keeps-one-exporter-per-signal.md)
and [Architecture decision 0152](../adr/ADR-0152-openbao-transit-holds-the-platform-keys-and-each-custody-has-its-own-mount-and-token.md),
accepted with this design under the owner's delegation (Owner decision 2026-11-24-4). The registers are in
`specs/015-opentelemetry-and-openbao/research.md`.

### Terms

- **Collector.** The OpenTelemetry Collector (contrib distribution), a separate process the operator runs.
- **Key authority.** Where the platform's keys live and are used: `local` (the key ring and key files of today) or
  `openbao` (OpenBao Transit). Chosen per deployment by `SAGAT_KEY_AUTHORITY`.
- **Data key.** A random 256-bit AES-GCM key that seals one secret version. Stored only wrapped.
- **Wrapped data key.** A data key encrypted by a Transit key (`vault:v<N>:<base64>`), stored next to the ciphertext.
- **Key encryption key (KEK).** The Transit key that wraps data keys, `sagat-secrets`, derived per tenant.
- **Custody.** Who may read a stored value: the platform's ordinary secret path, the database runner (017), the token
  broker (047), or nobody on the platform (031's client keys).

## The Problem

**Telemetry.** `crates/telemetry/src/otlp.rs` exports spans over OTLP/HTTP with JSON (`SpanSink` for live spans,
`OtlpClient::export` for spans projected from the journal) to the Collector of `infra/observability/otel-collector.yaml`,
whose only pipeline is `traces`. Metrics are rendered as a Prometheus page on each process's admin port
(`Metrics::render`) and scraped by Prometheus directly (`infra/observability/prometheus.yml`). Logs are JSON lines on
standard output (`json_log.rs`); `OtlpClient::export_log` exists but only the CLI uses it (`apps/cli/src/telemetry.rs`).
So an operator whose backend speaks OTLP must run a Prometheus and a log shipper as well, a log line cannot be followed
to its trace in the backend unless the shipper parses `trace_id` out of the line, and the Collector configuration in the
repository is local development only (no TLS, no authentication, one exporter to Jaeger).

**Keys.** `KeyRing` (`crates/infra-secrets/src/lib.rs`) seals secret versions with AES-256-GCM under keys parsed from
`SAGAT_SECRETS_KEYS`; rotating means adding a key to an environment variable on every process, redeploying, re-sealing
"at leisure" with nothing that does it, and removing the old key. The key material sits in the environment of every
control plane and engine, where a process dump or a misconfigured debug endpoint can read it. The script-signing key is
an Ed25519 private key in an environment variable of the control plane, and each sandbox runner is given the matching
public keys by hand (`SAGAT_RUNNER_TRUSTED_KEYS`). The intent key of feature 017 has "no multi-key rolling transition"
(`docs/runbooks/database-recovery.md`). OpenBao is deployed for feature 017 (KV version 2,
`secret/data/sagat/credentials/*`, the policies of `infra/openbao/policies/`, Kubernetes authentication through an
OpenBao Agent sidecar in `infra/helm`), and design 047 adds a second mount for OAuth tokens; nothing ties them to the
platform's own keys.

Temporal leaves secrets to the user; Windmill stores them encrypted in its database under one instance key. A platform
whose keys never leave a hardened key service, rotate with a command and are separated per tenant is a reason to choose
it for regulated work.

## Requirements

### Functional: OpenTelemetry

- **Requirement 015-001** Every platform process (control plane, execution engine, sandbox runner, database runner, which
  also serves lake work under ADR-0071, and the CLI) MUST be able to send its traces and its logs to a Collector over OTLP, configured by the standard
  `OTEL_*` variables, and expose its metrics for the Collector to scrape. A process with no endpoint configured MUST
  behave as today.
- **Requirement 015-002** Logs MUST be exported as OTLP log records through a bounded queue that never blocks the work
  (a full queue drops the record and counts it, as `SpanSink` does for spans), with the same scrubbing as standard
  output, the severity, the target, the structured fields, and the trace id and span id of the span the line was
  written in. Standard output MUST keep its JSON lines, so that a crash before an export leaves the line somewhere.
- **Requirement 015-003** The metrics MUST reach the Collector through its Prometheus receiver scraping each process's
  admin port, with the names, units and closed labels of today, and the shipped Collector configuration MUST export
  them over OTLP. No second metrics pipeline is added in the processes.
- **Requirement 015-004** The repository MUST ship a production Collector configuration (`infra/observability/
  otel-collector.production.yaml`) whose OTLP receiver requires TLS and a bearer token, whose memory limiter answers
  `429`/`503` before the Collector runs out of memory, which drops any attribute whose name matches the sensitive-name
  rule of `crates/telemetry/src/redact.rs` as a last defense, and whose exporters are the operator's choice; and the
  Helm chart MUST be able to point every process at it.
- **Requirement 015-005** Every log record and span carries the resource attributes `service.name`, `service.version`,
  `service.instance.id` and `deployment.environment.name`; none carries a tenant or namespace id as a resource
  attribute. Metrics MUST NOT gain a tenant, namespace, run or sink label (Principle V).
- **Requirement 015-006** The header that authenticates a process to the Collector MUST be readable from a file
  (`OTEL_EXPORTER_OTLP_HEADERS_FILE`, an extension of the standard variables), so that a token never sits in an
  environment variable a process listing shows; with OpenBao it is delivered by the OpenBao Agent sidecar into that file.

### Functional: OpenBao

- **Requirement 015-010** A deployment MUST choose its key authority, `local` (today's) or `openbao`, by
  `SAGAT_KEY_AUTHORITY`. Both are supported in production. Under `openbao` no platform key exists outside OpenBao: the
  processes MUST refuse to start if `SAGAT_SECRETS_KEYS`, `SAGAT_SCRIPT_SIGNING_KEY` or `SAGAT_OPENBAO_INTENT_KEY_FILE`
  is also set, except while the migration of Requirement 015-015 runs.
- **Requirement 015-011** Under `openbao`, every new secret version MUST be sealed by envelope encryption: a fresh data
  key from Transit (`transit/datakey/plaintext/sagat-secrets` with the tenant id as derivation context), AES-256-GCM over
  the value with the associated data of today, the plaintext data key wiped after use, and the wrapped data key stored
  with the ciphertext. Opening MUST unwrap through Transit with the same context; a wrapped key moved to another
  tenant's row MUST fail to unwrap.
- **Requirement 015-012** Rotation under `openbao` MUST need no restart and no redeploy: an operator rotates the Transit
  key (or OpenBao rotates it on its `auto_rotate_period`, 90 days by default), new versions are sealed under the newest
  key version at once, and a bounded pass rewraps the stored wrapped keys with `transit/rewrap` (the plaintext data key
  never leaves OpenBao on that path) and reports how many remain on each older version. Raising
  `min_decryption_version` is the operator's step, refused by the runbook's command while a row remains below it.
- **Requirement 015-013** Under `openbao`, scripts MUST be signed by Transit (`transit/sign/sagat-scripts`, Ed25519) and
  the sandbox runners MUST verify with the public keys of that Transit key, fetched with a read-only token and cached
  with every non-retired version, so that a rotation needs no change to a runner's configuration.
- **Requirement 015-014** Under `openbao`, feature 017's provisioning receipts MUST be authenticated by Transit HMAC
  (`transit/hmac/sagat-intent`), whose key versions make a rolling rotation possible; a receipt names its key version.
- **Requirement 015-015** A deployment MUST be able to move from `local` to `openbao` without downtime: start with both
  configured in `migrating` mode (seal with Transit, open with either), run the re-seal pass until it reports zero rows
  under a local key, then remove the local keys. The pass MUST be resumable, bounded per batch and audited.
- **Requirement 015-016** Each custody MUST have its own mount, its own token and its own policy: `sagat-transit` (the
  three Transit keys; the control plane may `datakey`, `decrypt` within its tenant contexts, `sign`, `hmac`; the
  engine may `decrypt` only; the sandbox runner may read public keys only), `secret/sagat/credentials` (feature 017,
  unchanged), and the OAuth mount of design 047. No platform policy MAY grant any path of a mount that holds feature
  031's client keys, and the bootstrap scripts MUST refuse to install a policy that does.
- **Requirement 015-017** If OpenBao cannot be reached, a secret MUST NOT be sealed or opened by any other means
  (Principle V: authorization and authentication fail closed). Opening MAY use an unwrapped data key held in a bounded
  in-memory cache (at most 4,096 keys, each for at most 300 seconds, wiped on eviction), so that a short outage does not
  stop deliveries of recently used secrets; sealing a new version always calls Transit.
- **Requirement 015-018** Every Transit call MUST be bounded (a 5-second timeout, at most 2 retries with jitter on a
  transport error or `5xx`), counted by `openbao_requests_total{operation, outcome}` and timed by
  `openbao_request_seconds{operation}`, traced without a value, a wrapped key or a token, and refused with a typed error
  (`KEY_AUTHORITY_UNAVAILABLE`, retryable) that never names the key's material.
- **Requirement 015-019** OpenBao stays the development service of `infra/docker/compose.database.yaml` (TLS, pinned by
  digest, loopback-only, with a healthcheck, prepared by `docs/runbooks/openbao.md`); its bootstrap
  (`infra/openbao/bootstrap-development.py`, and `bootstrap-kubernetes.py` for production) MUST enable Transit, create
  the three keys and install the policies of Requirement 015-016.

### Non-functional

- **Non-functional requirement 015-001** With the Collector unreachable, no request, pass or step is slower by more than
  1 millisecond at the 99th percentile than with telemetry off: both queues are non-blocking (measured, Task 015-9).
- **Non-functional requirement 015-002** Opening a secret under `openbao` with a cold cache adds at most one Transit
  round trip; the 99th percentile of a secret delivery with a warm cache stays within 5 percent of `local`, on a stated
  environment (Task 015-21).
- **Non-functional requirement 015-003** The rewrap pass handles at least 1,000 secret versions per second against a
  local OpenBao with batches of 250 (Transit's `batch_input`), on a stated environment (Task 015-21).
- **Non-functional requirement 015-004** No key material, wrapped key, token or plaintext data key appears in a log
  line, span, metric, audit record or error (canary test, Task 015-20).

### Success criteria

- **Success criterion 015-001** On the compose stack with the observability profile, one run's trace, its metrics and
  its log lines are found in the backend, each log line linked to its span by trace id and span id.
- **Success criterion 015-002** A Transit key rotation followed by the rewrap pass and the raise of
  `min_decryption_version` leaves every secret openable, and a value sealed before the rotation opens only through the
  rewrapped key; no process restarted.
- **Success criterion 015-003** A wrapped data key copied into a row of another tenant fails to unwrap, and the failure
  is a typed, audited refusal.
- **Success criterion 015-004** The `local` to `openbao` migration runs under load (a secret resolved every 10
  milliseconds) with zero failed resolutions, and ends with zero rows under a local key.
- **Success criterion 015-005** With OpenBao stopped, deliveries of secrets in the cache succeed for up to 300 seconds,
  every other delivery and every seal fails closed with `KEY_AUTHORITY_UNAVAILABLE`, and service returns when OpenBao
  does, with no manual step.

### User stories

- **User story 015-01** As an operator, I point the platform at my Collector once and see traces, metrics and logs of
  every process in my backend, linked to one another.
- **User story 015-02** As a security officer, I keep every platform key in OpenBao, rotate it on a schedule without a
  deployment, and prove that a tenant's secrets cannot be opened under another tenant's context.
- **User story 015-03** As an operator of an existing install, I move from the environment key ring to OpenBao without
  downtime.

## Goals

- One telemetry path for all three signals, configured once, that never slows the work.
- No platform key in an environment variable or a file when OpenBao is configured; rotation by command.
- One rule for who may read which stored credential, enforced by OpenBao policies, not only by code.

## Non-Goals

- **Running the Collector or the backend for the operator.** The repository ships configurations and charts; the
  operator runs them.
- **OTLP over gRPC or protobuf** in 0.3.0: the processes speak OTLP/HTTP with JSON (Decision 161 of feature 008); the
  Collector translates. A follow-up (Gap 015-001).
- **Moving ordinary secret values into OpenBao's KV engine.** The value stays sealed in PostgreSQL next to its record
  (Owner question 015-004 below); KV holds only the custodies that need it (017, 047).
- **Holding feature 031's client keys.** They belong to the customer (ADR-0066 decision 1); this design only keeps the
  platform out of them.
- **The SDK workers' own telemetry**, which the customer exports from their processes; the platform already propagates
  `traceparent` to them.
- **TLS certificates from OpenBao's PKI engine**: a follow-up (Gap 015-002).

## Open Questions

None for the owner. Technical gaps are registered in `specs/015-opentelemetry-and-openbao/research.md` (Gap 015-001 to
Gap 015-004).

## Owner decisions needed

None open. The questions this design raised are decided under Owner decision 2026-11-24-4, each a delegate row (Owner
decisions 2026-11-26-110 to 2026-11-26-117) and an entry of the research register:

| Question | Decision | Why this and not the others |
|---|---|---|
| O015-001 How metrics reach the Collector | The Collector scrapes the admin ports (Prometheus receiver); no OTLP metrics in the processes | One metrics implementation, already bounded and tested; the Collector converts to OTLP for any backend. In-process OTLP metrics would be a second pipeline with its own cardinality and failure modes for no gain to the operator |
| O015-002 How logs reach the Collector | In-process OTLP logs, standard output kept | The process knows the span of each line; a file shipper must parse it back and depends on the container runtime's paths. Standard output stays the last resort when a process dies |
| O015-003 Is OpenBao required in production | No: `local` and `openbao` are both supported; `openbao` is the recommended authority | Requiring a second stateful service of every installation would cost adoption against Windmill, which needs none; offering it gives regulated customers custody, rotation and audit in a hardened service |
| O015-004 Where secret values live | Sealed in PostgreSQL under a Transit-wrapped data key, as today's record | The value commits with its record and its access rows in one transaction and is backed up with them; one read serves a delivery. KV would make every write a two-system write and every backup a two-system restore |
| O015-005 Tenant separation of keys | Transit key derivation with the tenant id as context | A key copied across tenants fails to open by construction, with one Transit key to rotate instead of one per tenant |
| O015-006 Rotation period | `auto_rotate_period` 90 days for `sagat-secrets` and `sagat-intent`; `sagat-scripts` rotated by the operator | Ninety days is the common compliance interval; a signing key's rotation changes what runners trust and is an operator's act |
| O015-007 Unwrapped-key cache | 4,096 keys, 300 seconds | Covers a short OpenBao restart for the hot secrets; bounded memory (4,096 times 32 bytes is 128 KiB of key material) and a bounded window after a revocation of the platform's token |
| O015-008 The 042 per-tenant HMAC key | Stays in the database as design 042 decides | It is used on the dispatcher's hot path for every event; a Transit call per event would put OpenBao on that path |

## Critical User Journeys (CUJs)

### CUJ 1: The operator connects a backend

*   **Given**: the production Collector configuration with a bearer token, and the Helm values
    `telemetry.otlpEndpoint`, `telemetry.headersSecret`.
*   **When**: the operator installs the chart and starts a run.
*   **Then**: the backend shows the run's trace, the engine's `scheduler_tick_seconds`, and the engine's log lines of
    that tick, each linked to the tick's span; if the Collector is stopped, the platform keeps its latency and counts
    the dropped records.

### CUJ 2: A key rotation on a Tuesday afternoon

*   **Given**: `SAGAT_KEY_AUTHORITY=openbao` and 200,000 secret versions under `sagat-secrets` version 3.
*   **When**: the operator runs `bao write -f sagat-transit/keys/sagat-secrets/rotate` and then
    `control-plane secrets rewrap`.
*   **Then**: new versions are sealed under version 4 at once; the pass rewraps the 200,000 wrapped keys in under four
    minutes, reports zero rows below version 4, and the operator raises `min_decryption_version` to 4; no process
    restarted and no secret delivery failed.

### CUJ 3: OpenBao restarts during a busy hour

*   **Given**: deliveries of the same 500 secrets every second.
*   **When**: OpenBao is unavailable for 40 seconds.
*   **Then**: the deliveries continue from the unwrapped-key cache; a new secret written in that window is refused with
    `KEY_AUTHORITY_UNAVAILABLE` (retryable) and succeeds when the caller retries after OpenBao returns.

## Technical Plan (Architecture)

### 1. Logs over OTLP

`crates/telemetry` gains `LogSink`, the twin of `SpanSink`: `json_log::LogLayer` already builds each record and scrubs
it; when an endpoint is configured the layer also submits the record (severity, target, message, fields, and the
current span's trace id and span id) to a bounded queue (8,192 records) drained by a background task in batches of at
most 512 records or 2 seconds, through `OtlpClient::export_log`. A full queue drops and counts
(`telemetry_dropped_total{signal="logs"}`); a failed batch is abandoned and counted, as live spans are. `OtlpConfig`
reads `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT` (and the common endpoint), `OTEL_LOGS_EXPORTER` (`otlp` or `none`) and a
minimum severity (`SAGAT_OTEL_LOGS_LEVEL`, `info` by default), and `OTEL_EXPORTER_OTLP_HEADERS_FILE`. The CLI keeps
its direct export (`apps/cli/src/telemetry.rs`). No crate outside `crates/telemetry` changes for logs: the four
servers (`apps/control-plane`, `apps/execution-engine`, `apps/sandbox-runner`, `apps/database-runner`) already
initialize logging through `sagat_telemetry::init` in their `main.rs`.

### 2. Metrics through the Collector

No process change. The production configuration adds a `prometheus` receiver with one scrape job per process kind
(Kubernetes pod discovery by the chart's labels, or static targets in compose), and a `metrics` pipeline exporting over
OTLP. Prometheus may still scrape directly; the dashboards and alert rules of `infra/observability` are unchanged. The
autoscalers of the worker pools keep reading Prometheus (`workerPoolAutoscaling.prometheusAddress`).

### 3. The Collector configurations

`otel-collector.yaml` (development) gains `logs` and `metrics` pipelines to the debug exporter and to a local
Prometheus remote-write, and stays without TLS. `otel-collector.production.yaml` is new: OTLP/HTTP receiver on 4318
with TLS from a mounted certificate and the `bearertokenauth` extension reading its token from a file; processors
`memory_limiter` (limit 80 percent of the container's memory, spike 20 percent), `batch`, `resource` (adds
`deployment.environment.name`), and an `attributes` processor that deletes attributes whose names match the sensitive
patterns of `redact.rs` (a list generated from that file by a test, so the two cannot drift); exporters left as
commented examples (`otlphttp` to a vendor, `prometheusremotewrite`, `loki`). A test in
`infra/observability/tests/` runs `otelcol-contrib validate` on both files.

### 4. The key authority port

The domain port `SecretCipher` (`crates/domain/src/secrets.rs`) is synchronous and holds its keys (`seal(value, aad)`,
`open(sealed, aad)`). It cannot call a network service, so it gains two methods that take the key instead of holding it,
`seal_with(value, aad, &DataKey)` and `open_with(sealed, aad, &DataKey)`, implemented once over AES-256-GCM in
`crates/infra-secrets` (`DataKey` is a domain type holding 32 bytes in `Zeroizing`, which the domain may use); the keys
come from a new asynchronous port in `crates/application`, `KeyAuthority`:

```rust
// The application's ports return `impl Future` (as `SecretStore` does), not `async_trait`.
pub trait KeyAuthority: Send + Sync {
    fn new_data_key(&self, tenant: TenantId)                       // plaintext and wrapped
        -> impl Future<Output = Result<(DataKey, WrappedKey), KeyAuthorityError>> + Send;
    fn unwrap(&self, tenant: TenantId, wrapped: &WrappedKey)
        -> impl Future<Output = Result<DataKey, KeyAuthorityError>> + Send;
    fn rewrap(&self, tenant: TenantId, batch: &[WrappedKey])
        -> impl Future<Output = Result<Vec<WrappedKey>, KeyAuthorityError>> + Send;
    fn sign_script(&self, digest: &[u8; 32])
        -> impl Future<Output = Result<ScriptSignature, KeyAuthorityError>> + Send;
    fn hmac_intent(&self, message: &[u8])
        -> impl Future<Output = Result<IntentMac, KeyAuthorityError>> + Send;
}
```

Two adapters in `crates/infra-secrets`: `LocalKeyAuthority` over today's `KeyRing`, script key and intent key (a data
key is then the ring's active key itself, and `wrapped` names its id, so the stored format of `local` does not change),
and `TransitKeyAuthority` over OpenBao's HTTP API with the bounded client of `openbao.rs` (HTTPS in production, no
redirects, no proxies, a 64 KiB response cap). `DataKey` holds its bytes in `Zeroizing` and has no `Debug` of its
content. The secret service (`crates/application/src/secrets.rs`) calls `new_data_key` before a seal and `unwrap`
before an open; the engine's delivery path does the same.

### 5. Storage

A migration adds `secret_versions.wrapped_key text` (nullable; at most 1,024 characters; a check that it starts with
`vault:v`) and widens nothing else: `key_id` (1 to 32 of `[a-z0-9_-]`) holds `bao-<version>` for a Transit-sealed row
(the version of `sagat-secrets` that wrapped its data key) and the ring's id for a local row, so a row says which
authority opens it. An index on `key_id` exists (`secret_versions_key_idx`) and serves the rewrap and re-seal passes.
The classification registry gains `wrapped_key` as `secret` (ADR-0033). The runtime role keeps its grants on the table.

### 6. Rotation, rewrap and migration

`control-plane secrets rewrap` runs a pass in bounded batches of 250 rows selected by `key_id` below the newest version,
each batch one `transit/rewrap` call with `batch_input` and one `UPDATE ... WHERE id = ANY($1) AND key_id = $old` that
writes the new wrapped keys and `key_id`; a row changed in between is skipped and taken by the next batch. It is
resumable (no cursor is needed: the predicate is the cursor), audited (`secret.keys_rewrapped`, one record per run with
the counts), and prints the count per remaining version. `control-plane secrets reseal` does the same for rows under a
local key in `migrating` mode: open with the ring, seal with a new data key, one row per transaction (a value is
plaintext in memory only between the two). `SAGAT_KEY_AUTHORITY=migrating` seals with Transit and opens with both.

### 7. Mounts, tokens and policies

| Mount | Engine | Holds | Control plane | Engine | Sandbox runner | Database runner | Token broker (047) |
|---|---|---|---|---|---|---|---|
| `sagat-transit` | Transit | `sagat-secrets` (derived, AES-256-GCM96), `sagat-scripts` (Ed25519), `sagat-intent` (HMAC) | `datakey`, `decrypt`, `rewrap` on `sagat-secrets`; `sign` on `sagat-scripts`; `hmac` and `verify` on `sagat-intent` | `decrypt` on `sagat-secrets` | read of `sagat-scripts` public keys | none | none |
| `secret/sagat/credentials` | KV version 2 | 017 database credentials | as today (`management.hcl`) | as today (`runtime.hcl`) | none | via the engine's capability (017) | none |
| `sagat-oauth` (047) | KV version 2 | refresh tokens, client secrets | none | none | none | none | create, read, destroy |

Each process kind authenticates with its own Kubernetes service account through the OpenBao Agent sidecar the chart
already has (`infra/helm/templates/openbao-agent.yaml`), with a role per kind and no default policy; in compose, with a
token file per kind written by the bootstrap. The bootstrap scripts (`infra/openbao/bootstrap-*.py`) enable Transit at
`sagat-transit`, create the keys with `exportable=false`, `allow_plaintext_backup=false`, `deletion_allowed=false`, and
install the policies from `infra/openbao/policies/`, refusing any policy that grants a path outside the platform's
mounts (so a mount an operator gives feature 031's key providers, by convention `customer-*`, can never be granted to a
platform token). A test parses every policy file and fails on a grant outside the platform's three mounts.

### 8. Fail closed, with a bounded cache

`TransitKeyAuthority` keeps an LRU of unwrapped data keys keyed by (tenant, wrapped key digest), 4,096 entries, each
evicted 300 seconds after insertion, wiped on eviction. The engine opens a recently delivered secret without a Transit
call while it is cached; after a revocation of the platform's token, a cached key stays usable for at most 300 seconds,
which the runbook states. Nothing else is cached: a seal, a signature and an HMAC always call Transit.

### 9. Telemetry of OpenBao itself

`openbao_requests_total{operation, outcome}` (`operation` in `datakey`, `decrypt`, `rewrap`, `sign`, `hmac`, `verify`,
`keys_read`; `outcome` in `ok`, `refused`, `unavailable`, `timeout`), `openbao_request_seconds{operation}`,
`key_cache_entries`, `key_cache_hits_total{outcome}`, `secret_versions_by_authority{authority}` (sampled by the rewrap
command). An alert rule fires on `unavailable` for 2 minutes. Spans `openbao.<operation>` carry the key name and
version, never a wrapped key, a context or a token.

### 10. Rollout

`local` is the default, so an upgrade changes nothing until the operator acts. The column is additive and nullable; a
binary older than 0.3.0 cannot open a Transit-sealed row, so `openbao` is enabled only when every process runs 0.3.0:
the processes list the capability `key-authority-openbao-v1` in the instance registry (ADR-0082), and `migrating` mode
refuses to start unless the requirement is recorded for both kinds. Rolling back from `openbao` to `local` needs the
reverse re-seal and is a documented procedure, never automatic.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| OpenBao down stops secret deliveries | Fail closed by rule; a bounded cache rides out short outages; an alert; the runbook's high-availability topology for OpenBao (Raft, three nodes) |
| A Transit key is deleted | `deletion_allowed=false` at creation; the policies grant no delete |
| A plaintext data key leaks from memory | `Zeroizing`, no `Debug`, wiped on eviction; process dumps disabled in the chart (`securityContext`) |
| Logs exported over OTLP carry a secret | The same scrubbing as standard output; the Collector's attribute filter as a last defense; the canary test |
| The log queue slows the work | Non-blocking submit, drop and count; Non-functional requirement 015-001 measured |
| A platform token reaches feature 031's keys | Policies refused by the bootstrap and by a test when they grant outside the platform's mounts |
| A mixed fleet cannot open Transit rows | `openbao` gated on the instance registry's capability |
| The rewrap pass races a new write | The predicate on the old `key_id`; a changed row is skipped and retried |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Secret versions stay in PostgreSQL; rewrap and re-seal are idempotent per row, guarded by the old `key_id` |
| II. Modular domain, ports and adapters | Pass | `SecretCipher` stays in the domain; `KeyAuthority` in the application; Transit and the ring as adapters in `crates/infra-secrets`; telemetry stays in `crates/telemetry` |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect of a run; Transit calls are internal and idempotent per row |
| IV. Compatibility is versioned and pinned | Pass with conditions | `local` stays the default; the column is additive; `openbao` gated by the instance registry. Condition: rollback from `openbao` is a documented re-seal (Complexity Tracking) |
| V. Code, tenant boundary and secrets are isolated | Pass | Keys never leave OpenBao; derivation per tenant; a mount, token and policy per custody; fail closed; no tenant label |
| VI. Observability and audit are part of the feature | Pass | All three signals through the Collector; OpenBao calls counted and traced; rewrap and re-seal audited |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | No new message path |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Outage, rotation, cross-tenant unwrap, migration under load and canary tests on a real OpenBao; the Collector configurations validated |
| Delivery Constraints | Pass | PostgreSQL stays the authority of state; OpenBao is a component already deployed for feature 017; targets measured on a stated environment |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Two key authorities | Adoption without a second stateful service, custody for those who need it | Only `openbao`: every install must run OpenBao. Only `local`: no rotation by command, keys in the environment |
| Envelope encryption instead of Transit `encrypt` of the value | One Transit call per seal and per cold open, values up to 16 KiB never sent to OpenBao, the value stays next to its record | Transit `encrypt` of every value: the value crosses the network on every read |
| A bounded unwrapped-key cache | Short OpenBao outages must not stop deliveries | No cache: every delivery is a Transit call and an outage of seconds is an outage of the platform |
| A Collector scrape of metrics instead of OTLP metrics | One metrics implementation | A second pipeline inside every process |
| A migrating mode | Moving without downtime | A stop-the-world re-seal |

## Alternatives Considered

- **All secret values in OpenBao KV.** Rejected (Owner question 015-004): two systems on every write and restore.
- **One Transit key per tenant.** Rotation and policy per tenant; derivation gives the same separation with one key.
- **A cloud KMS instead of OpenBao.** OpenBao is the authority the roadmap names and already deployed; a KMS can sit
  under OpenBao as its seal (auto-unseal), which the runbook describes.
- **A log shipper on standard output** (the Collector's `filelog` receiver). Loses the span of each line unless parsed
  back; depends on the container runtime.
- **Logs as OCSF.** Design 040 keeps OCSF for durable records only.

## Non-Functional Requirements

See Requirements, Non-functional (Non-functional requirements 015-001 to 015-004).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Two lanes that touch no crate in common and can run at the same time.

- **Lane A, telemetry** (`crates/telemetry`, `infra/observability`, the telemetry values of `infra/helm`): Tasks 015-2 to
  015-9.
- **Lane B, keys** (`crates/domain/src/secrets.rs`, `crates/application/src/secrets.rs` and the delivery path,
  `crates/infra-secrets`, `crates/infra-postgres` migrations, `apps/control-plane` commands, `infra/openbao`,
  `infra/docker`, the OpenBao values of `infra/helm`): Tasks 015-10 to 015-21.
- Task 015-22 closes both.

- [ ] T015-1 Write this design, decide its questions and record the delegate rows and the two architecture decisions.
  Refs: Requirement 015-001, Requirement 015-010
- [ ] T015-2 Add `LogSink` and the OTLP log export to `json_log::LogLayer`, with the trace and span ids, the bounded
  queue, drop counting and the severity floor; unit tests for drop, scrub and correlation.
  Refs: Requirement 015-001, Requirement 015-002
- [ ] T015-3 Read `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, `OTEL_LOGS_EXPORTER`, `SAGAT_OTEL_LOGS_LEVEL` and
  `OTEL_EXPORTER_OTLP_HEADERS_FILE` in `OtlpConfig::from_lookup`, with tests of every refusal.
  Refs: Requirement 015-001, Requirement 015-006
- [ ] T015-4 Set the resource attributes of Requirement 015-005 on spans and logs; a test that no resource attribute
  names a tenant or namespace.
  Refs: Requirement 015-005
- [ ] T015-5 Write the production Collector configuration with TLS, bearer authentication, the memory limiter and the
  attribute filter generated from `redact.rs`, and a drift test between the two.
  Refs: Requirement 015-004
- [ ] T015-6 Add the `logs` and `metrics` pipelines and the Prometheus receiver to both configurations; validate both
  with `otelcol-contrib validate` in `infra/observability/tests/`.
  Refs: Requirement 015-003, Requirement 015-004
- [ ] T015-7 Add the chart values (`telemetry.otlpEndpoint`, `telemetry.headersSecret`, the logs settings) to every
  process template, with the NetworkPolicy egress to the Collector.
  Refs: Requirement 015-001, Requirement 015-006
- [ ] T015-8 End to end on the compose observability profile: a run's trace, metrics and logs in the backend, linked
  (Success criterion 015-001); the observability runbook updated.
  Refs: Success criterion 015-001
- [ ] T015-9 Measure Non-functional requirement 015-001 with the Collector stopped, on a stated environment.
  Refs: Non-functional requirement 015-001
- [ ] T015-10 Add the `KeyAuthority` port and the `LocalKeyAuthority` adapter over today's keys, with the secret service
  and the delivery path calling it; the existing secret suites pass unchanged.
  Refs: Requirement 015-010, Requirement 015-011
- [ ] T015-11 Add `TransitKeyAuthority`: data keys with tenant context, unwrap, rewrap, sign, HMAC, public keys; the
  bounded client, timeouts, retries and typed errors; unit tests against a recorded fake.
  Refs: Requirement 015-011, Requirement 015-013, Requirement 015-014, Requirement 015-018
- [ ] T015-12 Write the migration (`wrapped_key`, its check, classification as `secret`) with the migration checklist.
  Refs: Requirement 015-011
- [ ] T015-13 Add the unwrapped-key cache (4,096 entries, 300 seconds, wiped on eviction) and its metrics.
  Refs: Requirement 015-017
- [ ] T015-14 Add `control-plane secrets rewrap` and `control-plane secrets reseal`, resumable and audited, and the
  `migrating` mode.
  Refs: Requirement 015-012, Requirement 015-015
- [ ] T015-15 Sign scripts through Transit and verify in the sandbox runner with fetched public keys, keeping every
  non-retired version; a rotation test with no runner change.
  Refs: Requirement 015-013
- [ ] T015-16 Authenticate feature 017's provisioning receipts with Transit HMAC naming the key version; a rolling
  rotation test.
  Refs: Requirement 015-014
- [ ] T015-17 Extend the bootstrap scripts of `infra/openbao/` with Transit, the three keys and the policies, for the
  development OpenBao of `infra/docker/compose.database.yaml` and for Kubernetes; a policy test that refuses any grant
  outside the platform's mounts.
  Refs: Requirement 015-016, Requirement 015-019
- [ ] T015-18 Add the capability `key-authority-openbao-v1` to the instance registry and the start-up refusals of
  Requirement 015-010.
  Refs: Requirement 015-010
- [ ] T015-19 Integration tests on a real OpenBao: rotation and rewrap with no restart (Success criterion 015-002),
  cross-tenant unwrap refused (Success criterion 015-003), migration under load (Success criterion 015-004), outage with
  the cache (Success criterion 015-005).
  Refs: Success criterion 015-002, Success criterion 015-003, Success criterion 015-004, Success criterion 015-005
- [ ] T015-20 The canary test: marked key material, tokens and wrapped keys never in a log, span, metric, audit record or
  error, with both authorities.
  Refs: Non-functional requirement 015-004
- [ ] T015-21 Measure Non-functional requirements 015-002 and 015-003 on a stated environment and write the dated report.
  Refs: Non-functional requirement 015-002, Non-functional requirement 015-003
- [ ] T015-22 Write the runbooks (`docs/runbooks/observability.md` for the Collector, `docs/runbooks/key-authority.md`
  for OpenBao: install, rotate, rewrap, migrate, roll back, outage), the quickstart of CUJ 2 with the failure scenario
  of CUJ 3, the reference of the new variables, and update `docs/traceability.md`, the roadmap row and the release notes.
  Refs: Requirement 015-012, Requirement 015-015

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the three
checkers, the unit tests of the crates it changes and the suites it touches, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`
where it adds a statement. A test is seen failing for the right reason before the code that passes it.

- Lane A: `cargo test -p sagat-telemetry --locked`; the Collector validation tests; the attribute-filter drift test; the
  end-to-end check of Task 015-8 on the compose observability profile; a hand mutation (send a log record without its
  span id) that the correlation test catches.
- Lane B: `cargo test -p sagat-infra-secrets --locked`; a new target `integration_key_authority` that starts its own
  OpenBao container (rule 16 of `AGENTS.md`); the existing `integration_database_connections` unchanged under both
  authorities; the migration's up, down and privilege audit; a hand mutation (unwrap without the tenant context) that
  the cross-tenant test catches.
