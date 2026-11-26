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
proposed with this design and accepted with its approval, under the owner's delegation (Owner decision 2026-11-24-4). This revision answers the first
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

- **Requirement 015-010** A deployment MUST choose its key authority, `local` or `transit`, and the passage between them,
  `migrating`. The mode is recorded in PostgreSQL (`key_authority_state.mode`, one row), changed only by the commands of
  Technical Plan 6. A process whose configuration disagrees with it follows ADR-0082: it stays up with its gate closed
  and serves nothing but its liveness probe; it never exits. Both `local` and `transit` are supported in production.
  Under `transit` no platform key exists outside OpenBao: a process with `SAGAT_SECRETS_KEYS`,
  `SAGAT_SCRIPT_SIGNING_KEY` or `SAGAT_OPENBAO_INTENT_KEY_FILE` set, or in `migrating` or `transit` mode without its
  Transit configuration, keeps its gate closed; the Helm chart's validation follows.
- **Requirement 015-011** Under `migrating` and `transit`, every new secret version MUST be sealed by envelope
  encryption: a fresh data key from `transit/datakey/plaintext/sagat-secrets` at the recorded sealing version
  (`key_authority_state.seal_version`) with the tenant's derivation context, AES-256-GCM over the value, the plaintext
  data key wiped after use, and the wrapped data key, its version and the time it was issued stored with the
  ciphertext. A `transit` row has no `key_id` and no provider reference, so a binary older than this design reads it as
  corrupt and fails closed. The associated data names the custody with the literal `transit` and the version row's id
  as today, never the Transit key's version, so that a rewrap leaves it valid. Opening MUST unwrap through Transit with
  the same context; a wrapped key moved to another tenant's row MUST fail to unwrap. Values of `openbao` custody (feature
  017) stay in KV, unchanged; `local` rows open with the ring until re-sealed.
