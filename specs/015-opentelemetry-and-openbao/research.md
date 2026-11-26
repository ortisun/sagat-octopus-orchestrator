# Research: OpenTelemetry for the three signals, and OpenBao as the key and secret authority (design 015)

The registers of design 015 (`docs/designs/015-opentelemetry-and-openbao.md`). Append-only.

## Repository facts read on 2026-11-26

- Spans: `crates/telemetry/src/otlp.rs` (`SpanSink`, `OtlpClient::export`, OTLP/HTTP with JSON only, Decision 161 of
  feature 008). Logs: `crates/telemetry/src/json_log.rs` to standard output; `OtlpClient::export_log` used only by
  `apps/cli/src/telemetry.rs`. Metrics: `Metrics::render` on the admin port, scraped by `infra/observability/prometheus.yml`.
- The Collector of `infra/observability/otel-collector.yaml` has one pipeline (`traces`), no TLS, no authentication;
  `infra/docker/compose.observability.yaml` runs `otel/opentelemetry-collector-contrib:0.161.0`.
- Keys: `SAGAT_SECRETS_KEYS` (`crates/infra-secrets/src/lib.rs`, `KeyRing`, AES-256-GCM), `SAGAT_SCRIPT_SIGNING_KEY`
  (Ed25519, `apps/control-plane/src/lib.rs`), `SAGAT_RUNNER_TRUSTED_KEYS` (`apps/sandbox-runner/src/config.rs`),
  `SAGAT_OPENBAO_INTENT_KEY_FILE` (32 bytes, no rolling rotation, `docs/runbooks/database-recovery.md`).
- `secret_versions` (`20261010000200_secrets.up.sql`): `key_id` `^[a-z0-9_-]{1,32}$`, `nonce` 12 bytes, ciphertext 17
  to 16,400 bytes, index on `key_id`.
- OpenBao 2.7.1 (pinned by digest) in `infra/docker/compose.database.yaml`, the Helm chart's OpenBao Agent sidecar, the
  policies of `infra/openbao/policies/` for `secret/data/sagat/credentials/*`.

## Decisions

| Id | Decision | Date |
|---|---|---|
| D015-001 | Logs go over OTLP from the process through a queue of 8,192 records drained in batches of at most 512 records or every 2 seconds; standard output keeps its lines. | 2026-11-26 |
| D015-002 | Metrics reach the Collector by scrape; the processes keep one metrics implementation. | 2026-11-26 |
| D015-003 | `OTEL_EXPORTER_OTLP_HEADERS_FILE` extends the standard variables so a token is read from a file. | 2026-11-26 |
| D015-004 | Two key authorities, `local` and `openbao`, both supported in production; `openbao` recommended. | 2026-11-26 |
| D015-005 | Envelope encryption: a data key per secret version from `transit/datakey/plaintext/sagat-secrets` with the tenant id as context; the wrapped key in `secret_versions.wrapped_key`; `key_id` `bao-<version>`. | 2026-11-26 |
| D015-006 | Rewrap and re-seal passes select by `key_id` and update with the old `key_id` in the predicate, in batches of 250 (Transit `batch_input`). | 2026-11-26 |
| D015-007 | The unwrapped-key cache holds at most 4,096 keys for at most 300 seconds each (4,096 times 32 bytes is 131,072 bytes, 128 KiB, of key material), wiped on eviction. | 2026-11-26 |
| D015-008 | Transit calls: 5-second timeout, at most 2 retries with jitter on a transport error or `5xx`, typed error `KEY_AUTHORITY_UNAVAILABLE` (retryable). | 2026-11-26 |
| D015-009 | Transit keys are created with `exportable=false`, `allow_plaintext_backup=false`, `deletion_allowed=false`; `auto_rotate_period` 90 days for `sagat-secrets` and `sagat-intent`; `sagat-scripts` rotated by the operator. | 2026-11-26 |
| D015-010 | The per-tenant HMAC key of design 042's trigger chains stays in the database, because it is used per event on the dispatcher's path. | 2026-11-26 |

## Gaps

| Id | Gap | State |
|---|---|---|
| G015-001 | OTLP over gRPC or protobuf is not spoken; the Collector translates JSON. | Open; follow-up |
| G015-002 | TLS certificates are not issued by OpenBao's PKI engine. | Open; follow-up |
| G015-003 | A revoked platform token leaves cached data keys usable for at most 300 seconds. | Accepted; stated in the runbook |
| G015-004 | Rolling back from `openbao` to `local` is a manual re-seal procedure. | Accepted; documented |

## Owner questions

All decided on 2026-11-26 under Owner decision 2026-11-24-4 (delegate); the reasoning is in the design's table.

| Id | Question | Decision | State |
|---|---|---|---|
| O015-001 | How metrics reach the Collector | Scrape | Decided, Owner decision 2026-11-26-110 |
| O015-002 | How logs reach the Collector | OTLP from the process, standard output kept | Decided, Owner decision 2026-11-26-111 |
| O015-003 | Is OpenBao required in production | No; two authorities | Decided, Owner decision 2026-11-26-112 |
| O015-004 | Where secret values live | PostgreSQL, under wrapped data keys | Decided, Owner decision 2026-11-26-113 |
| O015-005 | Tenant separation | Transit derivation by tenant id | Decided, Owner decision 2026-11-26-114 |
| O015-006 | Rotation period | 90 days automatic; scripts by the operator | Decided, Owner decision 2026-11-26-115 |
| O015-007 | Unwrapped-key cache | 4,096 keys, 300 seconds | Decided, Owner decision 2026-11-26-116 |
| O015-008 | Design 042's HMAC key | Stays in the database | Decided, Owner decision 2026-11-26-117 |
