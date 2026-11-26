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
- **OpenBao.** OpenBao becomes the authority for every key the platform holds. Today one key ring in an environment
  variable (`SAGAT_SECRETS_KEYS`) seals the secrets stored in PostgreSQL, an Ed25519 key in another
  (`SAGAT_SCRIPT_SIGNING_KEY`) signs scripts, and a 32-byte file (`SAGAT_OPENBAO_INTENT_KEY_FILE`) binds feature 017's
  provisioning intents with no rotation; feature 017 already keeps some secret values in OpenBao's KV engine. After this
  design, with the `transit` key authority, those three keys live in OpenBao's Transit engine, never leave it, rotate
  without a restart, and the data keys of stored secrets are separated per tenant by derivation; each custody has its
  mount, its policy and its token; and feature 031's customer keys stay outside the platform's OpenBao.

The two halves share one design because they share one deployment story (a Collector and an OpenBao beside the
platform, both in the repository's compose files and Helm chart) and one rule: telemetry must never carry what OpenBao
guards. They are built by two lanes that touch different crates (Detailed Implementation Plan).

The decisions are [Architecture decision 0151](../adr/ADR-0151-every-signal-reaches-the-collector-and-the-platform-keeps-one-exporter-per-signal.md)
and [Architecture decision 0152](../adr/ADR-0152-openbao-transit-holds-the-platform-keys-and-each-custody-has-its-own-mount-and-token.md),
accepted with this design under the owner's delegation (Owner decision 2026-11-24-4). This revision answers the first
security review of 2026-11-26 (`docs/designs/015-opentelemetry-and-openbao.reviews/2026-11-26/round-1/security.md`). The
registers are in `specs/015-opentelemetry-and-openbao/research.md`.

### Terms

- **Collector.** The OpenTelemetry Collector (contrib distribution), a separate process the operator runs.
- **Key authority.** Where the platform's three keys live and are used: `local` (the key ring and key files of today) or
  `transit` (OpenBao Transit). Chosen per deployment by `SAGAT_KEY_AUTHORITY`; `migrating` is the passage between them.
- **Custody of a secret value.** Where one secret's value is kept, recorded per secret in `secrets.authority`: `local`
  (sealed in PostgreSQL under the key ring, today), `openbao` (in OpenBao's KV engine through feature 017's provider
  path, today) and, new, `transit` (sealed in PostgreSQL under a data key that Transit wraps).
- **Data key.** A random 256-bit AES-GCM key that seals one secret version. Stored only wrapped.
- **Wrapped data key.** A data key encrypted by the Transit key `sagat-secrets` (`vault:v<N>:<base64>`), stored with the
  ciphertext, with `N` also in an integer column.
- **Derivation context.** The bytes Transit derives a per-tenant key from: `base64("sagat/secret/v1" || the tenant id's
  16 bytes)`.

## The Problem

**Telemetry.** `crates/telemetry/src/otlp.rs` exports spans over OTLP/HTTP with JSON (`SpanSink` for live spans,
`OtlpClient::export` for spans projected from the journal) to the Collector of `infra/observability/otel-collector.yaml`,
whose only pipeline is `traces`. Metrics are rendered as a Prometheus page on each process's admin port
(`Metrics::render`) and scraped by Prometheus directly (`infra/observability/prometheus.yml`). Logs are JSON lines on
standard output (`json_log.rs`); `OtlpClient::export_log` exists but only the CLI uses it (`apps/cli/src/telemetry.rs`).
So an operator whose backend speaks OTLP must run a Prometheus and a log shipper as well, a log line cannot be followed
to its trace in the backend unless the shipper parses `trace_id` out of the line, and the Collector configuration in the
repository is local development only (no TLS, no authentication, one exporter to Jaeger).

**Keys.** `KeyRing` (`crates/infra-secrets/src/lib.rs`) seals `local` secret versions with AES-256-GCM under keys parsed
from `SAGAT_SECRETS_KEYS`; the associated data binds the tenant, namespace, name, version and key id
(`crates/domain/src/secrets.rs`). Rotating means adding a key to an environment variable on every process, redeploying,
running `control-plane reseal-secrets` and removing the old key. The key material sits in the environment of every
control plane and engine. The script-signing key is an Ed25519 private key in an environment variable of the control
plane; sandbox runners trust public keys given by hand (`SAGAT_RUNNER_TRUSTED_KEYS`) or fetched once from the control
plane (`/script-runner/signing-key`), and the signed message binds the tenant, the digest and the language
(`crates/application/src/script_signing.rs`). The intent key of feature 017 has "no multi-key rolling transition"
(`docs/runbooks/database-recovery.md`). Feature 017 already keeps `openbao`-custody values in KV version 2 through
`ManagedSecretService` (migration `20261122000100_openbao_secrets`), with the policies of `infra/openbao/policies/` and
an OpenBao Agent sidecar in `infra/helm`.

Temporal leaves secrets to the user; Windmill stores them encrypted in its database under one instance key. A platform
whose keys never leave a hardened key service, rotate with a command and are separated per tenant is a reason to choose
it for regulated work.

## Requirements

### Functional: OpenTelemetry

- **Requirement 015-001** Every platform process (control plane, execution engine, sandbox runner, database runner, which
  also serves lake work under ADR-0071, and the CLI) MUST be able to send its traces and its logs to a Collector over
  OTLP, configured by the standard `OTEL_*` variables, and expose its metrics for the Collector to scrape. A process with
  no endpoint configured MUST behave as today.
- **Requirement 015-002** Logs MUST be exported as OTLP log records through a bounded queue that never blocks the work
  (a full queue drops the record and counts it, as `SpanSink` does for spans), with the same scrubbing as standard
  output, the severity, the target, the structured fields, and the trace id and span id of the span the line was
  written in. Standard output MUST keep its JSON lines, so that a crash before an export leaves the line somewhere.
- **Requirement 015-003** The metrics MUST reach the Collector through its Prometheus receiver scraping each process's
  admin port, with the names, units and closed labels of today, and the shipped Collector configuration MUST export
  them over OTLP. No second metrics pipeline is added in the processes.
- **Requirement 015-004** The repository MUST ship a production Collector configuration
  (`infra/observability/otel-collector.production.yaml`) whose OTLP receiver requires TLS and a bearer token, whose
  memory limiter answers `429`/`503` before the Collector runs out of memory, which deletes any attribute whose name
  matches the sensitive-name rule of `crates/telemetry/src/redact.rs` as a last defense for attributes (the body of a
  log record is protected by the in-process scrub only), and whose exporters are the operator's choice; and the Helm
  chart MUST be able to point every process at it.
- **Requirement 015-005** Every log record and span carries the resource attributes `service.name`, `service.version`,
  `service.instance.id` and `deployment.environment.name`; none carries a tenant or namespace id as a resource
  attribute. Metrics MUST NOT gain a tenant, namespace, run or sink label (Principle V).
- **Requirement 015-006** The header that authenticates a process to the Collector MUST be readable from a file
  (`OTEL_EXPORTER_OTLP_HEADERS_FILE`, an extension of the standard variables), read again when it changes or after a
  `401`; it MUST be sent only to an `https` endpoint or a loopback address (a process with a header and a plain `http`
  endpoint elsewhere refuses to start), and marked sensitive in the HTTP client. With OpenBao it is written into that
  file by the OpenBao Agent from the KV path `secret/data/sagat/telemetry/collector` (a row of the custody table,
  Technical Plan 7).
- **Requirement 015-007** The sensitive-name rule and the value shapes of `redact.rs` MUST cover the material this design
  adds: the names `wrapped_key`, `data_key`, `plaintext`, `ciphertext`, `hmac` and the shapes `vault:v<N>:` and OpenBao
  tokens (`hvs.`, `hvb.`, `hvr.`, `s.`).

### Functional: OpenBao

- **Requirement 015-010** A deployment MUST choose its key authority, `local` or `transit`, by `SAGAT_KEY_AUTHORITY`.
  Both are supported in production. Under `transit` no platform key exists outside OpenBao: the processes MUST refuse to
  start if `SAGAT_SECRETS_KEYS`, `SAGAT_SCRIPT_SIGNING_KEY` or `SAGAT_OPENBAO_INTENT_KEY_FILE` is also set, except in
  `migrating` mode (Requirement 015-015); the Helm chart's validation follows.
- **Requirement 015-011** Under `transit`, every new secret version of `transit` custody MUST be sealed by envelope
  encryption: a fresh data key from `transit/datakey/plaintext/sagat-secrets` with the tenant's derivation context,
  AES-256-GCM over the value, the plaintext data key wiped after use, and the wrapped data key and its version stored with
  the ciphertext. The associated data MUST name the custody (`transit`) and never the Transit key's version, so that a
  rewrap leaves it valid. Opening MUST unwrap through Transit with the same context; a wrapped key moved to another
  tenant's row MUST fail to unwrap. Values of `openbao` custody (feature 017) stay in KV, unchanged; values of `local`
  custody open with the ring until re-sealed.
- **Requirement 015-012** Rotation under `transit` MUST need no restart: an operator rotates the Transit key (or OpenBao
  rotates it on its `auto_rotate_period`), new versions are sealed under the newest key version at once, and a bounded,
  resumable pass rewraps stored wrapped keys with `transit/rewrap` (the plaintext data key never leaves OpenBao on that
  path), counting and auditing every row it could not rewrap. Rewrap protects against the exposure of an older key
  version, not against data keys already unwrapped by a stolen token: for that, `control-plane secrets rekey` re-seals
  the rows with new data keys. Raising `min_decryption_version` is done by `control-plane secrets retire-key-versions`,
  which refuses while a row, a failed row or a sealing in flight (counted again after a grace of 60 seconds, longer than
  the longest seal) remains below the target.
- **Requirement 015-013** Under `transit`, scripts MUST be signed by Transit (`transit/sign/sagat-scripts`, Ed25519) over
  the message `script_signing::message(tenant, digest, language)` of today, never over a bare digest. Sandbox runners
  MUST get the public keys and the minimum version from the control plane's authenticated endpoint
  (`/script-runner/signing-key`, extended to list every version at or above the minimum), never from OpenBao, refresh
  them every 5 minutes and when a signature names an unknown version (at most once a minute), and refuse a retired
  version without a restart.
- **Requirement 015-014** Under `transit`, feature 017's provisioning intents MUST be bound by Transit HMAC
  (`transit/hmac/sagat-intent`) over the intent's metadata and the SHA-256 of the value, never the value itself, under a
  binding version 2 that records the key version; version-1 bindings are verified with the file key until no pending
  intent of version 1 remains, after which the file key may be removed.
- **Requirement 015-015** A deployment MUST be able to move from `local` to `transit` without downtime: in `migrating`
  mode new versions are sealed with Transit and both open; `control-plane reseal-secrets --to transit` (the existing
  command, extended) re-seals `local` rows in bounded, resumable, audited batches until it reports zero; then the local
  keys are removed and the mode becomes `transit`.
- **Requirement 015-016** Each custody MUST have its own mount and its own policy, and each process kind its own token
  per custody it uses, delivered by one OpenBao Agent per custody in its pod: `sagat-transit` (the control plane may
  `datakey`, `decrypt`, `rewrap` on `sagat-secrets`, `sign` on `sagat-scripts`, `hmac` and `verify` on `sagat-intent`;
  the execution engine and the secret cleanup job may `decrypt` on `sagat-secrets`), the KV mount of feature 017 (as
  today), the telemetry path of Requirement 015-006, and design 047's OAuth mount (its own token). The policies are an
  exact allow-list of (path, capabilities) per process kind and custody, plus the token self-paths each needs
  (`auth/token/lookup-self`, `renew-self`, `revoke-self`); a test MUST refuse any other grant, any wildcard in a mount
  segment, any `sys/` path and any other `auth/` path, and any `keys/*/config`, `export/`, `backup/` or `restore/` path
  of Transit. The configured mount names are substituted and validated by the bootstrap.
- **Requirement 015-017** If OpenBao cannot be reached, a secret MUST NOT be sealed or opened by any other means
  (Principle V). Opening MAY use an unwrapped data key held in a bounded in-memory cache (Technical Plan 8). Every path
  that opens a secret fails closed when it cannot, including the scan of a step's output for declared secrets
  (`secret_in_output`): a Transit row it cannot open refuses the report as retryable (`KEY_AUTHORITY_UNAVAILABLE`), never
  skips the scan.
- **Requirement 015-018** Every Transit call MUST be bounded (a 2-second timeout per attempt, at most 3 attempts with
  jitter on a transport error or `5xx`, within a total budget of 5 seconds), counted by the existing family
  `secret_provider_requests_total{provider, operation, outcome}` with `provider="transit"`, traced without a value, a
  wrapped key, a context or a token, and refused with a typed error (`KEY_AUTHORITY_UNAVAILABLE`, retryable) that never
  names key material. The client caches its token's `lookup-self` until the token's time to live.
- **Requirement 015-019** OpenBao stays the development service of `infra/docker/compose.database.yaml` (TLS, pinned by
  digest, loopback-only, with a healthcheck, prepared by `docs/runbooks/openbao.md`); its bootstrap
  (`infra/openbao/bootstrap-development.py`, and `bootstrap-kubernetes.py` for production) MUST enable Transit, create the
  three keys and install the policies of Requirement 015-016.
- **Requirement 015-020** Under `transit`, PostgreSQL alone no longer restores a secret, so the runbook and the chart
  MUST provide scheduled, encrypted Raft snapshots of OpenBao with the custody of the unseal and recovery keys, and a
  restore procedure that pairs a PostgreSQL backup with an OpenBao snapshot taken after it (and lowers
  `min_decryption_version` when the database is older than the last rewrap); trimming old key versions
  (`min_available_version`) is refused while a database backup older than the last rewrap is retained.
- **Requirement 015-021** Feature 031's customer keys MUST live outside the platform's OpenBao: in a separate cluster or a
  KMS the customer administers (ADR-0066 decision 1, accepted on the branch of pull request 279). The policy test of
  Requirement 015-016 is defense in depth, not the separation; a NetworkPolicy keeps every platform pod from reaching a
  customer key service.

### Non-functional

- **Non-functional requirement 015-001** With the Collector unreachable, no request, pass or step is slower by more than
  1 millisecond at the 99th percentile than with telemetry off: both queues are non-blocking (measured, Task 015-9).
- **Non-functional requirement 015-002** Opening a secret under `transit` with a cold cache adds one Transit round trip
  (the token's lookup is cached); the 99th percentile of a secret delivery with a warm cache stays within 5 percent of
  `local`, on a stated environment (Task 015-21).
- **Non-functional requirement 015-003** The rewrap pass handles at least 1,000 secret versions per second against a
  local OpenBao with batches of 250 (Transit's `batch_input`), on a stated environment (Task 015-21).
- **Non-functional requirement 015-004** No key material, wrapped key, token, context or plaintext data key appears in a
  log line, span, metric, audit record or error (canary test, Task 015-20).

### Success criteria

- **Success criterion 015-001** On the compose stack with the observability profile, one run's trace, its metrics and
  its log lines are found in the backend, each log line linked to its span by trace id and span id.
- **Success criterion 015-002** A Transit key rotation, the rewrap pass, a recount and the raise of
  `min_decryption_version` leave every secret openable with no process restarted; a row the pass could not rewrap blocks
  the raise; a seal begun before the rotation and committed after it is counted and blocks the raise.
- **Success criterion 015-003** A wrapped data key copied into a row of another tenant fails to unwrap, and the failure
  is a typed, audited refusal.
- **Success criterion 015-004** The `local` to `transit` migration runs under load (a secret resolved every 10
  milliseconds) with zero failed resolutions, and ends with zero rows under a local key.
- **Success criterion 015-005** With OpenBao stopped, deliveries of secrets whose data key was unwrapped less than 300
  seconds before succeed; every other delivery, every seal and every output scan of a Transit row fails closed with
  `KEY_AUTHORITY_UNAVAILABLE`; service returns when OpenBao does, with no manual step.
- **Success criterion 015-006** A script signed for tenant A is refused for tenant B and for another language under
  `transit`; a retired signing-key version is refused by a running sandbox runner within its refresh.
- **Success criterion 015-007** A paired restore (a PostgreSQL backup and a later OpenBao snapshot into fresh services)
  opens every secret.

### User stories

- **User story 015-01** As an operator, I point the platform at my Collector once and see traces, metrics and logs of
  every process in my backend, linked to one another.
- **User story 015-02** As a security officer, I keep every platform key in OpenBao, rotate it on a schedule without a
  deployment, re-key after a token compromise, and prove that a tenant's secrets cannot be opened under another
  tenant's context.
- **User story 015-03** As an operator of an existing install, I move from the environment key ring to OpenBao without
  downtime.

## Goals

- One telemetry path for all three signals, configured once, that never slows the work.
- No platform key in an environment variable or a file under `transit`; rotation and re-keying by command.
- One rule for who may read which stored credential, enforced by OpenBao policies as well as by code.

## Non-Goals

- **Running the Collector or the backend for the operator.** The repository ships configurations and charts.
- **OTLP over gRPC or protobuf** in 0.3.0: the processes speak OTLP/HTTP with JSON (Decision 161 of feature 008); the
  Collector translates. A follow-up (Gap 015-001).
- **Moving `local` or `transit` secret values into KV.** KV stays the custody feature 017 offers for values that need
  it; ordinary values stay sealed in PostgreSQL next to their record (Owner question 015-004 below).
- **Holding feature 031's client keys** (Requirement 015-021).
- **The SDK workers' own telemetry**, which the customer exports from their processes; the platform already propagates
  `traceparent` to them.
- **TLS certificates from OpenBao's PKI engine**: a follow-up (Gap 015-002).
- **Separating custodies against a compromised process.** A process holds the tokens of every custody it uses; the
  separation is between process kinds and between code paths (Technical Plan 7).

## Open Questions

None for the owner. Technical gaps are registered in `specs/015-opentelemetry-and-openbao/research.md` (Gap 015-001 to
Gap 015-005).

## Owner decisions needed

None open. The questions this design raised are decided under Owner decision 2026-11-24-4, each a delegate row (Owner
decisions 2026-11-26-110 to 2026-11-26-117) and an entry of the research register:

| Question | Decision | Why this and not the others |
|---|---|---|
| O015-001 How metrics reach the Collector | The Collector scrapes the admin ports (Prometheus receiver); no OTLP metrics in the processes | One metrics implementation, already bounded and tested; the Collector converts to OTLP for any backend |
| O015-002 How logs reach the Collector | In-process OTLP logs, standard output kept | The process knows the span of each line; a file shipper must parse it back and depends on the container runtime's paths |
| O015-003 Is OpenBao required in production | No: `local` and `transit` are both supported; `transit` is the recommended authority | Requiring a second stateful service of every installation would cost adoption against Windmill, which needs none; offering it gives regulated customers custody, rotation and audit in a hardened service |
| O015-004 Where secret values live | Sealed in PostgreSQL under a Transit-wrapped data key (`transit` custody), beside feature 017's KV custody for the values that need it | The value commits with its record and its access rows in one transaction and is backed up with them; one read serves a delivery. KV for every value would make every write and every restore span two systems |
| O015-005 Tenant separation of keys | Transit key derivation with the tenant's context | A wrapped key copied across tenants fails to open, with one Transit key to rotate instead of one per tenant |
| O015-006 Rotation period | `auto_rotate_period` 90 days for `sagat-secrets` and `sagat-intent`; `sagat-scripts` rotated by the operator | The common compliance interval; a signing key's rotation changes what runners trust and is an operator's act |
| O015-007 Unwrapped-key cache | 4,096 keys, at most 512 per tenant, each for at most 300 seconds since its unwrap, refreshed in the background after 240 seconds while OpenBao answers | Covers a short OpenBao restart for the hot secrets with 128 KiB of key material, keeps one tenant from evicting the others, and bounds the window after a revocation |
| O015-008 The 042 per-tenant HMAC key | Stays in the database as design 042 decides | It is used on the dispatcher's path for every event; a Transit call per event would put OpenBao on that path |

## Critical User Journeys (CUJs)

### CUJ 1: The operator connects a backend

*   **Given**: the production Collector configuration with a bearer token, and the Helm values
    `telemetry.otlpEndpoint` (an `https` URL) and the Agent template that writes the header file.
*   **When**: the operator installs the chart and starts a run.
*   **Then**: the backend shows the run's trace, the engine's `scheduler_tick_seconds`, and the engine's log lines of
    that tick, each linked to the tick's span; if the Collector is stopped, the platform keeps its latency and counts
    the dropped records.

### CUJ 2: A key rotation on a Tuesday afternoon

*   **Given**: `SAGAT_KEY_AUTHORITY=transit` and 200,000 secret versions whose data keys `sagat-secrets` version 3 wraps.
*   **When**: the operator runs `bao write -f sagat-transit/keys/sagat-secrets/rotate`, then
    `control-plane secrets rewrap`, then `control-plane secrets retire-key-versions --below 4`.
*   **Then**: new versions are sealed under version 4 at once; the pass rewraps the 200,000 wrapped keys (at the target
    of Non-functional requirement 015-003, about 200 seconds), reports zero rows and zero failures below version 4; the
    retire command counts again after 60 seconds and raises `min_decryption_version` to 4; every secret still opens; no
    process restarted and no delivery failed.

### CUJ 3: OpenBao restarts during a busy hour

*   **Given**: deliveries of the same 500 secrets every second.
*   **When**: OpenBao is unavailable for 40 seconds.
*   **Then**: the deliveries continue from the cache, whose entries the background refresh kept younger than 240
    seconds; a new secret written in that window is refused with `KEY_AUTHORITY_UNAVAILABLE` (retryable) and succeeds when
    the caller retries after OpenBao returns.

## Technical Plan (Architecture)

### 1. Logs over OTLP

`crates/telemetry` gains `LogSink`, the twin of `SpanSink`: `json_log::LogLayer` already builds each record and scrubs
it; when an endpoint is configured the layer also submits the record (severity, target, message, fields, and the
current span's trace id and span id) to a bounded queue (8,192 records) drained by a background task in batches of at
most 512 records or 2 seconds, through `OtlpClient::export_log`. A full queue drops and counts
(`telemetry_dropped_total{signal="logs"}`); a failed batch is abandoned and counted, as live spans are. `OtlpConfig`
reads `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT` (and the common endpoint), `OTEL_LOGS_EXPORTER` (`otlp` or `none`), a minimum
severity (`SAGAT_OTEL_LOGS_LEVEL`, `info` by default) and `OTEL_EXPORTER_OTLP_HEADERS_FILE`; it refuses a header with a
plain `http` endpoint that is not loopback, and the client marks header values sensitive. The CLI keeps its direct
export (`apps/cli/src/telemetry.rs`). The four servers (`apps/control-plane`, `apps/execution-engine`,
`apps/sandbox-runner`, `apps/database-runner`) already initialize logging through `sagat_telemetry::init`. `redact.rs`
gains the names and shapes of Requirement 015-007 first, so that both the scrub and the Collector filter generated from
it cover them.

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
commented examples (`otlphttp` to a vendor, `prometheusremotewrite`, `loki`). A test in `infra/observability/tests/`
runs `otelcol-contrib validate` on both files.

### 4. The key authority port

The domain port `SecretCipher` (`crates/domain/src/secrets.rs`) is synchronous and holds its keys (`seal(value, aad)`,
`open(sealed, aad)`). It gains two methods that take the key instead of holding it, `seal_with(value, aad, &DataKey)`
and `open_with(sealed, aad, &DataKey)`, implemented once over AES-256-GCM in `crates/infra-secrets` (`DataKey` is a domain
type holding 32 bytes in `Zeroizing`, which the domain may use; it has no `Debug` of its content). The keys come from a
new asynchronous port in `crates/application`:

```rust
// The application's ports return `impl Future` (as `SecretStore` does), not `async_trait`.
pub trait KeyAuthority: Send + Sync {
    fn new_data_key(&self, tenant: TenantId)
        -> impl Future<Output = Result<(DataKey, WrappedKey), KeyAuthorityError>> + Send;
    fn unwrap(&self, tenant: TenantId, wrapped: &WrappedKey)
        -> impl Future<Output = Result<DataKey, KeyAuthorityError>> + Send;
    fn rewrap(&self, batch: &[(TenantId, WrappedKey)])
        -> impl Future<Output = Vec<Result<WrappedKey, KeyAuthorityError>>> + Send;
    fn sign_script(&self, tenant: TenantId, digest: &Digest, language: Language)
        -> impl Future<Output = Result<ScriptSignature, KeyAuthorityError>> + Send;
    fn bind_intent(&self, metadata: &IntentMetadata, value_sha256: &[u8; 32])
        -> impl Future<Output = Result<IntentBinding, KeyAuthorityError>> + Send;
}
```

Two adapters in `crates/infra-secrets`: `LocalKeyAuthority` over today's ring, script key and intent key, and
`TransitKeyAuthority` over OpenBao's HTTP API. The Transit client is configured per mount, caches `lookup-self` until
the token's time to live, applies the timeouts of Requirement 015-018, disables redirects and proxies, caps responses
at 64 KiB, and parses the base64 `plaintext` of a data key into a type that wipes on drop. `StoredValue`
(`crates/application/src/secrets.rs`) gains a third variant, `Transit`, beside `Local` and `OpenBao`; every reader that
matches on it (delivery, webhook signing, the output scan, cleanup) handles it, and none treats a failure to open a
`Transit` row as "skip".

### 5. Storage

A migration widens `secrets.authority` to `('local', 'openbao', 'transit')` and adds to `secret_versions`
`wrapped_key text` (at most 1,024 characters, starting `vault:v`) and `wrapped_key_version integer`; the representation
constraint becomes: `local` rows have a ring key id, a nonce and a ciphertext and no wrapped key; `transit` rows have the
key id `transit`, a nonce, a ciphertext, a wrapped key and its version; `openbao` rows keep feature 017's provider
reference. `KeyRing::parse` refuses the id `transit`. The associated data of a `transit` row names `transit`, never a
version, so a rewrap changes only `wrapped_key` and `wrapped_key_version`. `SecretView.provider` is derived from
`secrets.authority`. An index on `wrapped_key_version` (partial, `transit` rows) serves the rewrap pass. The
classification registry gains `wrapped_key` as `secret` (ADR-0033). For the intents, a migration adds
`secret_provider_operations.binding_version smallint NOT NULL DEFAULT 1` and `binding_key_version integer` (set for
version 2), keeping the 32-byte binding.

### 6. Rotation, rewrap, re-key and migration

- `control-plane secrets rewrap` reads `transit` rows with `wrapped_key_version` below the newest, in batches of 250
  ordered by id with an `after` cursor, rewraps them in one `transit/rewrap` call each batch (one context per item),
  and updates each row `WHERE id = $id AND wrapped_key_version = $old`; a row changed in between is skipped and taken
  by the next run; a row Transit refuses is counted, audited (`secret.keys_rewrapped` with the counts, one record per
  run) and reported. It is resumable from its cursor.
- `control-plane secrets retire-key-versions --below N` counts rows and failed rows below `N`, waits 60 seconds, counts
  again (so a seal begun under an older version has committed or failed), and only then sets `min_decryption_version`.
- `control-plane secrets rekey` re-seals `transit` rows with new data keys, one row per transaction, audited; the
  runbook's procedure after a token compromise is rotate, rekey, retire.
- `control-plane reseal-secrets --to transit` (the existing command, which today selects `key_id <> active`) moves
  `local` rows to `transit` custody in `migrating` mode, one row per transaction; without `--to` it keeps today's
  behaviour for `local` rows and ignores `transit` and `openbao` rows.

### 7. Mounts, tokens and policies

| Custody | Mount and paths | Control plane | Execution engine | Secret cleanup job | Sandbox runner | Database runner | Token broker (047) |
|---|---|---|---|---|---|---|---|
| Platform keys | `sagat-transit`: `datakey`, `decrypt`, `rewrap` on `sagat-secrets`; `sign` on `sagat-scripts`; `hmac`, `verify` on `sagat-intent` | all of these | `decrypt` on `sagat-secrets` | `decrypt` on `sagat-secrets` | none (public keys come from the control plane) | none | none |
| Feature 017 values | the configured KV mount (`secret` by default), `sagat/credentials/*` | as `management.hcl` | as `runtime.hcl` | as `cleanup.hcl` | none | through the engine's capability | none |
| Collector credential | the KV mount, `sagat/telemetry/collector` | read | read | none | read | read | none |
| OAuth tokens (design 047) | `SAGAT_OAUTH_OPENBAO_MOUNT` | none | none | none | none | none | as design 047 decides |

Each process kind authenticates with its own Kubernetes service account. A pod that uses more than one custody runs one
OpenBao Agent sidecar per custody (`infra/helm/templates/openbao-agent.yaml`, extended), each with its role, its policy
and its token file; in compose the bootstrap writes one token file per (process kind, custody). A compromised process
holds every token it uses; the separation is that a code path holds only its custody's token and that another process
kind holds none of them. The bootstrap creates the Transit keys with `exportable=false`, `allow_plaintext_backup=false`
and `deletion_allowed=false`, substitutes and validates the configured mount names, and installs the policies; the
policy test of Requirement 015-016 checks the exact allow-list.

### 8. Fail closed, with a bounded cache

`TransitKeyAuthority` keeps an LRU of unwrapped data keys keyed by (tenant, wrapped key digest): at most 4,096 entries
and 512 per tenant, each dropped 300 seconds after its unwrap and wiped on eviction. While OpenBao answers, a background
task unwraps again every entry older than 240 seconds that was used in the last 300 seconds, so a hot key stays usable
through an outage of up to 60 seconds after its last refresh, and at most 300 seconds after a revocation of the
platform's token. Nothing else is cached: a seal, a signature and an HMAC always call Transit.

### 9. Telemetry of OpenBao itself

`secret_provider_requests_total{provider, operation, outcome}` (the existing family; `provider` gains `transit`;
`operation` in `datakey`, `decrypt`, `rewrap`, `sign`, `hmac`, `verify`; `outcome` in `ok`, `refused`, `unavailable`,
`timeout`), `secret_provider_request_seconds{provider, operation}`, `key_cache_entries`, `key_cache_hits_total{outcome}`,
`secret_versions_by_custody{custody}` (sampled by the rewrap command). An alert rule fires on `unavailable` for 2
minutes. Spans `openbao.<operation>` carry the key name and version, never a wrapped key, a context or a token. Each
process sets `PR_SET_DUMPABLE` to 0 and `RLIMIT_CORE` to 0 at start, so that no core dump holds key material.

### 10. Rollout

`local` is the default, so an upgrade changes nothing until the operator acts. The columns are additive. A binary older
than 0.3.0 cannot open a `transit` row, so `migrating` and `transit` are gated by the instance registry (ADR-0082): every
control plane, execution engine and secret cleanup job of 0.3.0 lists `key-authority-transit-v1`, and `migrating` mode
refuses to start unless the operator recorded that requirement for those kinds; a binary that meets a `transit` row it
cannot open refuses with a typed error. Rolling back from `transit` to `local` is the reverse re-seal, a documented
procedure, never automatic.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| OpenBao down stops secret deliveries | Fail closed by rule; the bounded, refreshed cache rides out short outages; an alert; a three-node Raft cluster in the runbook |
| OpenBao lost | Encrypted Raft snapshots and key custody (Requirement 015-020); the paired restore test |
| A rewrap breaks every secret | The associated data never names a version (Requirement 015-011); a test opens after a rewrap |
| A stolen engine token unwrapped data keys | Rewrap does not help; `secrets rekey` issues new data keys |
| A Transit key is deleted or exported | `deletion_allowed=false`, `exportable=false`; the policy test refuses `keys/*/config`, `export/`, `backup/` |
| A plaintext data key leaks from memory | `Zeroizing`, no `Debug`, wiped on eviction; core dumps disabled at start |
| Logs over OTLP carry a secret | The same scrub, extended to the new names and shapes; the Collector's attribute filter; the canary test |
| A token crosses the network in clear | A header is refused with a non-loopback `http` endpoint |
| A platform token reaches feature 031's keys | They live outside the platform's OpenBao (Requirement 015-021) |
| A mixed fleet cannot open Transit rows | Gated by the instance registry |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Secret versions stay in PostgreSQL; rewrap, re-key and re-seal are idempotent per row, guarded by the old version or key id |
| II. Modular domain, ports and adapters | Pass | `SecretCipher` stays in the domain; `KeyAuthority` in the application; Transit and the ring as adapters in `crates/infra-secrets`; telemetry stays in `crates/telemetry` |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect of a run; Transit calls are internal and idempotent per row |
| IV. Compatibility is versioned and pinned | Pass with conditions | `local` stays the default; the columns are additive; `transit` gated by the instance registry. Condition: rollback from `transit` is a documented re-seal (Complexity Tracking) |
| V. Code, tenant boundary and secrets are isolated | Pass | Keys never leave OpenBao; derivation per tenant; a mount and policy per custody and a token per process kind and custody; fail closed on every read path; the signed script message keeps its tenant; no tenant label |
| VI. Observability and audit are part of the feature | Pass | All three signals through the Collector; OpenBao calls counted and traced; rewrap, re-key and re-seal audited |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | No new message path |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Outage, rotation, cross-tenant unwrap, migration under load, signature binding, paired restore and canary tests on a real OpenBao; the Collector configurations validated |
| Delivery Constraints | Pass | PostgreSQL stays the authority of state; OpenBao is a component already deployed for feature 017, now backed up with it; targets measured on a stated environment |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Two key authorities | Adoption without a second stateful service, custody for those who need it | Only `transit`: every install must run OpenBao. Only `local`: no rotation by command, keys in the environment |
| A third custody of values (`transit`) beside `local` and feature 017's `openbao` | Envelope encryption keeps the value with its record and the key in OpenBao | Moving every value to KV: two systems on every write and restore |
| A bounded, refreshed unwrapped-key cache | Short OpenBao outages must not stop deliveries | No cache: every delivery is a Transit call and an outage of seconds is an outage of the platform |
| One Agent sidecar per custody | A token per (process kind, custody), as design 047 needs for its broker | One token per process kind with several policies: no separation between code paths |
| A Collector scrape of metrics instead of OTLP metrics | One metrics implementation | A second pipeline inside every process |
| A migrating mode | Moving without downtime | A stop-the-world re-seal |

## Alternatives Considered

- **All secret values in OpenBao KV.** Rejected (Owner question 015-004): two systems on every write and restore.
- **One Transit key per tenant.** Rotation and policy per tenant; derivation gives the same separation with one key.
- **A cloud KMS instead of OpenBao.** OpenBao is the authority the roadmap names and already deployed; a KMS can sit
  under OpenBao as its seal (auto-unseal), which the runbook describes.
- **Runners reading the signing keys from OpenBao.** It gives the host of the container engine a token and a network path
  to the key service; the control plane already serves the keys.
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
- **Lane B, keys** (`crates/domain/src/secrets.rs`, `crates/application` secret and script-signing paths,
  `crates/infra-secrets`, `crates/infra-postgres` migrations, `apps/control-plane` commands, `apps/sandbox-runner` trust
  refresh, `infra/openbao`, the OpenBao templates of `infra/helm`): Tasks 015-10 to 015-21 and 015-23 to 015-25.
- Order point: Task 015-2's change to `redact.rs` (lane A) merges before Task 015-20's canary test (lane B). Task 015-22
  closes both.

- [ ] T015-1 Write this design, decide its questions and record the delegate rows and the two architecture decisions.
  Refs: Requirement 015-001, Requirement 015-010
- [ ] T015-2 Extend `redact.rs` with the names and shapes of Requirement 015-007; add `LogSink` and the OTLP log export to
  `json_log::LogLayer`, with the trace and span ids, the bounded queue, drop counting and the severity floor; unit tests
  for drop, scrub and correlation.
  Refs: Requirement 015-001, Requirement 015-002, Requirement 015-007
- [ ] T015-3 Read `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, `OTEL_LOGS_EXPORTER`, `SAGAT_OTEL_LOGS_LEVEL` and
  `OTEL_EXPORTER_OTLP_HEADERS_FILE` in `OtlpConfig::from_lookup` (re-read on change and on `401`); refuse a header with a
  non-loopback `http` endpoint; mark header values sensitive; tests of every refusal.
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
- [ ] T015-7 Add the chart values (`telemetry.otlpEndpoint`, the logs settings, the Agent template for the Collector
  credential) to every process template, with the NetworkPolicy egress to the Collector.
  Refs: Requirement 015-001, Requirement 015-006
- [ ] T015-8 End to end on the compose observability profile: a run's trace, metrics and logs in the backend, linked
  (Success criterion 015-001); the observability runbook updated.
  Refs: Success criterion 015-001
- [ ] T015-9 Measure Non-functional requirement 015-001 with the Collector stopped, on a stated environment.
  Refs: Non-functional requirement 015-001
- [ ] T015-10 Add the `KeyAuthority` port, `seal_with` and `open_with`, and the `LocalKeyAuthority` adapter over today's
  keys, with the secret service, the delivery path, webhook signing and the output scan calling it and `StoredValue`
  gaining `Transit`; the existing secret suites pass unchanged.
  Refs: Requirement 015-010, Requirement 015-011, Requirement 015-017
- [ ] T015-11 Add `TransitKeyAuthority`: data keys with the derivation context (a golden test of its encoding), unwrap,
  per-item rewrap, script signing over the full message, intent binding over metadata and a digest; the client per
  mount with cached lookup, per-attempt timeouts within the budget, wiping types and typed errors; unit tests against a
  recorded fake.
  Refs: Requirement 015-011, Requirement 015-013, Requirement 015-014, Requirement 015-018
- [ ] T015-12 Write the migrations (`secrets.authority` widened; `wrapped_key`, `wrapped_key_version` and the
  representation constraint; the intent binding version and key version; classification as `secret`) with the
  migration checklist; `KeyRing::parse` refuses the id `transit`.
  Refs: Requirement 015-011, Requirement 015-014
- [ ] T015-13 Add the unwrapped-key cache (4,096 entries, 512 per tenant, 300 seconds, background refresh after 240
  seconds, wiped on eviction) and its metrics.
  Refs: Requirement 015-017
- [ ] T015-14 Add `control-plane secrets rewrap`, `secrets retire-key-versions`, `secrets rekey`, and
  `reseal-secrets --to transit`, resumable and audited, and the `migrating` mode.
  Refs: Requirement 015-012, Requirement 015-015
- [ ] T015-15 Sign scripts through Transit; extend `/script-runner/signing-key` to list the versions at or above the
  minimum; the sandbox runner refreshes every 5 minutes and on an unknown version; tests that a signature for another
  tenant or language is refused and that a retired version is refused without a restart (Success criterion 015-006).
  Refs: Requirement 015-013, Success criterion 015-006
- [ ] T015-16 Bind feature 017's intents with Transit HMAC under binding version 2; verify version 1 with the file key
  until none is pending; change the chart's validation and environment; a rolling rotation test.
  Refs: Requirement 015-014
- [ ] T015-17 Extend the bootstrap scripts of `infra/openbao/` with Transit, the three keys and the policies, for the
  development OpenBao of `infra/docker/compose.database.yaml` and for Kubernetes, with the configured mount names
  validated; the exact allow-list policy test of Requirement 015-016.
  Refs: Requirement 015-016, Requirement 015-019
- [ ] T015-18 Add the capability `key-authority-transit-v1` to the instance registry for the control plane, the
  execution engine and the secret cleanup job, and the start-up refusals of Requirement 015-010.
  Refs: Requirement 015-010
- [ ] T015-19 Integration tests on a real OpenBao: rotation, rewrap, recount and retire with no restart (Success
  criterion 015-002), cross-tenant unwrap refused (Success criterion 015-003), migration under load (Success criterion
  015-004), outage with the cache and the output scan (Success criterion 015-005).
  Refs: Success criterion 015-002, Success criterion 015-003, Success criterion 015-004, Success criterion 015-005
- [ ] T015-20 The canary test: marked key material, tokens, contexts and wrapped keys, under ordinary and unlucky field
  names, never in a log, span, metric, audit record or error, with both authorities.
  Refs: Non-functional requirement 015-004
- [ ] T015-21 Measure Non-functional requirements 015-002 and 015-003 on a stated environment and write the dated report.
  Refs: Non-functional requirement 015-002, Non-functional requirement 015-003
- [ ] T015-22 Write the runbooks (`docs/runbooks/observability.md` for the Collector, `docs/runbooks/key-authority.md`
  for OpenBao: install, snapshots and key custody, rotate, rewrap, retire, rekey after a compromise, migrate, roll back,
  outage, paired restore), the quickstart of CUJ 2 with the failure scenario of CUJ 3, the reference of the new
  variables, and update `docs/traceability.md`, the roadmap row and the release notes.
  Refs: Requirement 015-012, Requirement 015-015, Requirement 015-020
- [ ] T015-23 The Helm chart: one OpenBao Agent per custody where a pod needs more than one, roles per (process kind,
  custody), the Raft snapshot schedule, and the NetworkPolicy that keeps platform pods from customer key services.
  Refs: Requirement 015-016, Requirement 015-020, Requirement 015-021
- [ ] T015-24 The paired restore test (Success criterion 015-007) in `integration_key_authority`.
  Refs: Requirement 015-020, Success criterion 015-007
- [ ] T015-25 Disable core dumps at the start of every server process, with a test that reads the process's state.
  Refs: Requirement 015-011

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
  authorities; the migrations' up, down and privilege audit; hand mutations (unwrap without the tenant context; put the
  Transit version into the associated data; sign a bare digest) that the cross-tenant, rewrap and signature tests catch.