- **Requirement 015-012** Rotation MUST need no restart and MUST never outrun the backups: `control-plane secrets rotate`
  rotates `sagat-secrets` in Transit and advances `seal_version` only after it has verified that an OpenBao snapshot (and,
  for a cell with standby regions, the standby's OpenBao) holds the new version; there is no automatic rotation period.
  `control-plane secrets rewrap` rewraps stored wrapped keys with `transit/rewrap` (the plaintext data key never leaves
  OpenBao on that path) to a version that a snapshot holds, guarded per row by the old wrapped key, audited per batch in
  the batch's transaction, counting every row it could not rewrap and, apart, every whole-call outage. Rewrap protects
  against the exposure of an older key version, not against data keys already unwrapped with a stolen token: for that,
  `control-plane secrets rekey --issued-before <time>` re-seals the rows whose data keys were issued before it with new
  data keys, after the stolen tokens were revoked. `control-plane secrets retire-key-versions --below N` first raises
  `key_authority_state.min_seal_version` to `N` in a transaction that locks that row, which a trigger then enforces on
  every insert and every update of `wrapped_key` or `wrapped_key_version` of a `transit` row (seals, re-seals and
  re-keys alike) (a seal at an older version fails and is retried with a fresh data key), then counts
  the rows and failed rows below `N`, and only when both are zero sets Transit's `min_decryption_version` through the
  operator's token.
- **Requirement 015-013** Under `transit`, scripts MUST be signed by Transit (`transit/sign/sagat-scripts`, Ed25519) over
  the message `script_signing::message(tenant, digest, language)` of today (made public to the crate), never over a bare
  digest, at a key version published at least 5 minutes earlier. Sandbox runners MUST get the public keys, the minimum
  version and, until every artifact is re-signed, the legacy local public key from the control plane's authenticated
  endpoint (`/script-runner/signing-key`, extended), never from OpenBao; refresh them every 5 minutes and when a
  signature names an unknown version (at most once a minute; a refusal for an unknown key is retryable, never
  `Untrusted`); record in each cached artifact the key that verified it and evict it at a refresh that no longer trusts
  that key. `control-plane scripts resign` re-signs every artifact under the newest version, guarded per artifact by the
  old key id; retiring a script-key version and leaving `migrating` require it to report zero.
- **Requirement 015-014** Under `transit`, feature 017's provisioning intents MUST be bound by Transit HMAC
  (`transit/hmac/sagat-intent`, `algorithm=sha2-256`, so the binding stays 32 bytes) over the intent's metadata and the
  SHA-256 of the value, never the value itself, under a binding version 2 that records the key version. A replay MUST
  read the stored intent first, outside the provider lock, and verify against the stored version (the file key for
  version 1, `transit/verify` at the stored key version for version 2), never by recomputing at the newest version.
  Version-1 bindings are verified with the file key until no pending intent of version 1 remains. The immutability
  trigger of `secret_provider_operations` covers the two new columns.
- **Requirement 015-015** A deployment MUST be able to move from `local` to `transit` without downtime:
  `control-plane key-authority begin-migration` sets `migrating`, after which a trigger refuses any new `local` row;
  `control-plane reseal-secrets --to transit` (the existing command, extended) re-seals `local` rows in bounded,
  resumable, audited batches; `control-plane key-authority finish-migration` sets `transit` in one transaction that
  checks zero `local` rows, zero pending version-1 intents and zero artifacts signed by the local key.
- **Requirement 015-016** Each custody MUST have its own mount or path prefix and its own policy, and each process kind
  its own token per custody it uses, delivered by one OpenBao Agent per custody in its pod. The custodies and grants are
  the table of Technical Plan 7, plus the token self-paths each needs (`auth/token/lookup-self`, `renew-self`,
  `revoke-self`) and `read` on the Transit keys the control plane uses. Raising `min_decryption_version` runs under a
  separate operator token whose policy allows `update` on `sagat-transit/keys/sagat-secrets/config` with only the
  parameter `min_decryption_version` (`allowed_parameters`), and nothing else. A test MUST refuse any other grant, any
  wildcard in a mount segment, any `sys/` path and any other `auth/` path, and any other `keys/*/config`, `export/`,
  `backup/`, `restore/` or `trim` path of Transit. The configured mount names are substituted and validated by the
  bootstrap.
- **Requirement 015-017** If OpenBao cannot be reached, a secret MUST NOT be sealed or opened by any other means
  (Principle V). `KeyAuthorityError` has three kinds: `Unavailable` (retryable), `Refused` (an integrity failure: a key
  that does not open, a version below the minimum, a corrupt row; not retryable, audited, alerted) and `Denied` (the
  token's policy refused). Every reader of a secret maps them, never as "skip": delivery, the output scan of a step,
  webhook signing, the token broker's credentials and the connector supervisor's credentials (`crates/infra-connectors`).
  A scan that meets `Unavailable` refuses the report as retryable; one that meets `Refused` holds it with a typed,
  non-retryable error that operators see; a step that declares secrets and reaches a replica with no reader configured
  has its report refused as retryable. Opening MAY use an unwrapped data key held in a bounded in-memory cache (Technical
  Plan 8).
- **Requirement 015-018** Every Transit call MUST be bounded (a 2-second timeout per attempt, at most 3 attempts with
  jitter on a transport error or `5xx`, within a total budget of 5 seconds), counted by the existing family
  `secret_provider_requests_total{provider, operation, outcome}` with `provider="transit"` and one outcome set for every
  provider (`ok`, `denied`, `conflict`, `failed`, `unavailable`, `timeout`), traced without a value, a wrapped key, a
  context or a token. The client reads its token file on each call and caches the token's `lookup-self` by a digest of
  the file's content, looking up again after a `403`.
- **Requirement 015-019** OpenBao stays the development service of `infra/docker/compose.database.yaml` (TLS, pinned by
  digest, loopback-only, with a healthcheck, prepared by `docs/runbooks/openbao.md`); its bootstrap
  (`infra/openbao/bootstrap-development.py`, and `bootstrap-kubernetes.py` for production) MUST enable Transit, create the
  three keys and install the policies of Requirement 015-016.
- **Requirement 015-020** Under `transit`, PostgreSQL alone no longer restores a secret, so the runbook and the chart
  MUST provide scheduled, encrypted Raft snapshots of OpenBao with the custody of the unseal and recovery keys, and a
  restore procedure that pairs a PostgreSQL backup with an OpenBao snapshot taken after it (and lowers
  `min_decryption_version` when the database is older than the last rewrap). Old key versions are trimmed
  (`min_available_version`) only by `control-plane secrets trim-key-versions`, the one holder of that capability, which
  refuses below the oldest key version of the oldest database backup the operator declares retained. A restore that
  meets a row it cannot open reports it (`Refused`) and never half-opens silently.
- **Requirement 015-021** Feature 031's customer keys MUST live outside the platform's OpenBao: in a separate cluster or a
  KMS the customer administers (ADR-0066 decision 1, accepted on the branch of pull request 279). The policy test of
  Requirement 015-016 is defense in depth, not the separation; a NetworkPolicy keeps every platform pod from reaching a
  customer key service.
- **Requirement 015-022** A delivery and a step report MUST compare a secret's logical identity (the version row's id,
  its version number and its custody) between their two reads, never its sealed bytes, so that a rewrap, re-key or
  re-seal committed in between does not fail them.

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
- **Success criterion 015-002** A rotation, the rewrap pass and the retirement leave every secret openable with no
  process restarted; a row the pass could not rewrap blocks the retirement; a seal held at a barrier between its data
  key and its commit while the retirement raises the sealing floor is refused at its commit and retried with a fresh key;
  a delivery held between its two reads while a rewrap commits succeeds.
- **Success criterion 015-003** A wrapped data key copied into a row of another tenant fails to unwrap, and the failure
  is a typed, audited refusal.
- **Success criterion 015-004** The `local` to `transit` migration runs under load (a secret resolved every 10
  milliseconds) with zero failed resolutions, and ends with zero rows under a local key.
- **Success criterion 015-005** With OpenBao stopped, deliveries of secrets whose data key is still cached succeed
  (with the platform's clock driven past the 239/241 and 299/301 second boundaries); every other delivery, every seal,
  every output scan of a Transit row, webhook signing and the broker's and connectors' credentials fail closed with
  `Unavailable`; a corrupt row is `Refused`, never retried for ever; service returns when OpenBao does, with no manual
  step. A 0.2.x reader meeting a `transit` row reads it as corrupt and refuses, never scans past it.
- **Success criterion 015-006** A script signed for tenant A is refused for tenant B and for another language under
  `transit`; a retired signing-key version is refused by a running sandbox runner within its refresh.
- **Success criterion 015-007** A paired restore (a PostgreSQL backup and a later OpenBao snapshot into fresh services),
  with a rotation, a rewrap and a retirement between the two, opens every secret; an unpaired restore reports the rows it
  cannot open as `Refused` instead of half-opening silently; a script artifact run before and after a retirement is
  refused after it, from the runner's cache too.

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
- **Automatic rotation.** Rotation is the operator's command, gated on a snapshot (Owner question 015-006).

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
| O015-006 Rotation | By the operator's command only (`secrets rotate`), which advances the sealing version once a snapshot holds the new key version; no automatic rotation period; the runbook recommends every 90 days | An automatic rotation seals under a key version no backup holds yet, so losing OpenBao before the next snapshot would lose every secret sealed since |
| O015-007 Unwrapped-key cache | 4,096 keys of current versions, at most 512 per tenant, each for at most 300 seconds since its unwrap, refreshed every 10 seconds once older than 240 seconds while OpenBao answers | Covers an OpenBao outage of at least 50 seconds for the hot secrets with 128 KiB of key material, keeps one tenant from evicting the others, and bounds the window after a revocation |
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
*   **When**: the operator runs `control-plane secrets rotate` (which waits until the scheduled OpenBao snapshot holds
    version 4), then `control-plane secrets rewrap`, then `control-plane secrets retire-key-versions --below 4`.
*   **Then**: new versions are sealed under version 4 once the snapshot holds it; the pass rewraps the 200,000 wrapped
    keys (at the target of Non-functional requirement 015-003, about 200 seconds) and reports zero rows and zero
    failures below version 4; the retire command raises the sealing floor, counts zero and raises
    `min_decryption_version` to 4; every secret still opens; no process restarted and no delivery failed.

### CUJ 3: OpenBao restarts during a busy hour

*   **Given**: deliveries of the same 500 secrets every second.
*   **When**: OpenBao is unavailable for 40 seconds.
*   **Then**: the deliveries continue from the cache, whose entries the background refresh kept at most 250 seconds old
    (refreshed after 240 seconds, every 10 seconds), so each stays usable for at least 50 seconds; a new secret written in
    that window is refused with `KEY_AUTHORITY_UNAVAILABLE` (retryable) and succeeds when
    the caller retries after OpenBao returns.

## Technical Plan (Architecture)

### 1. Logs over OTLP

`crates/telemetry` gains `LogSink`, the twin of `SpanSink`: `json_log::LogLayer` already builds each record and scrubs
it; when an endpoint is configured the layer also submits the record (severity, target, message, fields, and the
current span's trace id and span id) to a bounded queue (8,192 records) drained by a background task in batches of at
most 512 records or 2 seconds, through `OtlpClient::export_log`. A full queue drops and counts
(`telemetry_dropped_total{signal="logs"}`); a failed batch is abandoned and counted, as live spans are. `OtlpConfig`
reads `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT` (and the common endpoint), `OTEL_LOGS_EXPORTER` (`otlp` or `none`), a minimum
severity (`SAGAT_OTEL_LOGS_LEVEL`, `info` by default) and `OTEL_EXPORTER_OTLP_HEADERS_FILE`; a logs-only configuration
(no traces endpoint) is valid, which `from_lookup` today refuses by returning nothing; it refuses a header with a plain
`http` endpoint that is not loopback, and the client marks header values sensitive. `export_log` sends one record per
request today; a batch encoder sends up to 512 per request. The drop counter follows the spans' existing one:
`otel_logs_total{outcome}` beside `otel_spans_total{outcome}`. The CLI keeps its direct
export (`apps/cli/src/telemetry.rs`). The four servers (`apps/control-plane`, `apps/execution-engine`,
`apps/sandbox-runner`, `apps/database-runner`) already initialize logging through `sagat_telemetry::init`. `redact.rs`
gains the names and shapes of Requirement 015-007 first, so that both the scrub and the Collector filter generated from
it cover them: `scrub_words` splits on `:`, so `vault:v<N>:` is matched with a lookback over the two previous words, as
`follows_scheme` does; the token shapes are bounded (`hvs.`, `hvb.`, `hvr.` followed by at least 24 characters of the
token alphabet; `s.` followed by exactly 24), so ordinary text is not scrubbed. The CLI has no admin port; its metrics
stay the gauge it already exports over OTLP.

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
patterns of `redact.rs` from string attributes only (numeric attributes such as `payload_bytes` keep their value, as in
the process; a list generated from that file by a test, so the two cannot drift); exporters left as
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
    fn new_data_key(&self, tenant: TenantId, version: KeyVersion)
        -> impl Future<Output = Result<(DataKey, WrappedKey), KeyAuthorityError>> + Send;
    fn unwrap(&self, tenant: TenantId, wrapped: &WrappedKey)
        -> impl Future<Output = Result<DataKey, KeyAuthorityError>> + Send;
    fn rewrap(&self, batch: &[(TenantId, WrappedKey)], to: KeyVersion)
        -> impl Future<Output = Result<Vec<Result<WrappedKey, KeyAuthorityError>>, KeyAuthorityError>> + Send;
    fn sign_script(&self, tenant: TenantId, digest: &Digest, language: Language, version: KeyVersion)
        -> impl Future<Output = Result<ScriptSignature, KeyAuthorityError>> + Send;
    fn bind_intent(&self, metadata: &IntentMetadata, value_sha256: &[u8; 32])
        -> impl Future<Output = Result<IntentBinding, KeyAuthorityError>> + Send;
    fn verify_intent(&self, metadata: &IntentMetadata, value_sha256: &[u8; 32], binding: &IntentBinding)
        -> impl Future<Output = Result<bool, KeyAuthorityError>> + Send;
    fn public_keys(&self) -> impl Future<Output = Result<ScriptKeys, KeyAuthorityError>> + Send;
    fn key_info(&self, key: TransitKey) -> impl Future<Output = Result<KeyInfo, KeyAuthorityError>> + Send;
}
```

`rewrap` returns an outer error for a whole-call failure and per-item results otherwise (Transit's `batch_results` are
parsed whatever the HTTP status). Setting `min_decryption_version` and trimming are not on this port: they are commands
of the operator, with the operator's token. Two adapters in `crates/infra-secrets`: `LocalKeyAuthority` over today's
ring, script key and intent key, and `TransitKeyAuthority` over OpenBao's HTTP API. The Transit client is configured per
mount, applies the timeouts of Requirement 015-018, disables redirects and proxies, caps responses at 64 KiB, and parses
the base64 `plaintext` of a data key into a type that wipes on drop. `StoredValue` (`crates/application/src/secrets.rs`)
gains a third variant, `Transit`, beside `Local` and `OpenBao`; every `SecretReader::resolve` caller handles it under
Requirement 015-017: delivery (`crates/application/src/worker/secrets.rs`), the output scan (`secret_in_output`,
`checked_secret_report`), webhook signing and the token broker's credentials (`crates/scheduler/src/deliveries.rs`), and
the connector supervisor (`crates/infra-connectors/src/credentials.rs`). Delivery and the report path compare a
secret's logical identity, not its sealed bytes (Requirement 015-022).

### 5. Storage

- `key_authority_state`: one row (`mode` in `local`, `migrating`, `transit`; `seal_version`, the Transit version new
  data keys are issued at; `min_seal_version`; the time of the last rewrap). Seals and processes read it `FOR SHARE`;
  the commands of section 6 change it `FOR UPDATE`. A trigger on `secret_versions` refuses a `local` row unless the mode
  is `local`, and a `transit` row whose `wrapped_key_version` is below `min_seal_version`.
- `secrets.authority` is widened to `('local', 'openbao', 'transit')` and means the custody of the secret's new
  versions: `put_version` sets it, and it becomes `transit` when the first `transit` version commits. Reading is routed
  by each version row's own columns.
- `secret_versions` gains `wrapped_key text` (at most 1,024 characters, starting `vault:v`), `wrapped_key_version
  integer` and `data_key_issued_at timestamptz`. The representation constraint, on columns only: a `local` row has a
  ring key id, a nonce and a ciphertext and no wrapped key; a `transit` row has no key id, no provider reference, a
  nonce, a ciphertext, a wrapped key, its version and its issue time, and `wrapped_key_version` equals the number in
  `wrapped_key`'s prefix; an `openbao` row keeps feature 017's provider reference. An index on `wrapped_key_version`
  and one on `data_key_issued_at` (partial, `transit` rows) serve the passes. The migration refuses to run when a ring
  key is named `transit`, and `KeyRing::parse` refuses that id afterwards.
- `secret_provider_operations` gains `binding_version smallint NOT NULL DEFAULT 1` and `binding_key_version integer`,
  both covered by its immutability trigger; the binding stays 32 bytes.
- The classification registry gains `wrapped_key` as `secret` (ADR-0033).
- A `transit` → `openbao` move (feature 017's `migrate`) is offered: `transit` joins the sources it accepts.

### 6. Commands

| Command | What it does | Guard |
|---|---|---|
| `key-authority begin-migration` | sets `migrating` | the requirement `key-authority-transit-v1` is recorded for the control plane and the execution engine (instance registry, ADR-0082) |
| `reseal-secrets --to transit` (existing, extended) | re-seals `local` rows, one per transaction, audited, and the sealed columns of design 040's `audit_sinks` | the mode is `migrating`; without `--to` it keeps today's behaviour for `local` rows and those columns |
| `scripts resign` | re-signs every artifact at the newest published script-key version | per artifact, `WHERE key_id = $old` |
| `key-authority finish-migration` | sets `transit` | zero `local` rows, zero pending version-1 intents, zero artifacts signed by the local key, in one transaction |
| `secrets rotate` | rotates `sagat-secrets` in Transit, then advances `seal_version` | an OpenBao snapshot (and a standby, for a cell with standby regions) holds the new version |
| `secrets rewrap` | rewraps rows below the target version, batches of 250 ordered by id, each batch's updates and its audit record in one transaction | per row `WHERE id = $id AND wrapped_key = $old`; the target is a snapshotted version; a restart begins again from the start, which is idempotent |
| `secrets rekey --issued-before T` | re-seals rows whose `data_key_issued_at` is before `T` with new data keys, one per transaction, audited, with an id cursor | `WHERE id = $id AND wrapped_key = $old AND nonce = $old_nonce` |
| `secrets retire-key-versions --below N` | raises `min_seal_version`, counts, then sets Transit's `min_decryption_version` with the operator's token | zero rows and zero failures below `N` after the fence committed |
| `secrets trim-key-versions --below N` | sets `min_available_version` with the operator's token | `N` is not above the oldest key version of the oldest retained backup the operator declares |

The runbook's procedure after a token is stolen: revoke the token's accessor and every token issued to its role (rotate
the Kubernetes service account), then rotate, rekey, rewrap and retire.

### 7. Mounts, tokens and policies

| Custody | Mount or path and grants | Control plane | Execution engine | Database runner | Token broker (047) | Operator |
|---|---|---|---|---|---|---|
| Platform keys | `sagat-transit`: `datakey`, `decrypt`, `rewrap` on `sagat-secrets`; `sign` on `sagat-scripts`; `hmac`, `verify` on `sagat-intent`; `read` on the three keys | all of these | `decrypt` on `sagat-secrets` | none | none | `update` on `keys/sagat-secrets/config` with `allowed_parameters` limited to `min_decryption_version`, and the same for `min_available_version` |
| Feature 017 values | the configured KV mount (`secret` by default), `sagat/credentials/*` | as `management.hcl` | as `runtime.hcl` | through the engine's capability | none | none |
| Collector credential | the KV mount, path prefix `sagat/telemetry/` with a policy of its own | read | read | read | none | none |
| Design 040's operator sink | the KV mount, path prefix `sagat/audit/` with a policy of its own | none | read | none | none | none |
| OAuth tokens (design 047) | `SAGAT_OAUTH_OPENBAO_MOUNT` | none | none | none | as design 047 decides | none |

The secret cleanup job keeps `cleanup.hcl` (no read of a value) and gets no Transit grant: it opens no secret. The
sandbox runner gets no OpenBao token at all; the Helm chart has no sandbox-runner template, and its configuration (the
Collector credential included) is the operator's. Each process kind authenticates with its own Kubernetes service
account; a pod that uses more than one custody runs one OpenBao Agent sidecar per custody
(`infra/helm/templates/openbao-agent.yaml`, extended), each with its role, policy and token file; in compose the
bootstrap writes one token file per (process kind, custody). A compromised process holds every token it uses; the
separation is that a code path reads only its custody's token file and that another process kind holds none of them.
The bootstrap creates the Transit keys with `exportable=false`, `allow_plaintext_backup=false`,
`deletion_allowed=false` and no `auto_rotate_period`, substitutes and validates the configured mount names, and installs
the policies; the policy test of Requirement 015-016 checks the exact allow-list. The Collector's bearer token is
rotated with an overlap: the Collector is given the new token beside the old one, then the processes' token file
changes (they read it again), then the old token is removed.

### 8. Fail closed, with a bounded cache

`TransitKeyAuthority` keeps an LRU of unwrapped data keys keyed by (tenant, wrapped key digest), driven by the
platform's `Clock`: at most 4,096 entries and 512 per tenant, each dropped 300 seconds after its unwrap and wiped on
eviction. Only keys of current versions (the version a delivery opens) enter it; the output scan's opens of older kept
versions use a key held for that scan only. While OpenBao answers, a background task every 10 seconds unwraps again the
entries older than 240 seconds that were used in the last 300 seconds; an entry still cached therefore stays usable for
at least 50 seconds after OpenBao stops answering (300 seconds, less the 240 at which it is refreshed, less the 10 of
the refresh period), and at most 300 seconds after a revocation of the platform's token. Nothing else is cached: a seal,
a signature and an HMAC always call Transit.

### 9. Telemetry of OpenBao itself

`secret_provider_requests_total{provider, operation, outcome}` and `secret_provider_request_seconds{provider, operation}`
(the existing family; `provider` gains `transit`; `operation` in `datakey`, `decrypt`, `rewrap`, `sign`, `hmac`,
`verify`, `read`; `outcome` in the one set of Requirement 015-018), `key_cache_entries`, `key_cache_hits_total{outcome}`,
`secret_versions_by_custody{custody}` (sampled by the rewrap command). An alert rule fires on `unavailable` for 2
minutes, another on any `Refused`. Spans `openbao.<operation>` carry the key name and version, never a wrapped key, a
context or a token. Each server process sets `PR_SET_DUMPABLE` to 0 and `RLIMIT_CORE` to 0 at start, so that no core
dump holds key material.

### 10. Rollout

`local` is the default, so an upgrade changes nothing until the operator acts. The columns and the state row are
additive. A binary older than this design reads a `transit` row as corrupt (it has no key id and no provider reference)
and fails closed on every path, the output scan included (Requirement 015-011), so a stale replica can refuse work but
never accept unscanned output. `begin-migration` is gated by the instance registry as ADR-0082 provides: a control
plane or execution engine lists the capability `key-authority-transit-v1` only when its Transit configuration is
present, the operator records the requirement for both kinds, which the registry refuses while a live process lacks it,
and a process restarted later without the configuration keeps its gate closed (ADR-0082 decisions 3 and 7). The secret
cleanup job opens no secret and is not gated. Rolling back from `transit` to `local` is the reverse re-seal, a
documented procedure, never automatic.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| OpenBao down stops secret deliveries | Fail closed by rule; the bounded, refreshed cache rides out short outages; an alert; a three-node Raft cluster in the runbook |
| OpenBao lost | Encrypted Raft snapshots and key custody (Requirement 015-020); the paired restore test |
| A rewrap breaks every secret | The associated data never names a version (Requirement 015-011); a test opens after a rewrap |
| A rotation outruns the backups | The sealing version advances only once a snapshot holds the new key version (Requirement 015-012) |
| An old replica scans past a Transit row | A `transit` row has no key id, so an older reader refuses it as corrupt |
| A retirement races a seal | The sealing floor is enforced by a trigger before Transit's minimum is raised |
| A stolen engine token unwrapped data keys | Revoke the token first; rewrap does not help; `secrets rekey --issued-before` issues new data keys |
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
  credential) to every process template, with the NetworkPolicy egress to the Collector and ingress from it to the admin
  ports it scrapes.
  Refs: Requirement 015-001, Requirement 015-006
- [ ] T015-8 End to end on the compose observability profile: a run's trace, metrics and logs in the backend, linked
  (Success criterion 015-001); the observability runbook updated.
  Refs: Success criterion 015-001
- [ ] T015-9 Measure Non-functional requirement 015-001 with the Collector stopped, on a stated environment.
  Refs: Non-functional requirement 015-001
- [ ] T015-10 Add the `KeyAuthority` port, `seal_with` and `open_with`, the three error kinds, and the `LocalKeyAuthority`
  adapter over today's keys, with every `SecretReader::resolve` caller of Technical Plan 4 mapping the kinds, `StoredValue`
  gaining `Transit`, and delivery and the report path comparing logical identity; the existing secret suites pass
  unchanged, and a test holds a delivery between its reads while a re-seal commits.
  Refs: Requirement 015-010, Requirement 015-011, Requirement 015-017
- [ ] T015-11 Add `TransitKeyAuthority`: data keys with the derivation context (a golden test of its encoding), unwrap,
  per-item rewrap, script signing over the full message, intent binding over metadata and a digest; the client per
  mount with cached lookup, per-attempt timeouts within the budget, wiping types and typed errors; unit tests against a
  recorded fake.
  Refs: Requirement 015-011, Requirement 015-013, Requirement 015-014, Requirement 015-018
- [ ] T015-12 Write the migrations (`key_authority_state` and its triggers; `secrets.authority` widened;
  `wrapped_key`, `wrapped_key_version`, `data_key_issued_at` and the representation constraint with no key id on a
  `transit` row; the intent binding version and key version under the immutability trigger; classification as `secret`;
  a refusal when a ring key is named `transit`) with the migration checklist; `KeyRing::parse` refuses the id
  `transit`; a test runs the 0.2.x reader against a `transit` row and sees it refuse.
  Refs: Requirement 015-011, Requirement 015-014
- [ ] T015-13 Add the unwrapped-key cache (4,096 entries of current versions, 512 per tenant, 300 seconds, a refresh
  every 10 seconds of entries older than 240 seconds, wiped on eviction, driven by the platform's clock) and its metrics,
  with the boundary tests of Success criterion 015-005.
  Refs: Requirement 015-017
- [ ] T015-14 Add the commands of Technical Plan 6 (`key-authority begin-migration` and `finish-migration`,
  `reseal-secrets --to transit`, `secrets rotate`, `rewrap`, `rekey`, `retire-key-versions`, `trim-key-versions`) with
  their guards, per-batch audit and the operator's token for the two Transit configuration changes.
  Refs: Requirement 015-012, Requirement 015-015
- [ ] T015-15 Sign scripts through Transit at a version published 5 minutes earlier; extend `/script-runner/signing-key`
  to list the versions at or above the minimum and the legacy local key until `scripts resign` reports zero; the sandbox
  runner refreshes every 5 minutes and on an unknown version (a retryable refusal), records the verifying key in its
  artifact cache and evicts on refresh; add `scripts resign`; tests that a signature for another tenant or language is
  refused and that a retired version is refused without a restart, from the cache too (Success criterion 015-006).
  Refs: Requirement 015-013, Success criterion 015-006
- [ ] T015-16 Bind feature 017's intents with Transit HMAC (`sha2-256`) under binding version 2; verify a replay against
  the stored version, outside the provider lock; verify version 1 with the file key until none is pending; change the
  chart's validation and environment; a test replays an intent across a rotation.
  Refs: Requirement 015-014
- [ ] T015-17 Extend the bootstrap scripts of `infra/openbao/` with Transit, the three keys and the policies, for the
  development OpenBao of `infra/docker/compose.database.yaml` and for Kubernetes, with the configured mount names
  validated; the exact allow-list policy test of Requirement 015-016.
  Refs: Requirement 015-016, Requirement 015-019
- [ ] T015-18 Add the capability `key-authority-transit-v1` to the instance registry for the control plane and the
  execution engine, listed only with a Transit configuration, and the closed gate of Requirement 015-010 (ADR-0082).
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
  Refs: Requirement 015-017

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
