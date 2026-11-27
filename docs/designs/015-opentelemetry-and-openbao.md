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
round of 2026-11-26 (`docs/designs/015-opentelemetry-and-openbao.reviews/2026-11-26/round-1/`: security, correctness and
architecture) and the implementation-readiness read of 2026-11-27
(`docs/designs/015-opentelemetry-and-openbao.reviews/2026-11-27/round-1/egm-implementation-readiness.md`, Decision
015-016). The registers are in `specs/015-opentelemetry-and-openbao/research.md`.

### Terms

- **Collector.** The OpenTelemetry Collector (contrib distribution), a separate process the operator runs.
- **Key authority.** Where the platform's three keys live and are used: `local` (the key ring and key files of today) or
  `transit` (OpenBao Transit). It is the mode recorded in `key_authority_state.mode`, changed only by the commands of
  Technical Plan 6; `migrating` is the passage from `local` to `transit` and `returning` the passage back. A process does
  not choose the mode: its configuration only says which keys it can use (Technical Plan 11).
- **Local key.** Any of the three keys of today held by a process: the ring (`SAGAT_SECRETS_KEYS`), the script-signing
  key (`SAGAT_SCRIPT_SIGNING_KEY`) and the intent key file (`SAGAT_OPENBAO_INTENT_KEY_FILE`).
- **Hand-over window.** The time between `finish-migration` and the moment the operator records the requirement
  `key-authority-ring-absent-v1`, during which a process may still hold a local key under `transit` but never uses it
  (Requirement 015-010); and, on the way back, the time between removing that requirement and `begin-return`.
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
from `SAGAT_SECRETS_KEYS`; the associated data binds the tenant, namespace, name, version row id and key id
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
  memory limiter answers `429`/`503` before the Collector runs out of memory, which replaces with `[redacted]` the
  string value of any attribute whose name matches the sensitive-name rule of `crates/telemetry/src/redact.rs` and keeps
  numeric and boolean values, exactly as the process does (`is_sensitive_name`: a size such as `payload_bytes` carries no
  secret), as a last defense for attributes (the body of a log record is protected by the in-process scrub only), with
  one active OTLP exporter whose endpoint and credential the operator supplies (Technical Plan 3); and the Helm chart
  MUST be able to point every process at it.
- **Requirement 015-005** Every log record and span carries the resource attributes `service.name`, `service.version`,
  `service.instance.id` and `deployment.environment.name`, set by the process from the sources of Technical Plan 1
  (the Collector adds none of them, so the development and the production Collector see the same attributes); none
  carries a tenant or namespace id as a resource attribute. Metrics MUST NOT gain a tenant, namespace, run or sink
  label (Principle V).
- **Requirement 015-006** The header that authenticates a process to the Collector MUST be readable from a file
  (`OTEL_EXPORTER_OTLP_HEADERS_FILE`, an extension of the standard variables, in the format, precedence and reload rule
  of Technical Plan 1), read again when its content changes or after a `401`, and marked sensitive in the HTTP client.
  A header read from that file MUST be sent only to an `https` endpoint
  or a loopback address, unless the operator sets `SAGAT_OTEL_ALLOW_PLAINTEXT_HEADERS=true` for a Collector reached
  through a mesh that encrypts the hop; otherwise the configuration is an error and the process exports nothing while
  it logs and counts the refusal (it does not stop serving). `OTEL_EXPORTER_OTLP_HEADERS` keeps today's behaviour, so an
  upgrade stops no process that sends a header to an in-cluster `http` Collector today; with a non-loopback `http`
  endpoint it logs a warning once at start. The file is written by the OpenBao Agent from the KV path
  `secret/data/sagat/telemetry/collector` for the control plane and the execution engine, and is a Kubernetes Secret
  mounted by the chart for the database runner and the sandbox runner, which hold no OpenBao token (Technical Plan 7).
- **Requirement 015-007** The sensitive-name rule and the value shapes of `redact.rs` MUST cover the material this design
  adds: the names `wrapped_key`, `data_key`, `plaintext`, `ciphertext`, `hmac` and the shapes `vault:v<N>:` and OpenBao
  tokens (`hvs.`, `hvb.`, `hvr.`, `s.`). The new names match as parts of a name, as the existing ones do, and this is
  accepted: a textual `data_key_issued_at` or `hmac_algorithm` is hidden too, so the platform logs the issue time as a
  number (Unix seconds) and never logs an algorithm name under such a field.

### Functional: OpenBao

- **Requirement 015-010** A deployment MUST choose its key authority, `local` or `transit`, and the passages between
  them, `migrating` (from `local` to `transit`) and `returning` (back). The mode is recorded in PostgreSQL
  (`key_authority_state.mode`, one row), changed only by the commands of Technical Plan 6 through functions of the
  schema-owning role (Technical Plan 5). Both `local` and `transit` are supported in production. A process whose
  configuration disagrees with the mode follows ADR-0082: it stays up with its gate closed and serves nothing but its
  liveness probe; it never exits. Only the control plane and the execution engine take part in this gate (the kinds of
  the instance registry); the secret cleanup job opens no secret, and the sandbox runners are gated as Requirement
  015-013 states.

  *When the check runs and how a process learns the mode.* A process reads `key_authority_state.mode` with a plain
  `SELECT` at registration and after every successful registry heartbeat (ADR-0082 decision 4: every 10 seconds,
  jittered to 12, on the registry's own connection), and compares its configuration with it each time. A process
  therefore learns a mode change within one heartbeat. What it believes the mode is only picks its code path: the
  triggers of Technical Plan 5 enforce the mode on every write under the state row's lock, so a seal begun under a mode
  that has just changed is refused at its commit, and the writer reads the mode again and retries once (Technical
  Plan 5).

  *The configuration each mode needs.* The rules are chosen so that no passage closes a serving process's gate:

  | Mode | A process keeps its gate closed when | A process stays open, and only warns, when |
  |---|---|---|
  | `local` | the ring is missing while the secret routes are enabled (today's rule) | it also has a Transit configuration: it never seals with Transit (the trigger refuses a new `transit` row) |
  | `migrating` | it lacks the ring or its Transit configuration | never |
  | `transit` | it lacks its Transit configuration; or it holds a local key and the requirement `key-authority-ring-absent-v1` is recorded | it holds a local key and that requirement is not recorded (the hand-over window): it never seals, signs or binds with the local key, logs `key_authority.local_key_configured` once per mode change and sets the gauge `key_authority_local_key_configured{key}` to 1 |
  | `returning` | it lacks the ring or its Transit configuration | never |

  A process lists the capability `key-authority-ring-absent-v1` only when none of the three local keys is configured
  (ADR-0082 decision 7). The operator records that requirement after the rollout that removes the local keys (Technical
  Plan 10), and from then on the registry itself refuses a process started with a local key (ADR-0082 decision 3), which
  is what "under `transit` no platform key exists outside OpenBao" means once the hand-over window is over. The way
  back is symmetric: `begin-return` requires that `key-authority-ring-absent-v1` is not recorded and that
  `key-authority-return-v1` is; after `finish-return` a process may keep its Transit configuration under `local` until
  the operator removes the Transit requirements (Technical Plan 10). The Helm chart's validation follows (Technical
  Plan 11).
- **Requirement 015-011** Under `migrating` and `transit`, every new secret version MUST be sealed by envelope
  encryption: a fresh data key from `transit/datakey/plaintext/sagat-secrets` at the recorded sealing version
  (`key_authority_state.seal_version`) with the tenant's derivation context, AES-256-GCM over the value, the plaintext
  data key wiped after use, and the wrapped data key, its version and the time it was issued stored with the
  ciphertext. A `transit` row has no `key_id` and no provider reference, so a binary older than this design reads it as
  corrupt and fails closed. Which custody a write takes, in each mode:

  | Mode | A write with `requiredProvider: openbao`, or to a secret whose custody is `openbao` | Any other write |
  |---|---|---|
  | `local`, feature 017's provider configured | KV, as today (`requestId` and `expectedVersion` required) | KV, as today |
  | `local`, no provider | refused with `503`, as today | `local`, as today |
  | `migrating`, `transit`, feature 017's provider configured | KV (`requestId` and `expectedVersion` required) | `transit`: `expectedVersion`, when sent, is checked in the write's transaction (`409` on a mismatch), and `requestId`, when sent, is accepted and not stored, since the write is one PostgreSQL transaction whose retry `expectedVersion` detects |
  | `migrating`, `transit`, no provider | refused with `503` (`UNAVAILABLE`), nothing written, as in `local` (`crates/api-http/src/routes/governance.rs`) | `transit`, with the field rules above |
  | `returning`, provider configured | KV (`requestId` and `expectedVersion` required) | `local`, with the same field rules as `transit` |
  | `returning`, no provider | refused with `503`, nothing written | `local`, with the same field rules as `transit` |

  Feature 017's database connection forms send `requiredProvider: openbao` (the contract requires it), so database
  credentials stay in KV. The Studio's general Secrets form sends it today on every write as well
  (`frontend/apps/studio/src/features/secrets/api.ts`) and refuses any answer whose `provider` is not `openbao`
  (`SecretForm.tsx`), which would keep every value written from the Studio out of `transit` custody: Task 015-28 makes
  that form send `requiredProvider: openbao` only when the person chooses "Keep in OpenBao KV" (offered only when the
  provider is configured, and preselected for a secret whose custody is already `openbao`), accept `local`, `transit`
  and `openbao` in the answer and show the custody, while the connection forms keep sending it. A test covers each cell.
  The associated data names the custody with the literal `transit` and the version row's id
  as today, never the Transit key's version, so that a rewrap leaves it valid. Opening MUST unwrap through Transit with
  the same context; a wrapped key moved to another tenant's row MUST fail to unwrap. Values of `openbao` custody (feature
  017) stay in KV, unchanged; `local` rows open with the ring until re-sealed.
- **Requirement 015-012** Rotation MUST need no restart and MUST never outrun the backups: `control-plane secrets rotate`
  rotates `sagat-secrets` in Transit (the new version exists but seals nothing) and records the rotation's time;
  `control-plane secrets advance-seal-version --to N --snapshot-taken-at <time>` advances `seal_version` only when the
  operator attests, in the audited command, an OpenBao snapshot taken after the creation time Transit itself reports
  for version `N` (`key_info`, never the time the platform recorded), and, for a cell with standby regions, its restore
  at every standby, `--standby-restored-at <region>=<time>` (Task 015-32, which waits for feature 032's standby list).
  The platform cannot read a snapshot without restoring it and its policies grant no `sys/` path, so the gate is the
  operator's recorded attestation; the runbook's monthly restore drill checks it. There is no automatic rotation
  period. Each command that changes Transit records an intent, calls Transit, then records the outcome, and a rerun
  reconciles with what Transit reports (Technical Plan 6), so a crash between the two never leaves a change unrecorded
  or an audit record claiming one that did not happen.
  `control-plane secrets rewrap` rewraps stored wrapped keys with `transit/rewrap` (the plaintext data key never leaves
  OpenBao on that path) to a version that a snapshot holds, guarded per row by the old wrapped key, audited per batch in
  the batch's transaction, counting every row it could not rewrap and, apart, every whole-call outage. Rewrap protects
  against the exposure of an older key version, not against data keys already unwrapped with a stolen token: for that,
  `control-plane secrets rekey --issued-before <time>` re-seals the rows whose data keys were issued before it with new
  data keys, after the stolen tokens were revoked. `control-plane secrets retire-key-versions --below N` is refused
  unless `N` is at most `seal_version` (the function that raises the floor checks it again under the row's lock), so the
  floor can never pass the version new seals use. It first raises `key_authority_state.min_seal_version` to `N` in a
  transaction that locks that row, which a trigger then enforces on every insert and every update of `wrapped_key` or
  `wrapped_key_version` of a `transit` row (seals, re-seals and re-keys alike): a seal at an older version fails at its
  commit and is retried with a fresh data key at the `seal_version` it then reads, at most twice, after which the
  write fails `KEY_AUTHORITY_UNAVAILABLE` (retryable). It then counts the rows and failed rows below `N`; only when both
  are zero, and every PostgreSQL standby connected to the primary has replayed past the write-ahead log position read
  after that count, does it set Transit's `min_decryption_version` through the operator's token. A count that is not
  zero leaves the floor raised (it is safe, since `N` is at most `seal_version`), reports the rows and exits non-zero;
  the operator rewraps and runs the command again. The standbys waited for are those `pg_stat_replication` shows on the
  primary; a cell with none skips the wait and records `standbys: none` in the audit record; the standby regions of
  feature 032, which are not built, are waited for by Task 015-32 once that feature provides its list (Owner decision
  2026-11-27-150). `control-plane secrets trim-key-versions --below N` counts the
  rows below `N` again in its own transaction and refuses unless the count is zero and `N` is at most
  `min_decryption_version`. The same commands exist for the other two keys: `scripts rotate-key`,
  `scripts retire-key-versions --below N` (refused while an artifact is signed below `N`) and `intents rotate-key`,
  `intents retire-key-versions --below N` (refused while a pending intent is bound below `N`); a new script-key version
  is used for signing 5 minutes after its rotation (Requirement 015-013), and a new intent-key version at once, since
  verification reads the stored version.
- **Requirement 015-013** Under `transit`, scripts MUST be signed by Transit (`transit/sign/sagat-scripts`, Ed25519) over
  the message `script_signing::message(tenant, digest, language)` of today (made public to the crate), never over a bare
  digest, at a key version published at least 5 minutes earlier. Sandbox runners MUST get the public keys, the minimum
  version and, until every artifact is re-signed, the legacy local public key from the control plane's authenticated
  endpoint (`/script-runner/signing-key`, extended), never from OpenBao; refresh them every 5 minutes and when a
  signature names an unknown version (at most once a minute; a refusal for an unknown key is retryable, never
  `Untrusted`); record in each cached artifact the key that verified it and evict it at a refresh that no longer trusts
  that key.

  *Where signatures live.* Artifact bytes stay immutable (`script_artifacts_immutable`, `ENABLE ALWAYS`, is not
  touched): every signature of an artifact is a row of a new insert-only table `script_artifact_signatures` (Technical
  Plan 5), the first one written with the artifact in its transaction, and the migration copies each existing
  artifact's own `signature` and `key_id` into it as its first row. A local signature has the signer `local`, the key
  id `key_id_of` of the public key as today (16 hexadecimal characters) and no key version; a Transit signature has the
  signer `transit`, the key version `N` parsed from Transit's `vault:v<N>:` prefix, the key id `transit-v<N>`, and the
  64 raw bytes of the Ed25519 signature. The control plane serves, from `/script-runner/artifacts/{digest}`, the newest
  signature (by `signed_at`) whose key it still publishes as trusted, in the existing `signature` and `keyId` fields,
  with the additive `signer` and `keyVersion`; the runner verifies a `local` signature by `keyId` and a `transit` one by
  `keyVersion` against the list it fetched. `script_artifacts.signature` and `key_id`, and `script_versions.key_id`,
  stay frozen as the record of the key that signed at build time; no trust decision reads them after the migration.

  `control-plane scripts resign` inserts, for every artifact that has no signature by the signer of the mode at the
  newest published version (the newest Transit version in `migrating` and `transit`, the local key in `returning`), a
  new signature row; the primary key `(tenant_id, digest, key_id)` makes a rerun or a concurrent run insert nothing
  twice (`ON CONFLICT DO NOTHING`). An artifact counts as "signed below `N`" or "signed by the local key" when it has
  no signature row that the runner would trust: retiring a script-key version and leaving either passage require that
  count to be zero. From `begin-migration` on (in `migrating` and `transit`) a new signature row is signed only by
  Transit and a trigger refuses a new row whose signer is `local`; in `returning` and `local` only the local key signs
  and the trigger refuses a new `transit` row, so the zero counts of `finish-migration` and `finish-return` cannot be
  outrun.

  *The runner gate.* A sandbox runner of 0.2.x trusts one key fetched once (trust on first use,
  `apps/sandbox-runner/src/lib.rs`) and never asks again, so it would refuse every Transit signature for good, and a
  look-back at its fetches alone cannot see it. Every request a runner of this design sends to any `/script-runner/`
  operation carries the header `Sagat-Runner-Trust`: `key-list-v1` when it trusts the control plane's list, or
  `configured-v1` when `SAGAT_RUNNER_TRUSTED_KEYS` is set; a 0.2.x runner sends none, and it polls the claim operations
  all the time it runs. The control plane records, at most once a minute per control-plane process and per (runner
  identity, form), the principal subject of the runner's `script:execute` token, the form (`key-list-v1`,
  `configured-v1` or `absent`) and the time, in `script_runner_trust_sightings` (rows older than 24 hours deleted by
  the upkeep). `begin-migration` is refused while any sighting of the form `absent` or `configured-v1` is younger than
  24 hours, and lists the subjects it saw; and the operator attests, in the audited command, that no stopped runner of
  0.2.x or with hand-configured keys will be started again, the one case no sighting can show. A runner of 0.2.x that is
  started anyway refuses the Transit-signed artifacts (it fails closed) and is counted by
  `script_runner_legacy_requests_total`, with an alert.
- **Requirement 015-014** Under `transit`, feature 017's provisioning intents MUST be bound by Transit HMAC
  (`transit/hmac/sagat-intent`, `algorithm=sha2-256`, so the binding stays 32 bytes) over the intent's metadata and the
  SHA-256 of the value, never the value itself, under a binding version 2 that records the key version. A replay MUST
  read the stored intent first, outside the provider lock, and verify against the stored version (the file key for
  version 1, `transit/verify` at the stored key version for version 2), never by recomputing at the newest version.
  Version-1 bindings are verified with the file key until no pending intent of version 1 remains. From
  `begin-migration` on, a new intent is bound only at version 2, and a trigger refuses a new version-1 row outside
  `local` and `returning` (in `returning`, new intents are bound at version 1 and the trigger refuses a new version-2
  row). The immutability trigger of `secret_provider_operations` covers the two new columns.
- **Requirement 015-015** A deployment MUST be able to move from `local` to `transit` without downtime:
  `control-plane key-authority begin-migration` sets `migrating`, after which a trigger refuses any new `local` row;
  `control-plane reseal-secrets --to transit` (the existing command, extended) re-seals `local` rows in bounded,
  resumable, audited batches, over every entry of the re-seal inventory (Technical Plan 5: `secret_versions` and the
  sealed columns other features register, such as design 040's `audit_sinks`); `control-plane key-authority
  finish-migration` sets `transit` in one transaction that checks, under the state row's lock, zero `local` values in
  every inventory entry, zero `secrets.authority = 'local'`, zero pending version-1 intents and zero artifacts without a
  Transit signature. The way back is the same passage reversed and equally supported: `key-authority begin-return` sets
  `returning` (the processes then need the ring beside their Transit configuration), `reseal-secrets --to local`
  re-seals `transit` values of every inventory entry under the ring, `scripts resign` re-signs with the local key, and
  `key-authority finish-return` sets `local` when zero `transit` values in every entry, zero
  `secrets.authority = 'transit'`, zero pending version-2 intents and zero artifacts without a local signature remain.
  Each of these commands writes its audit record in its own transaction. The exact rollout order of both passages, in
  which no serving process's gate closes, is Technical Plan 10, and a test runs each under load (Task 015-18).
- **Requirement 015-016** Each custody MUST have its own mount or path prefix and its own policy, and each process kind
  its own token per custody it uses, delivered by one OpenBao Agent per custody in its pod. The custodies and grants are
  the table of Technical Plan 7, plus the token self-paths each needs (`auth/token/lookup-self`, `renew-self`,
  `revoke-self`) and `read` on the Transit keys the control plane uses. Rotating, raising `min_decryption_version` and
  trimming run under a separate operator token (`SAGAT_OPENBAO_OPERATOR_TOKEN_FILE`, read only by the commands of
  Technical Plan 6) whose policy, `operator-keys.hcl`, allows exactly: `read` on `sagat-transit/keys/<key>` for the
  three keys (the key's metadata and public keys, never its material, so the commands' `key_info` needs no second
  token); `update` on `sagat-transit/keys/<key>/rotate` for the three keys; `update` on
  `sagat-transit/keys/<key>/config` for the three keys with only the parameter `min_decryption_version`
  (`allowed_parameters`); and `update` on `sagat-transit/keys/<key>/trim` for the three keys with only
  `min_available_version`. A test MUST refuse any other grant, any wildcard in a mount segment, any `sys/` path and any
  other `auth/` path, and any other `keys/*/config`, `keys/*/rotate`, `keys/*/trim`, `export/`, `backup/` or
  `restore/` path of Transit. The test is `scripts/check-openbao-policies.py` (Technical Plan 7). The configured mount
  names are substituted and validated by the bootstrap. The policies of other features (design 040's operator sink,
  design 047's broker) are not written by this design: the bootstrap installs every policy fragment of
  `infra/openbao/policies/<feature>/` with the allow-list file beside it, and the test reads each fragment's expected
  grants from that file, so each feature adds its own in its own delivery. The snapshot policy of Requirement 015-020
  belongs to the operator's OpenBao installation, not to the platform, and lives outside that directory, named as such.
- **Requirement 015-017** If OpenBao cannot be reached, a secret MUST NOT be sealed or opened by any other means
  (Principle V). `KeyAuthorityError` has three kinds: `Unavailable` (retryable), `Refused` (an integrity failure: a key
  that does not open, a version below the minimum, a corrupt row; not retryable, audited, alerted) and `Denied` (the
  token's policy refused). Every reader of a secret maps them, never as "skip": delivery, the output scan of a step,
  webhook signing, inbound webhook verification (`crates/application/src/events.rs`: `Unavailable` answers the sender
  `503` with `Retry-After`, never `Refusal::NoSecret`), the managed-database reader (`crates/application/src/worker/database.rs`),
  the token broker's credentials and the connector supervisor's credentials (`crates/infra-connectors`). A check of
  `scripts/check-architecture.py` refuses a `resolve_secret` whose error is discarded (Technical Plan 4). How each kind
  reaches each surface (the `AppError` variant, the REST status and code, the gRPC status, whether it is retryable, its
  audit and its alert) is the table of Technical Plan 12, with the three catalogue entries.
  A scan that meets `Unavailable` refuses the report as retryable; one that meets `Refused` refuses the report with
  `KEY_AUTHORITY_REFUSED`, and the engine fails the attempt with that non-retryable failure: the step fails without its
  retry policy (a retry cannot cure an integrity failure), the run follows its failure path, the output is never
  stored, and an audit record and an alert name the secret's version row; a step that declares secrets and reaches a
  replica with no reader configured has its report refused as retryable. Opening MAY use an unwrapped data key held in
  a bounded in-memory cache (Technical Plan 8).
- **Requirement 015-018** Every Transit call MUST be bounded (a 2-second timeout per attempt, at most 3 attempts with
  jitter on a transport error or `5xx`, within a total budget of 5 seconds), counted by the existing family
  `secret_provider_requests_total{provider, operation, outcome}` with `provider="transit"` and one outcome set for every
  provider (`ok`, `denied`, `conflict`, `refused`, `failed`, `unavailable`, `timeout`), traced without a value, a wrapped key, a
  context or a token. The existing KV client (`crates/infra-secrets/src/openbao.rs`) moves to the same set, which
  changes its labels: today every error but `Denied` and `Conflict` is `failed`; after this design a transport failure
  is `unavailable`, the client's own timeout `timeout`, and `Configuration`, `Rejected` and `InvalidResponse` stay
  `failed` (KV never answers `refused`, which is an integrity failure of Transit). No alert or dashboard reads the
  family today; the release notes name the change. The client reads its token file on each call and caches the token's
  `lookup-self` by a digest of the file's content, looking up again after a `403`.
- **Requirement 015-019** OpenBao stays the development service of `infra/docker/compose.database.yaml` (TLS, pinned by
  digest, loopback-only, with a healthcheck, prepared by `docs/runbooks/openbao.md`); its bootstrap
  (`infra/openbao/bootstrap-development.py`, and `bootstrap-kubernetes.py` for production) MUST enable Transit, create the
  three keys and install the policies of Requirement 015-016.
- **Requirement 015-020** Under `transit`, PostgreSQL alone no longer restores a secret, so the repository MUST provide,
  for the operator's own OpenBao installation (`infra/openbao/production-values.yaml`, a snapshot job beside it and
  the runbook; the platform's chart does not deploy OpenBao), scheduled, encrypted Raft snapshots of OpenBao under an
  operator-only policy that no platform role holds (Technical Plan 7), with the custody of the unseal and recovery
  keys, and a restore procedure that pairs a PostgreSQL backup with an OpenBao snapshot taken after it (and lowers
  `min_decryption_version` when the database is older than the last rewrap). Old key versions are trimmed
  (`min_available_version`) only by `control-plane secrets trim-key-versions`, the one holder of that capability, which
  refuses below the oldest key version of the oldest database backup the operator declares retained. A restore that
  meets a row it cannot open reports it (`Refused`) and never half-opens silently.
- **Requirement 015-021** Feature 031's customer keys MUST live outside the platform's OpenBao: in a separate cluster or a
  KMS the customer administers (ADR-0066 decision 1, accepted on 2026-11-24 under Owner decision 2026-11-24-4). The policy test of
  Requirement 015-016 is defense in depth, not the separation; the chart's egress NetworkPolicy, which allows only the
  peers listed and refuses global ones, MUST NOT allow a customer key service: the operator lists those services'
  address blocks in `networkPolicy.customerKeyServices`, and the chart refuses to render when any egress `ipBlock` of
  a platform pod overlaps one of them (Technical Plan 11).
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
  every output scan of a Transit row, webhook signing, inbound webhook verification (a retryable `503` to the sender),
  the managed-database reader and the broker's and connectors' credentials fail closed with `Unavailable`; a corrupt row is `Refused`, never retried for ever; service returns when OpenBao does, with no manual
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
Gap 015-007). The two security residual risks, Gaps 015-003 and 015-005, are accepted by the owner (Owner decision
2026-11-27-1, "Aceito os dois, cache 300 s").

## Owner decisions needed

None open. The questions this design raised are decided under Owner decision 2026-11-24-4, each a delegate row (Owner
decisions 2026-11-26-110 to 2026-11-26-117) and an entry of the research register. The revision after the
implementation-readiness read added the delegate rows Owner decisions 2026-11-27-150 to 2026-11-27-157 (the gate of
Task 015-32 on feature 032's standby list, the hand-over window, the signatures table, the runner header, the
Studio's custody choice, the error mapping, the operator-only snapshot and the lock function with `pg_monitor`):

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

*   **Given**: the mode `transit` and 200,000 secret versions whose data keys `sagat-secrets` version 3 wraps.
*   **When**: the operator runs `control-plane secrets rotate`, waits for the scheduled OpenBao snapshot, runs
    `control-plane secrets advance-seal-version --to 4 --snapshot-taken-at <its time>`, then `control-plane secrets
    rewrap`, then `control-plane secrets retire-key-versions --below 4`.
*   **Then**: new versions are sealed under version 4 once the attested snapshot holds it; the pass rewraps the 200,000 wrapped
    keys (at the target of Non-functional requirement 015-003, about 200 seconds) and reports zero rows and zero
    failures below version 4; the retire command raises the sealing floor, counts zero and raises
    `min_decryption_version` to 4; every secret still opens; no process restarted and no delivery failed.

### CUJ 3: OpenBao restarts during a busy hour

*   **Given**: deliveries of the same 500 secrets every second.
*   **When**: OpenBao is unavailable for 40 seconds.
*   **Then**: the deliveries continue from the cache, whose entries the background refresh kept at most 250 seconds old
    (refreshed after 240 seconds, every 10 seconds), so each stays usable for at least 50 seconds; a new secret written in
    that window is refused with `KEY_AUTHORITY_UNAVAILABLE` (retryable) and succeeds when
    the caller retries after OpenBao returns. A step report whose handed-out secrets are cached is scanned and accepted;
    one whose handed-out key is not cached is refused as retryable and accepted when the worker retries after OpenBao
    returns.

## Technical Plan (Architecture)

### 1. Logs over OTLP

`crates/telemetry` gains `LogSink`, the twin of `SpanSink`: `json_log::LogLayer` already builds each record and scrubs
it; when an endpoint is configured the layer also submits the record (severity, target, message, fields, and the
current span's trace id and span id) to a bounded queue (8,192 records) drained by a background task in batches of at
most 512 records or 2 seconds, through `OtlpClient::export_log`. A full queue drops and counts
(`otel_logs_total{outcome="dropped"}`); a failed batch is abandoned and counted, as live spans are. `OtlpConfig`
reads `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT` (and the common endpoint), `OTEL_LOGS_EXPORTER` (`otlp` or `none`; `otlp` by
default, as the OpenTelemetry specification says), a minimum severity (`SAGAT_OTEL_LOGS_LEVEL`: `trace`, `debug`,
`info`, `warn` or `error`, `info` by default) and `OTEL_EXPORTER_OTLP_HEADERS_FILE`. Because the default is `otlp`, an
upgrade of a process that already has the common endpoint set (as `infra/docker/compose.observability.yaml` does)
starts exporting logs; a Collector without a `logs` pipeline answers `404`, which is counted as lost batches
(`otel_logs_total{outcome="lost"}`) and never slows the work; the release notes say to add the pipeline or set
`OTEL_LOGS_EXPORTER=none`. A logs-only configuration (no traces endpoint) is valid, which `from_lookup` today refuses by
returning nothing; in it no `OtlpLayer` is installed, so log records carry no span ids (said in the runbook). Any
`OTEL_LOGS_EXPORTER` value other than `otlp` or `none` is a configuration error (ADR-0017 decision 4).

The public types of `sagat-telemetry` change, and the CLI with them:

- `OtlpConfig.endpoint: String` becomes `traces_endpoint: Option<String>`, and a configuration is valid when at least
  one of the traces and logs endpoints is set; `apps/cli/src/telemetry.rs`, which reads `config.endpoint`, iterates
  over the endpoints that are set.
- `LogData.trace_id` and `span_id` become `Option<TraceId>` and `Option<SpanId>` (absent fields in the OTLP record),
  and `error: bool` becomes `severity: Severity`, an enumeration `Trace`, `Debug`, `Info`, `Warn`, `Error` mapped to
  the OpenTelemetry severity numbers 1, 5, 9, 13 and 17 and their texts; `encode_log` stops mapping a boolean.
- `Open`, the private extension `OtlpLayer` keeps on each span (`otlp.rs`), becomes `pub(crate)` with a crate-visible
  accessor of its trace id and span id, so that `json_log::LogLayer` reads the current span's ids from it.
- The headers move out of the frozen `Arc<OtlpConfig>` into `HeaderSource` (in `otlp.rs`), which holds the current
  headers of each signal behind a lock read once per batch, and which the reload task replaces.

*The header file.* `OTEL_EXPORTER_OTLP_HEADERS_FILE` names a file in the format of `OTEL_EXPORTER_OTLP_HEADERS`: one
line of comma-separated `name=value` pairs, values percent-encoded, a final newline ignored, at most 8 KiB, so that the
OpenBao Agent template and the Kubernetes Secret write the same thing (`Authorization=Bearer%20<token>`). The file
applies to the three signals: for each signal the headers of today (the signal's own `*_HEADERS` variable, else the
common one) are taken first, then each header the file names replaces the header of the same name, compared without
case. A process re-reads the file when its content changes, which a background task checks every 30 seconds by its
size and modification time and then the SHA-256 of its content, and at once after an answer `401` (at most once
every 5 seconds). A file that cannot be read or parsed after a good read keeps the last good headers and is counted
(`otel_header_file_reads_total{outcome="failed"}`, beside `ok`); a file that cannot be read at start is a
configuration error, so the process exports nothing and logs it (Requirement 015-006).

*The resource attributes.* `service.name` comes from `OTEL_SERVICE_NAME`, else the process's default name (today's
rule); `service.version` from the binary's version; `service.instance.id` is the instance registry's id for the
control plane and the execution engine (the identifier each generates at start and registers with) and a random
identifier made at start for the sandbox runner, the database runner and the CLI, which are not in the registry;
`deployment.environment.name` comes from `OTEL_RESOURCE_ATTRIBUTES` when it sets that key, else from `SAGAT_ENV`
(`dev`, `staging` or `prod`, `prod` by default), which all five processes read for this purpose. Other keys of
`OTEL_RESOURCE_ATTRIBUTES` are added as the specification says, except that one of the three keys the process sets
itself is ignored with a warning at start, and a key whose name contains `tenant` or `namespace` is a configuration
error (ADR-0151 decision 4), as is a value longer than 255 characters.

The log targets of the exporter itself (`sagat_telemetry::otlp`) are kept out
of the log queue, so a failing export cannot feed itself. A header from the file with a plain `http` endpoint that is
not loopback is refused as Requirement 015-006 states, and the client marks header values sensitive. `export_log` sends one record per
request today; a batch encoder sends up to 512 per request. The drop counter follows the spans' existing one:
`otel_logs_total{outcome}` beside `otel_spans_total{outcome}`. The CLI keeps its direct
export (`apps/cli/src/telemetry.rs`), adapted to the types above. The four servers (`apps/control-plane`, `apps/execution-engine`,
`apps/sandbox-runner`, `apps/database-runner`) already initialize logging through `sagat_telemetry::init`. `redact.rs`
gains the names and shapes of Requirement 015-007 first, so that both the scrub and the Collector filter generated from
it cover them: `scrub_words` splits on `:`, so `vault:v<N>:` is matched with a lookback over the two previous words, as
`follows_scheme` does; the token shapes are bounded (`hvs.`, `hvb.`, `hvr.` followed by at least 24 characters of the
token alphabet; `s.` followed by exactly 24), so ordinary text is not scrubbed. The CLI has no admin port; its metrics
stay the gauge it already exports over OTLP.

### 2. Metrics through the Collector

No process change. Both Collector configurations add a `prometheus` receiver with one scrape job per process kind,
named as today's jobs (`sagat-control-plane`, `sagat-execution-engine`, `sagat-sandbox-runner`, and
`sagat-database-runner` in Kubernetes), so that the `job` label the alert rules read (`up{job="sagat-execution-engine"}`)
is unchanged; and a `metrics` pipeline. Each metrics store gets the series by one path only, because two paths into
one store give every series twice and the `sum`-based alert rules would count double:

- **Development (compose).** The Collector is the only scraper of the platform's processes (static targets) and
  exports to the compose Prometheus through `prometheusremotewrite`; the Prometheus of
  `infra/docker/compose.observability.yaml` gains `--web.enable-remote-write-receiver` and
  `infra/observability/prometheus.yml` loses its three platform scrape jobs (it keeps its rule files). The dashboards
  and alert rules are unchanged, and Task 015-8 checks that `up{job="sagat-execution-engine"}` is in Prometheus.
- **Production (Kubernetes).** The Collector scrapes by pod discovery (`kubernetes_sd_configs`, role `pod`, in the
  namespace given by `${env:SAGAT_COLLECTOR_NAMESPACE}`, kept by the chart's label `app.kubernetes.io/component`) and
  exports over OTLP to the operator's backend. An operator whose own Prometheus also scrapes the pods (the autoscalers
  of the worker pools read `workerPoolAutoscaling.prometheusAddress`) must not also send the Collector's metrics into
  that same Prometheus; the runbook says so.

### 3. The Collector configurations

`otel-collector.yaml` (development) gains a `logs` pipeline to Loki and the debug exporter, and the `metrics` pipeline
of section 2, and stays without TLS. The compose observability profile gains a Loki service (`grafana/loki`, version 3,
pinned by digest, loopback-only, with a healthcheck, which takes OTLP logs on `/otlp`) and Grafana data sources for
Loki and Jaeger with the link from a log line's trace id to its trace, so that Success criterion 015-001 has a backend
in which to find a log line and follow it to its span.

`otel-collector.production.yaml` is new: OTLP/HTTP receiver on 4318 with TLS from a mounted certificate and the
`bearertokenauth` extension reading its token from a file; processors `memory_limiter` (limit 80 percent of the
container's memory, spike 20 percent), `batch`, and the attribute filter; one active exporter, `otlphttp`, and
commented examples of others (`prometheusremotewrite`, `loki`). It adds no resource attribute (Requirement 015-005).
Every value the operator supplies is an environment placeholder, listed at the top of the file and in the runbook:

| Placeholder | What it is |
|---|---|
| `${env:SAGAT_COLLECTOR_TLS_CERT_FILE}`, `${env:SAGAT_COLLECTOR_TLS_KEY_FILE}` | the receiver's certificate and key |
| `${env:SAGAT_COLLECTOR_BEARER_TOKEN_FILE}` | the file `bearertokenauth` reads the processes' token from |
| `${env:SAGAT_COLLECTOR_NAMESPACE}` | the namespace whose pods the `prometheus` receiver discovers |
| `${env:SAGAT_COLLECTOR_EXPORT_ENDPOINT}` | the operator's OTLP/HTTP backend |
| `${env:SAGAT_COLLECTOR_EXPORT_AUTHORIZATION}` | the `Authorization` header sent to that backend |

The attribute filter replaces string values and keeps every other type, as the process does (Requirement 015-004).
The `attributes` processor cannot do this (its `delete` matches the key whatever the value's type), so it is a
`transform` processor whose statements, for the span, span event and log contexts, are generated from the sensitive
parts of `redact.rs` into one regular expression `<pattern>` and written in this form:

```yaml
- set(cache["sensitive"], attributes)
- keep_matching_keys(cache["sensitive"], "<pattern>")
- replace_all_patterns(cache["sensitive"], "value", "^(?s).*$", "[redacted]")
- merge_maps(attributes, cache["sensitive"], "upsert")
```

`replace_all_patterns` changes string values only, so a matching number is merged back unchanged. A test generates
the statements from `redact.rs` and fails when the file in the repository differs (the drift test), and a second test
sends through the pinned Collector a span with a string `api_token` and an integer `payload_bytes` and sees the first
replaced and the second kept; if the pinned Collector does not behave so, Task 015-5 stops and this design returns to
review, with no fallback to deleting attributes.

The existing step `OpenTelemetry Collector validate` of `.github/workflows/ci.yml` runs `otelcol-contrib validate` on
both files, the production one with every placeholder of the table set to a dummy value and dummy certificate files
mounted; `infra/observability/tests/` keeps holding the promtool rule tests only.

### 4. The key authority ports

The domain port `SecretCipher` (`crates/domain/src/secrets.rs`) is synchronous and holds the ring (`seal(value, aad)`,
`open(sealed, aad)`); it stays as it is and serves `local` rows only. Envelope sealing is a second synchronous domain
port, `EnvelopeCipher` (`seal(value, aad, &DataKey) -> EnvelopeSealed`, `open(&EnvelopeSealed, aad, &DataKey)`),
implemented once over AES-256-GCM in `crates/infra-secrets`; it holds no key. `DataKey` is a domain type holding 32
bytes in `Zeroizing`, with no `Debug` of its content. The keys come from three asynchronous ports in
`crates/application`, object-safe with boxed futures as `SecretProvider` is (`secret_provider.rs`, `ProviderFuture`),
because `SecretReader` holds its readers as `Arc<dyn …>` and the authority is chosen at run time:

```rust
pub type KeyFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, KeyAuthorityError>> + Send + 'a>>;

/// Transit only: there is no local envelope key.
pub trait EnvelopeKeys: Send + Sync {
    fn new_data_key<'a>(&'a self, tenant: TenantId, version: KeyVersion) -> KeyFuture<'a, (DataKey, WrappedKey)>;
    /// `Cacheable::Current` for the version a delivery opens; `Cacheable::ScanOnly` keeps the key out of the cache.
    fn unwrap<'a>(&'a self, tenant: TenantId, wrapped: &'a WrappedKey, cache: Cacheable) -> KeyFuture<'a, DataKey>;
    /// One `batch_input` call; the outer error is a whole-call failure, the items are per row.
    fn unwrap_batch<'a>(&'a self, items: &'a [(TenantId, WrappedKey)]) -> KeyFuture<'a, Vec<Result<DataKey, KeyAuthorityError>>>;
    fn rewrap<'a>(&'a self, batch: &'a [(TenantId, WrappedKey)], to: KeyVersion) -> KeyFuture<'a, Vec<Result<WrappedKey, KeyAuthorityError>>>;
    fn key_info<'a>(&'a self, key: TransitKey) -> KeyFuture<'a, KeyInfo>;
}

/// Two adapters: the local Ed25519 key of today, and Transit `sagat-scripts`.
pub trait ScriptSigner: Send + Sync {
    fn sign<'a>(&'a self, message: &'a ScriptMessage) -> KeyFuture<'a, ScriptSignature>;
    fn public_keys<'a>(&'a self) -> KeyFuture<'a, ScriptKeys>;
}

/// Transit `sagat-intent` only (binding version 2). Version 1 stays inside `ManagedSecretService`, which HMACs
/// the value with the file key as today and verifies version-1 rows until none is pending.
pub trait IntentBinder: Send + Sync {
    fn bind<'a>(&'a self, metadata: &'a IntentMetadata, value_sha256: &'a [u8; 32]) -> KeyFuture<'a, IntentBinding>;
    fn verify<'a>(&'a self, metadata: &'a IntentMetadata, value_sha256: &'a [u8; 32], binding: &'a IntentBinding) -> KeyFuture<'a, bool>;
}
```

`ScriptMessage` is `script_signing::message(tenant, digest, language)` of today, so no signer can be handed a bare
digest. Setting `min_decryption_version`, rotating and trimming are not on these ports: they are commands of the
operator, with the operator's token. The adapters live in `crates/infra-secrets`: `LocalScriptSigner` over today's key,
and `TransitEnvelopeKeys` (which owns the unwrapped-key cache of Technical Plan 8), `TransitScriptSigner` and
`TransitIntentBinder` over one Transit client per mount, which
applies the timeouts of Requirement 015-018, disables redirects and proxies, caps responses at 64 KiB, parses the base64
`plaintext` of a data key into a type that wipes on drop, and bounds its in-flight calls (Technical Plan 8).

`StoredValue` (`crates/application/src/secrets.rs`) gains a third variant, `Transit(EnvelopeSealed)`, beside
`Local(SealedSecret)` and `OpenBao(ProviderReference)`; `EnvelopeSealed` holds the nonce, the ciphertext, the wrapped
key, its version and its issue time, and `SealedSecret` keeps its required `key_id`. `SecretReader` gains an optional
`Arc<dyn EnvelopeKeys>` and `Arc<dyn EnvelopeCipher>`; with none configured a `Transit` row is `Unavailable`, never
skipped. `SecretReader::resolve` is renamed `resolve_secret`, a name no other method of the workspace has, and gains a
batch twin, `resolve_secrets(&[StoredSecret]) -> Result<Vec<Result<SecretValue, KeyAuthorityError>>, AppError>`,
which opens `local` and cached rows directly and unwraps every other `transit` row in one `unwrap_batch` call (the
output scan of Technical Plan 8 uses it). Every `resolve_secret` caller handles the three kinds under Requirement
015-017: delivery
(`crates/application/src/worker/secrets.rs`), the output scan (`secret_in_output`, `checked_secret_report`), webhook
signing and the token broker's credentials (`crates/scheduler/src/deliveries.rs`), inbound webhook verification
(`crates/application/src/events.rs`), the managed-database reader (`crates/application/src/worker/database.rs`) and the
connector supervisor (`crates/infra-connectors/src/credentials.rs`). Five of these turn a resolve error into a skip
today (`worker/secrets.rs`, two sites of `deliveries.rs`, `events.rs`, `credentials.rs`); Task 015-10 corrects them, and
a rule of `scripts/check-architecture.py`, `secret-resolve-never-discards`, works on the text of every Rust file of
`crates/` and `apps/` outside tests. It finds each call `.resolve_secret(` or `.resolve_secrets(` (the unique names
make a text match exact; the other `.resolve(` calls of the workspace, such as `guard.rs`, `scripts.rs`, `relay.rs`
and `executor.rs`, are not matched) and refuses one whose expression, up to the end of its statement or `match`
block, contains any of: `.ok()`, `.is_err()`, `.is_ok()`, `.unwrap_or`, `.unwrap_or_default()`, `.unwrap_or_else(`,
`Err(_) =>` followed by `continue`, `break`, `None` or `Ok(`, `let _ =`, or a `map_err(|_|` closure that ignores its
argument; a site may be listed in its allow-list file with a reason. It also refuses a `fn resolve(` in
`crates/application/src/secrets.rs`, where `SecretReader` is defined, so the old name cannot come back. Negative fixtures prove each
pattern fires, and `docs/engineering/architecture-principles.md` gains its row and one for the `transit`
representation trigger. Delivery and the report path compare a secret's logical identity, not its sealed bytes
(Requirement 015-022).

### 5. Storage

- `key_authority_state`: one row (`mode` in `local`, `migrating`, `transit`, `returning`; `seal_version`, the Transit
  version new data keys are issued at; `min_seal_version`; the time of the last rotation of each key and of the last
  rewrap). The runtime role `sagat_app` has `SELECT` only: the row changes only through functions owned by the
  schema-owning role and not executable by `sagat_app` (`key_authority_set_mode`, `key_authority_advance_seal`,
  `key_authority_raise_floor`, `key_authority_record_rotation`), which take it `FOR UPDATE` and write the command's
  audit record in the same transaction; a trigger `ENABLE ALWAYS` refuses any other change, as ADR-0082 decision 1 does
  for the registry. The commands of section 6 run with the schema-owning role, as `provision-tenant` does. A process
  reads the mode with a plain `SELECT` (Requirement 015-010), which needs no lock.
- *The lock a seal takes.* PostgreSQL requires the `UPDATE` privilege for `SELECT ... FOR SHARE`, which `sagat_app`
  must not have, so the lock is taken inside the database, not by the application: one function,
  `key_authority_share()`, owned by the schema-owning role, `SECURITY DEFINER` with `SET search_path = pg_catalog,
  pg_temp`, takes the state row `FOR SHARE` and returns its mode, `seal_version` and `min_seal_version`. `sagat_app` is
  not granted `EXECUTE` on it (it is revoked from `PUBLIC`): it is called only by the trigger functions below, which
  are `SECURITY DEFINER` too and run as their owner, as the instance registry's functions do. The lock is therefore
  taken by the first write of a sealed value in a transaction and held until that transaction ends. The order of a
  seal is: read the mode and `seal_version` with a plain `SELECT`; call Transit `datakey` at that version, outside any
  database transaction, so that no transaction is held open across a network call of up to 5 seconds; then open the
  write transaction, whose insert fires the trigger, which takes the lock and checks the row against the mode and the
  floor read under it; commit. A function that changes the row (`FOR UPDATE`) therefore waits for every seal holding
  the lock to commit, and the count it or its command makes afterwards sees their rows; a seal that starts after it
  sees the new values. A seal refused by the trigger (a mode or floor that changed after its `SELECT`) reads them again
  and starts over with a fresh data key, at most twice (Requirement 015-012). The privilege audit gains the rows that
  prove `sagat_app` cannot call `key_authority_share` or the four functions, cannot write the table, and that the
  triggers are `ENABLE ALWAYS`.
- A trigger on `secret_versions`, `BEFORE INSERT OR UPDATE OF key_id, wrapped_key, wrapped_key_version`, refuses a new or
  re-sealed `local` row unless the mode is `local` or `returning`; a new, re-sealed, re-keyed or rewrapped `transit` row
  unless the mode is `migrating` or `transit`; and a `transit` row whose `wrapped_key_version` is below
  `min_seal_version`, whatever its writer. A seal or re-seal therefore cannot land under a floor raised after it fetched
  its data key: it fails at its commit and is retried with a fresh key. The same function guards the intent binding
  version (Requirement 015-014) and the signer of a new script signature (below).
- `secrets.authority` is widened to `('local', 'openbao', 'transit')` and means the custody of the secret's new
  versions: `put_version` sets it, and it becomes `transit` when the first `transit` version commits. A re-seal (to
  `transit` or to `local`) sets it to the target custody in the same transaction as the re-seal of the secret's last
  retained version still in the other custody, so that a secret that is never written again after a full passage does
  not keep the old label; an `openbao` secret is never changed by a re-seal. `finish-migration` requires zero
  `secrets.authority = 'local'` and `finish-return` zero `secrets.authority = 'transit'`, which also keeps the `down`
  migration possible after a full return. Reading is routed by each version row's own columns.
- *The re-seal inventory.* Every table that holds values sealed by the ring or under a Transit-wrapped data key is an
  entry of one inventory, so that no passage can finish while a value it would make unreadable remains. An entry has
  three parts, each added by the feature that owns the table in its own delivery: a counting function
  `<entry>_sealed_count(custody text) RETURNS bigint`, owned by the schema-owning role and registered in the owner-only
  table `key_authority_reseal_inventory (name text PRIMARY KEY, count_function regprocedure)`; a mode trigger on its
  table that calls `key_authority_share()` and refuses a new value in the custody the mode forbids; and an
  implementation of the application trait `ResealTarget` (`crates/application/src/key_authority.rs`: `name`,
  `reseal_batch(to, limit)`), listed in the control plane's wiring, which `reseal-secrets` runs. `secret_versions` is
  the first entry; design 040's `audit_sinks` columns are the second (Task 040-26). `key_authority_set_mode` sums, under
  its `FOR UPDATE`, every registered count for the custody the passage leaves, and a test fails when a `ResealTarget`
  has no registered count or a registered count has no `ResealTarget`.
- `secret_versions` gains `wrapped_key text` (at most 1,024 characters, starting `vault:v`), `wrapped_key_version
  integer` and `data_key_issued_at timestamptz`. The representation constraint, on columns only: a `local` row has a
  ring key id, a nonce and a ciphertext and no wrapped key; a `transit` row has no key id, no provider reference, a
  nonce, a ciphertext, a wrapped key, its version and its issue time, and `wrapped_key_version` equals the number in
  `wrapped_key`'s prefix; an `openbao` row keeps feature 017's provider reference. An index on `wrapped_key_version`
  and one on `data_key_issued_at` (partial, `transit` rows) serve the passes. A migration cannot read
  `SAGAT_SECRETS_KEYS`, so the migration refuses to run when an existing row has `key_id = 'transit'`, and
  `KeyRing::parse` refuses that id afterwards. The `down` migration refuses while any `transit` row, any
  `secrets.authority = 'transit'` or a mode other than `local` exists, because the old representation constraint cannot
  hold them.
- `script_artifact_signatures` (`tenant_id uuid`, `digest text`, `signer text` in `local` and `transit`, `key_id text`,
  `key_version integer`, `signature bytea`, `signed_at timestamptz DEFAULT clock_timestamp()`; primary key
  `(tenant_id, digest, key_id)`; foreign key to `script_artifacts (tenant_id, digest)`) holds every signature of an
  artifact (Requirement 015-013). Its constraints: `octet_length(signature) = 64`; a `local` row has
  `key_id ~ '^[0-9a-f]{16}$'` and no `key_version`; a `transit` row has `key_version >= 1` and
  `key_id = 'transit-v' || key_version`. Triggers `ENABLE ALWAYS` refuse every `UPDATE`, `DELETE` and `TRUNCATE`, as
  for `script_artifacts`, and a `BEFORE INSERT` trigger calls `key_authority_share()` and refuses a `local` row in
  `migrating` and `transit` and a `transit` row in `returning` and `local`. `sagat_app` gets `SELECT, INSERT` only. The
  migration inserts one row per existing artifact from its own `signature` and `key_id` (signer `local`). The counts
  of `finish-migration`, `finish-return` and `scripts retire-key-versions` are counts of artifacts with no signature
  row the runner would trust (a `transit` row at or above the minimum version, or a `local` row while the legacy key is
  published), served by an index on `(tenant_id, digest, signer, key_version)`. The durable-quota accounting of
  `script_artifacts` (migration `20261024000000_durable_quotas`) does not count these rows: each is under 200 bytes and
  bounded by the number of signing-key versions.
- `script_runner_trust_sightings` (`subject text`, `form text` in `key-list-v1`, `configured-v1`, `absent`,
  `last_seen_at timestamptz`; primary key `(subject, form)`; rows older than 24 hours deleted by the control plane's
  upkeep) serves the runner gate of Requirement 015-013; `sagat_app` may insert, update its `last_seen_at` and delete.
- `secret_provider_operations` gains `binding_version smallint NOT NULL DEFAULT 1` and `binding_key_version integer`,
  both covered by its immutability trigger; the binding stays 32 bytes.
- `steps` gains `lease_granted_at timestamptz`, set on the database clock by the statement that grants a lease, in
  both lease producers (`crates/infra-postgres/src/execution.rs` and `execution/path.rs`), in the same `UPDATE` that
  writes `lease_token_hash`, so it costs no extra write; it is null for a lease granted before the upgrade (Technical
  Plan 8).
- Every new variable-width column has its entry in `crates/infra-postgres/storage-classification.json`, which lists
  each such column with one of its four classes (the review's "classification registry"; ADR-0033 classifies payload
  fields, not columns, so it is not what changes). The registry gains no new class: `secret_versions.wrapped_key` is
  `protected_system`, with the measurement `protected_bound` and the evidence "bounded to 1,024 characters by the CHECK
  `secret_versions_wrapped_key`", as `key_id` and `nonce` are today; `script_artifact_signatures.signer`, `key_id` and
  `signature` are `protected_system` with their CHECKs as evidence, and `digest` as for `script_artifacts`;
  `script_runner_trust_sightings.subject` is `protected_system` (an identity subject bounded to 255 characters) and
  `form` `protected_system` (a closed set); `key_authority_state.mode` and `key_authority_reseal_inventory.name` are
  `protected_system`. The registry's exhaustiveness test fails when one is missing.
- A `transit` → `openbao` move (feature 017's `migrate`) is offered: the predicate of
  `crates/infra-postgres/src/managed_secrets.rs` that selects a migration source (`s.authority='local'`) accepts
  `'transit'` too, and `ManagedSecretService` opens the source through the full `SecretReader` (ring and Transit)
  instead of the ring-only `SecretReader::local` (`crates/application/src/managed_secrets.rs`). Task 015-31 builds it.

### 6. Commands

Every command runs with the schema-owning role, changes `key_authority_state` only through the functions of section 5,
and writes its audit record (action `key_authority.<command>`, the actor, the versions and counts, never a key, a
wrapped key or a token) in the transaction that commits its change; each new action has its row in design 040's total
mapping table (Task 015-26), whose completeness test fails otherwise.

*Tokens.* Data operations (`datakey`, `decrypt`, `rewrap`, `sign`) use the control plane's platform-keys token
(`SAGAT_OPENBAO_TRANSIT_TOKEN_FILE`); key administration (`rotate`, `config`, `trim`) and `key_info` use the
operator's token (`SAGAT_OPENBAO_OPERATOR_TOKEN_FILE`), which no serving process reads.

*A command that changes Transit.* OpenBao is not part of any PostgreSQL transaction, so each command that changes it
(`secrets rotate`, `retire-key-versions`, `trim-key-versions`, and the rotate and retire commands of the other two
keys) runs three steps: (1) it commits an audit record `key_authority.<command>.started` with the key, the expected
result (for a rotation, the version Transit will create, read with `key_info` as its latest version plus one; for a
config or trim, the value) and the actor; (2) it calls Transit; (3) it commits `key_authority.<command>.completed` with
what Transit then reports through `key_info`, and, for a rotation, `key_authority_record_rotation` with Transit's own
creation time of the new version. A crash between (1) and (3) leaves a `started` record with no `completed`, which the
next run of any command for that key reads first: for a rotation it compares `key_info` with the expected version and
records `completed` without rotating again when the version exists (a rotation is not idempotent: each call creates a
version), or rotates when it does not; a config or a trim is idempotent and is sent again. The guard of
`advance-seal-version` compares the attested snapshot time with Transit's creation time of version `N` from
`key_info`, never with the recorded time.

*Standbys.* The replay waits of `rekey` and `retire-key-versions` read, on the primary, every row of
`pg_stat_replication` and wait until each standby's `replay_lsn` has passed the position, at most 10 minutes, then
refuse. Reading `replay_lsn` needs the predefined role `pg_monitor`, which the runbook's role setup grants the
schema-owning role; when the column reads null for a connected standby the command refuses instead of skipping it. A
cell with no row skips the wait and records `standbys: none`. Feature 032's standby regions (the restore attestation
of `advance-seal-version` and the replay of each region) are added by Task 015-32 once feature 032 provides its list
(Owner decision 2026-11-27-150); until then a cell has no standby region, and the commands record so.

| Command | What it does | Guard |
|---|---|---|
| `key-authority begin-migration` | sets `migrating` | the mode is `local`; the requirement `key-authority-transit-v1` is recorded for the control plane and the execution engine (instance registry, ADR-0082); no runner sighting of the form `absent` or `configured-v1` younger than 24 hours, and the operator's attestation for stopped runners (Requirement 015-013) |
| `reseal-secrets --to transit` (existing, extended) | re-seals the `local` values of every entry of the re-seal inventory, one per transaction, audited, setting `secrets.authority` as Technical Plan 5 says | the mode is `migrating`; without `--to` it keeps today's behaviour for `local` rows |
| `scripts resign` | inserts a signature row by the signer of the mode (the newest published Transit version in `migrating` and `transit`, the local key in `returning`) for every artifact that has none | `ON CONFLICT DO NOTHING` on `(tenant_id, digest, key_id)`; the mode is `migrating`, `transit` or `returning` |
| `key-authority finish-migration` | sets `transit` | the mode is `migrating`; under the state row's lock: zero `local` values in every inventory entry, zero `secrets.authority = 'local'`, zero pending version-1 intents, zero artifacts without a trusted Transit signature |
| `key-authority begin-return` | sets `returning` | the mode is `transit`; the requirement `key-authority-ring-absent-v1` is not recorded; `key-authority-return-v1` is recorded for both kinds, which a process lists only when its ring and its Transit configuration are both present |
| `reseal-secrets --to local` | re-seals the `transit` values of every inventory entry under the ring, one per transaction, audited, setting `secrets.authority` | the mode is `returning` |
| `key-authority finish-return` | sets `local` | the mode is `returning`; under the state row's lock: zero `transit` values in every inventory entry, zero `secrets.authority = 'transit'`, zero pending version-2 intents, zero artifacts without a local signature |
| `secrets rotate` | rotates `sagat-secrets` in Transit with the operator's token, in the three steps above | the mode is `transit` or `migrating` |
| `secrets advance-seal-version --to N` | advances `seal_version` to `N` | `N` is above the current `seal_version` and exists in Transit (`key_info`); the attested snapshot time is after Transit's creation time of `N` |
| `secrets rewrap` | rewraps rows below the target version, batches of 250 ordered by id, each batch's updates and its audit record in one transaction | per row `WHERE id = $id AND wrapped_key = $old`; the target is at most `seal_version`; a restart begins again from the start, which is idempotent |
| `secrets rekey --issued-before T` | re-seals rows whose `data_key_issued_at` is before `T` with new data keys, one per transaction, audited, with an id cursor; reports the write-ahead log position of its last commit and waits for the standbys | `WHERE id = $id AND wrapped_key = $old AND nonce = $old_nonce` |
| `secrets retire-key-versions --below N` | raises `min_seal_version`, counts, waits for the standbys past the position read after the count, then sets Transit's `min_decryption_version`, in the three steps above | `N` is at most `seal_version` (checked again by `key_authority_raise_floor` under the lock); zero rows and zero failures below `N` after the fence committed, else the floor stays raised and the command exits non-zero |
| `secrets trim-key-versions --below N` | sets `min_available_version` with the operator's token, in the three steps above | a recount in its own transaction finds zero rows below `N`; `N` is at most `min_decryption_version` and not above the oldest key version of the oldest retained backup the operator declares |
| `scripts rotate-key`, `scripts retire-key-versions --below N` | rotate `sagat-scripts`; raise its `min_decryption_version`, in the three steps above | retire: `N` is at most the newest published version; zero artifacts without a trusted signature at or above `N` |
| `intents rotate-key`, `intents retire-key-versions --below N` | rotate `sagat-intent`; raise its `min_decryption_version`, in the three steps above | retire: zero pending intents bound below `N` |

The runbook's procedure after a token is stolen: revoke the token's accessor and every token issued to its role (rotate
the Kubernetes service account), then rotate, advance the sealing version, rekey (which waits for the standbys),
rewrap and retire (which waits for the standbys too). For a cell with standby regions, feature 032's key inventory
fingerprints the three keys as blocking checks (`specs/032-multi-region-failover/research.md`); under `transit` a key
held in Transit has no material to fingerprint, so the inventory reads, per region, the key's name and latest version
through `key_info` instead. That change belongs to feature 032 and is recorded as its dependency (Gap 015-006).

Task 015-11 begins by proving, against the pinned OpenBao 2.7.1, that `datakey` issues at the requested `key_version`
and that `rotate`, `config` and `trim` accept exactly the parameters above; if `datakey` ignores the version, the task
stops and this design returns to review, with no silent fallback to the latest version.

### 7. Mounts, tokens and policies

| Custody | Mount or path and grants | Control plane | Execution engine | Database runner | Token broker (047) | Operator |
|---|---|---|---|---|---|---|
| Platform keys | `sagat-transit`: `datakey`, `decrypt`, `rewrap` on `sagat-secrets`; `sign` on `sagat-scripts`; `hmac`, `verify` on `sagat-intent`; `read` on the three keys | all of these | `decrypt` on `sagat-secrets` | none | none | `read` on the three keys; `rotate` on the three keys; `config` on the three keys limited to `min_decryption_version`; `trim` on the three keys limited to `min_available_version` |
| Feature 017 values | the configured KV mount (`secret` by default), `sagat/credentials/*` | as `management.hcl` | as `runtime.hcl` | through the engine's capability | none | none |
| Collector credential | the KV mount, path prefix `sagat/telemetry/` with a policy of its own | read | read | none: a Kubernetes Secret mounted as its header file | none | none |
| Other features' custodies | a policy fragment and allow-list file under `infra/openbao/policies/<feature>/`, added by that feature (design 040's `sagat/audit/` prefix, design 047's OAuth mount) | as the fragment declares | as the fragment declares | as the fragment declares | as the fragment declares | none |

The secret cleanup job keeps `cleanup.hcl` (no read of a value) and gets no Transit grant: it opens no secret. The
sandbox runner and the database runner get no OpenBao token at all: the database runner uses the shared service
account and has no Agent (`infra/helm/templates/database-runner.yaml`), and giving it one would open the path to OpenBao
that this design refuses the sandbox runner; its Collector header comes from a Kubernetes Secret the chart mounts. The
Helm chart has no sandbox-runner template, and its configuration (the Collector credential included) is the
operator's. The custody tables are "mount or path prefix": the KV custodies share the KV mount under prefixes, each
with its own policy. Each process kind authenticates with its own Kubernetes service
account; a pod that uses more than one custody runs one OpenBao Agent sidecar per custody
(`infra/helm/templates/openbao-agent.yaml`, extended), each with its role, policy and token file; in compose the
bootstrap writes one token file per (process kind, custody). A compromised process holds every token it uses; the
separation is that a code path reads only its custody's token file and that another process kind holds none of them.
The bootstrap enables the Transit engine at the configured mount (`sagat-transit` by default), substitutes and
validates the configured mount names, creates the three keys and installs the policies. Each key is created by
`POST <mount>/keys/<name>` with exactly these parameters, then configured by `POST <mount>/keys/<name>/config`:

| Key | Creation | Configuration after creation |
|---|---|---|
| `sagat-secrets` | `type=aes256-gcm96`, `derived=true` (so every `datakey`, `decrypt` and `rewrap` needs the tenant's context, and a call without it is refused), `convergent_encryption=false`, `exportable=false`, `allow_plaintext_backup=false`, `auto_rotate_period=0` | `deletion_allowed=false` |
| `sagat-scripts` | `type=ed25519`, `derived=false` (one public key per version, which the runners trust), `exportable=false`, `allow_plaintext_backup=false`, `auto_rotate_period=0` | `deletion_allowed=false` |
| `sagat-intent` | `type=hmac`, `key_size=32`, `derived=false`, `exportable=false`, `allow_plaintext_backup=false`, `auto_rotate_period=0` | `deletion_allowed=false` |

`exportable` and `allow_plaintext_backup` can only ever be turned on, so they are set at creation; `deletion_allowed`
is a configuration of an existing key. When a key already exists the bootstrap reads it and refuses to go on, with the
differing field named, unless its type, `derived`, `convergent_encryption`, `exportable`, `allow_plaintext_backup`,
`deletion_allowed` and `auto_rotate_period` are exactly those of the table; it never changes an existing key.

The policy test of Requirement 015-016 is `scripts/check-openbao-policies.py`, a checker in Python with the standard
library only, run by the continuous integration workflow beside the other checkers and by `/verify`. It reads every
`.hcl` file of `infra/openbao/policies/` and of its subdirectories, accepts only the policy subset `path "<path>" {
capabilities = [...] allowed_parameters = { "<name>" = [] } }` (a file it cannot read in that subset is refused, not
skipped), and compares the grants with the allow-list file beside each, `<policy>.allow.json`, of the form
`{"policy": "<name>", "grants": [{"path": "...", "capabilities": [...], "allowed_parameters": {"<name>": []}}]}`,
exactly: a grant missing from either side fails. It also applies the refusals of Requirement 015-016 to every file
whatever its allow-list says.

The snapshot schedule of Requirement 015-020 belongs to the operator's OpenBao installation, which the platform's chart
does not deploy (`infra/helm/values.yaml`). Its policy, `infra/openbao/operator-only/snapshot.hcl`, grants `read` on
`sys/storage/raft/snapshot` and nothing else, and is the one file of the repository that names a `sys/` path; it lives
outside `infra/openbao/policies/`, so the platform's allow-list test does not read it, and the checker instead
verifies that this directory holds exactly that grant and that no role of the bootstrap's platform role table binds
it. The snapshot job (`infra/openbao/operator-only/snapshot-cronjob.yaml`, referenced by
`infra/openbao/production-values.yaml`) runs in the OpenBao namespace under a service account of its own, logs in
through the Kubernetes auth role `sagat-operator-snapshot`, saves the snapshot with `bao operator raft snapshot save`,
encrypts it inside the pod with the operator's public key (`age`, the image pinned by digest) before it leaves, and
writes it to the destination the operator configures; the runbook gives the schedule (at least daily, and after every
rotation before `advance-seal-version`), the retention and the restore. The Collector's bearer token is
rotated with an overlap: the Collector is given the new token beside the old one, then the processes' token file
changes (they read it again), then the old token is removed.

### 8. Fail closed, with a bounded cache

`TransitEnvelopeKeys` keeps an LRU of unwrapped data keys keyed by (tenant, wrapped key digest), driven by the
platform's `Clock`: at most 4,096 entries and 512 per tenant, each dropped 300 seconds after its unwrap and wiped on
eviction. Only keys of current versions (the version a delivery opens, `Cacheable::Current`) enter it; the output
scan's opens of older versions use a key held for that scan only (`Cacheable::ScanOnly`).

The output scan of a step report opens, for each declared secret, only the versions that could have been handed out to
that attempt: the version current when the attempt's lease began and every version created since (both on the database
clock), not every kept version; for a `local` row the scan stays as today, since it costs no network call. The lease's
start is `steps.lease_granted_at` (Technical Plan 5); a lease granted before the upgrade has none, and its scan opens
every kept version, as today. The store port gains `secret_versions_since(names, since: Option<Timestamp>)` beside
`secret_versions`, implemented in both stores (`crates/infra-postgres/src/execution.rs` and `execution/path.rs`): per
secret, the newest version created at or before `since` and every version created after it, or every version when
`since` is null. The keys of those versions that are not cached are unwrapped through
`SecretReader::resolve_secrets` in one `unwrap_batch` call per report (Transit's `batch_input`), so a report costs at
most one Transit round trip. During an outage a report whose keys are all cached is scanned; any other is refused as
retryable (CUJ 3). Task 015-30 builds this.

Each Transit client bounds its calls: at most 64 in flight per process and 8 per tenant, a call beyond either waiting
at most its budget of Requirement 015-018 and then failing `Unavailable`; `transit_calls_in_flight` (a gauge) and
`transit_calls_throttled_total{scope}` (`scope` in `process`, `tenant`; no tenant label) show it, so one tenant cannot
saturate the shared OpenBao. While OpenBao answers, a background task every 10 seconds unwraps again the
entries older than 240 seconds that were used in the last 300 seconds; an entry still cached therefore stays usable for
at least 50 seconds after OpenBao stops answering (300 seconds, less the 240 at which it is refreshed, less the 10 of
the refresh period), and at most 300 seconds after a revocation of the platform's token. Nothing else is cached: a seal,
a signature and an HMAC always call Transit.

### 9. Telemetry of OpenBao itself

`secret_provider_requests_total{provider, operation, outcome}` and `secret_provider_request_seconds{provider, operation}`
(the existing family; `provider` gains `transit`; `operation` in `datakey`, `decrypt`, `rewrap`, `sign`, `hmac`,
`verify`, `read`; `outcome` in the one set of Requirement 015-018), `key_cache_entries`, `key_cache_hits_total{outcome}`,
`secret_versions_by_custody{custody}` (sampled every 60 seconds by the control plane's metrics upkeep, since a one-shot
command is never scraped), `transit_calls_in_flight`, `transit_calls_throttled_total{scope}`,
`key_authority_local_key_configured{key}` (Requirement 015-010) and `script_runner_legacy_requests_total` (Requirement
015-013). Four alert rules are added to `infra/observability/alerts.yaml`, each with promtool cases in
`infra/observability/tests/key_authority_test.yaml` (which the continuous integration step runs, as it runs every
`*_test.yaml`): `KeyAuthorityUnavailable` on `secret_provider_requests_total{provider="transit",outcome="unavailable"}`
rising for 2 minutes; `KeyAuthorityRefused` on any increase of the outcome `refused`; `KeyAuthorityDenied` on any
increase of the outcome `denied` for `provider="transit"`; and `ScriptRunnerLegacy` on any increase of
`script_runner_legacy_requests_total` while the mode is not `local` (the control plane exports the mode as
`key_authority_mode{mode}`, 1 for the recorded mode). The platform dashboard gains a row: Transit calls by outcome, the
cache's hit ratio and entries, the calls in flight and throttled, and the secret versions by custody. Spans `openbao.<operation>` carry the key name and version, never a wrapped key, a
context or a token. Each server process sets `RLIMIT_CORE` to 0 at start and, on Linux, `PR_SET_DUMPABLE` to 0, through
the safe wrappers of `rustix` (the workspace forbids `unsafe` code), so that no core dump holds key material; on other
platforms (development on macOS) only `RLIMIT_CORE` is set, and the start log says so.

### 10. Rollout

`local` is the default, so an upgrade changes nothing until the operator acts. The columns and the state row are
additive. A binary older than this design reads a `transit` row as corrupt (it has no key id and no provider reference)
and fails closed on every path, the output scan included (Requirement 015-011), so a stale replica can refuse work but
never accept unscanned output. `begin-migration` is gated by the instance registry as ADR-0082 provides: a control
plane or execution engine lists the capability `key-authority-transit-v1` only when its Transit configuration is
present, the operator records the requirement for both kinds, which the registry refuses while a live process lacks it,
and a process restarted later without the configuration keeps its gate closed (ADR-0082 decisions 3 and 7). The secret
cleanup job opens no secret and is not gated; the sandbox runners are gated by their sightings and the operator's
attestation (Requirement 015-013). Rolling back from `transit` to `local` is the `returning` passage of Requirement
015-015, gated the same way by `key-authority-return-v1`, guarded, audited and tested like the way in; never automatic.

The capabilities a process lists, each only for what its configuration has (ADR-0082 decision 7):
`key-authority-transit-v1` with a Transit configuration; `key-authority-return-v1` with both the ring and a Transit
configuration; `key-authority-ring-absent-v1` with none of the three local keys. Requirements are recorded with the
generic `control-plane capability-activate <capability>` and removed in the two phases of ADR-0131.

*The way in, in this order; no serving process's gate closes at any step:*

1. Upgrade every process to this release (mode `local`) and every sandbox runner, which then sends
   `Sagat-Runner-Trust`.
2. Run the bootstrap of Technical Plan 7 against OpenBao, then add the Transit configuration to every control plane and
   execution engine by a rolling restart, keeping their local keys; each now lists `key-authority-transit-v1` and
   `key-authority-return-v1`, and under `local` it stays open and seals nothing with Transit.
3. Record `key-authority-transit-v1` for both kinds (refused while a live process lacks it).
4. `key-authority begin-migration`. From the next heartbeat, every process sees `migrating`; each has the ring and
   Transit, so each stays open.
5. `reseal-secrets --to transit`, `scripts resign`, and wait until no version-1 intent is pending.
6. `key-authority finish-migration`. Every process sees `transit` within a heartbeat; each still holds its local keys,
   so it stays open, never uses them, and warns (Requirement 015-010).
7. Remove the ring, the script-signing key and the intent key file from every control plane and engine by a rolling
   restart; each now lists `key-authority-ring-absent-v1` and the warning stops.
8. Record `key-authority-ring-absent-v1` for both kinds (refused while a live process still holds a local key). From
   now on a process started with a local key keeps its gate closed.

*The way back, in this order:*

1. Remove the requirement `key-authority-ring-absent-v1` (both phases of ADR-0131).
2. Add the ring, and the intent key file, to every control plane and engine by a rolling restart, keeping Transit; each
   lists `key-authority-return-v1` again and, under `transit`, stays open and warns.
3. Record `key-authority-return-v1` for both kinds.
4. `key-authority begin-return`. Every process sees `returning` within a heartbeat and has both, so it stays open.
5. `reseal-secrets --to local`, `scripts resign` (which needs the script-signing key on the control plane), and wait
   until no version-2 intent is pending.
6. `key-authority finish-return`. Every process sees `local`; its Transit configuration stays and is not used.
7. Remove the requirements `key-authority-return-v1` and `key-authority-transit-v1` (both phases), then, if the
   operator wishes, the Transit configuration by a rolling restart.

Task 015-18 runs both orders under load (a secret resolved every 10 milliseconds) with a rolling restart at each
configuration step and checks that no process's gate closes and no resolution fails; and that, after step 8 of the way
in, a process started with the ring is refused, and, before step 1 of the way back, `begin-return` is refused.

### 11. Configuration

The variables this design adds or reads, each validated at start (a value that is set and cannot be used is a
configuration error, ADR-0017 decision 4). Task 015-29 wires them into `apps/control-plane` and
`apps/execution-engine` and documents them in the reference of Task 015-22 before the code is written.

| Variable | Process kinds | Default | Validation |
|---|---|---|---|
| `SAGAT_OPENBAO_ADDRESS`, `SAGAT_OPENBAO_CA_FILE` (feature 017's, unchanged) | control plane, engine, the key commands | unset | an `https` origin with no credentials, path or query; a readable PEM file; both required when a Transit token file is set |
| `SAGAT_OPENBAO_TRANSIT_TOKEN_FILE` | control plane, engine, the key commands | unset: no Transit configuration | a readable file (the platform-keys Agent's sink); setting it is what gives a process a Transit configuration |
| `SAGAT_OPENBAO_TRANSIT_MOUNT` | control plane, engine, the key commands | `sagat-transit` | `^[A-Za-z0-9_-]{1,64}$` |
| `SAGAT_OPENBAO_OPERATOR_TOKEN_FILE` | the key commands of Technical Plan 6 only | unset | a readable file; set on `serve` or on the engine it is a configuration error, so no serving process holds the operator's token |
| `SAGAT_SECRETS_KEYS`, `SAGAT_SCRIPT_SIGNING_KEY`, `SAGAT_OPENBAO_INTENT_KEY_FILE` (today's local keys) | as today | unset | as today; whether each is set decides the capabilities above |
| `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, `OTEL_LOGS_EXPORTER` | the five processes | the common endpoint; `otlp` | Technical Plan 1 |
| `SAGAT_OTEL_LOGS_LEVEL` | the five processes | `info` | `trace`, `debug`, `info`, `warn` or `error` |
| `OTEL_EXPORTER_OTLP_HEADERS_FILE` | the five processes | unset | Technical Plan 1 |
| `SAGAT_OTEL_ALLOW_PLAINTEXT_HEADERS` | the five processes | `false` | `true` or `false` |
| `OTEL_RESOURCE_ATTRIBUTES`, `SAGAT_ENV` | the five processes | unset; `prod` | Technical Plan 1 |

The Transit call budgets (Requirement 015-018), the in-flight bounds and the cache bounds (Technical Plan 8) are
constants of the code, not configuration, because Owner decision 2026-11-26-116 fixed them. The wiring builds, when a
Transit configuration is present, one `TransitClient` per mount and from it `TransitEnvelopeKeys`,
`TransitScriptSigner` and `TransitIntentBinder`, and hands them to every holder of a `SecretReader` or a signer that
today is built from the ring only: `SecretService` and `ManagedSecretService`, the `WorkerService` vault, the
scheduler's deliveries, the events service and the connector supervisor in the engine, and the script service in the
control plane.

The Helm chart's values and their validation (Task 015-35):

| Value | Default | Validation |
|---|---|---|
| `openbao.transit.enabled` | `false` | requires `openbao.enabled`; renders the platform-keys Agent and `SAGAT_OPENBAO_TRANSIT_TOKEN_FILE` for the control plane and the engine |
| `openbao.transit.mount` | `sagat-transit` | a single name, as `openbao.mount` |
| `openbao.transit.roles.control-plane`, `openbao.transit.roles.execution-engine` | `sagat-platform-keys-management`, `sagat-platform-keys-runtime` | required when Transit is enabled |
| `keyAuthority.localKeys` | `present` | `present` or `absent`; `absent` requires `openbao.transit.enabled` and refuses `secrets.existingSecret` and `openbao.intentKeySecret` (the chart sets no script-signing key today); `openbao.intentKeySecret`, required today whenever `openbao.enabled`, is required only with `present` |
| `networkPolicy.customerKeyServices` | `[]` | a list of IPv4 address blocks; the chart fails when any egress `ipBlock` it renders for a platform pod (the PostgreSQL, OIDC, NATS, Redis, OpenBao, telemetry and extra peers, and `databaseRunner.targetEgress`) overlaps one of them, compared as integer intervals in the helper `sagat.networkPolicy.validateCustomerKeyServices`; an IPv6 block is refused with a message until the helper compares IPv6 (Gap 015-007) |
| `telemetry.otlpEndpoint` | `""` | an `https` origin with no credentials; sets `OTEL_EXPORTER_OTLP_ENDPOINT` for the control plane, the engine and the database runner; with `networkPolicy.enabled` it requires `telemetry.egress` |
| `telemetry.egress` | `[]` | explicit Collector peers, validated as the other egress rules; rendered for every pod that exports |
| `telemetry.logsLevel` | `info` | as `SAGAT_OTEL_LOGS_LEVEL` |
| `telemetry.headersSecret` | `""` | a Kubernetes Secret with the key `headers`, mounted as the database runner's header file |
| `databaseRunner.telemetry.tracesEndpoint`, `databaseRunner.telemetry.egress` (today's) | `""`, `[]` | kept, deprecated: rendered as today when `telemetry.otlpEndpoint` is empty; setting both forms fails |

### 12. How the three error kinds reach each surface

| Kind | `AppError` variant | REST status and code | gRPC status (worker API) | Retryable | Audit | Alert |
|---|---|---|---|---|---|---|
| `Unavailable` | `KeyAuthorityUnavailable` | `503`, `KEY_AUTHORITY_UNAVAILABLE`, `Retry-After: 5` | `UNAVAILABLE` | yes | none; counted with the outcome `unavailable` or `timeout` | `KeyAuthorityUnavailable` after 2 minutes |
| `Refused` | `KeyAuthorityRefused` | `500`, `KEY_AUTHORITY_REFUSED` | `DATA_LOSS` | no | `key_authority.refused`: tenant, the version row's id, and a reason in `unwrap_failed`, `below_minimum`, `corrupt_row`, `signature_refused` | `KeyAuthorityRefused` at once |
| `Denied` | `KeyAuthorityDenied` | `500`, `KEY_AUTHORITY_DENIED` | `FAILED_PRECONDITION` | no | `key_authority.denied`: the operation and the key's name | `KeyAuthorityDenied` at once |

Each code carries the error code in the gRPC error details, as the existing worker errors do. On the paths that do not
answer a caller: a scheduler delivery or a token-broker credential retries `Unavailable` with its own backoff and fails
the delivery non-retryably on the other two, with the code on the attempt; the connector supervisor retries
`Unavailable` and stops the connector on the other two, with the code in its status; inbound webhook verification
answers the sender `503` with `Retry-After` on `Unavailable` and `500` on the other two, accepting nothing; the output
scan does what Requirement 015-017 says. The three entries of `sdks/error-catalog.json`:

```json
{"code": "KEY_AUTHORITY_UNAVAILABLE", "origin": "rest", "http": [503], "grpc": "UNAVAILABLE", "retryable": true, "remediation": "The platform's key service cannot be reached: retry with the same idempotency key after Retry-After; the operator is alerted."},
{"code": "KEY_AUTHORITY_REFUSED", "origin": "rest", "http": [500], "grpc": "DATA_LOSS", "retryable": false, "remediation": "A stored secret failed its integrity check and cannot be opened: do not retry; report the trace id to the operator, who restores or rewrites the secret."},
{"code": "KEY_AUTHORITY_DENIED", "origin": "rest", "http": [500], "grpc": "FAILED_PRECONDITION", "retryable": false, "remediation": "The key service refused the platform's own credentials: do not retry; report the trace id to the operator, who checks the OpenBao policies."}
```

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
| A 0.2.x sandbox runner refuses Transit signatures for good | Every runner request carries `Sagat-Runner-Trust`; `begin-migration` is refused while a runner without it, or with hand-configured keys, was seen in the last 24 hours, and the operator attests for stopped runners; a late one fails closed and is alerted |
| A passage closes every serving process's gate | The configuration each mode needs is chosen so that no step of either rollout order closes one (Technical Plan 10); local keys leave only after `finish-migration`, through the requirement `key-authority-ring-absent-v1`; a test runs both orders under load |
| A passage finishes while another feature's sealed values remain | One re-seal inventory, counted under the state row's lock by `key_authority_set_mode` |
| Re-signing breaks the immutability of artifacts | Signatures are rows of an insert-only table; artifact bytes are never updated |
| A compromised API process lowers the sealing floor or advances the sealing version | `key_authority_state` changes only through owner-role functions that `sagat_app` cannot call; the privilege audit proves it |
| `transit` is a one-way door | The `returning` passage, gated and tested like the way in |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Secret versions stay in PostgreSQL; rewrap, re-key and re-seal are idempotent per row, guarded by the old version or key id |
| II. Modular domain, ports and adapters | Pass | `SecretCipher` and `EnvelopeCipher` in the domain; the ports `EnvelopeKeys`, `ScriptSigner` and `IntentBinder` in the application; Transit and the ring as adapters in `crates/infra-secrets`; telemetry stays in `crates/telemetry` |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect of a run; Transit calls are internal and idempotent per row |
| IV. Compatibility is versioned and pinned | Pass | `local` stays the default; the columns are additive; `transit` gated by the instance registry, the sandbox runners by their sightings; the way back is the `returning` passage, gated the same way; the contract changes are additive (Task 015-26) |
| V. Code, tenant boundary and secrets are isolated | Pass | Keys never leave OpenBao; derivation per tenant; a mount and policy per custody and a token per process kind and custody; fail closed on every read path; the signed script message keeps its tenant; no tenant label |
| VI. Observability and audit are part of the feature | Pass | All three signals through the Collector; OpenBao calls counted and traced; every command of Technical Plan 6 audited in its own transaction, each action mapped in design 040's table; the state row writable only by the schema-owning role |
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
| A returning mode | A customer can leave `transit` without downtime, with the same guards | Rollback unsupported: a one-way door for a security option slows its adoption |
| Three key ports instead of one | Each has a meaning under each authority; no stub adapter | One port with a local adapter that cannot wrap a key: stubs (`AGENTS.md` rule 11) |
| The requirement `key-authority-ring-absent-v1` and a hand-over window | A passage without downtime: local keys leave by a rolling restart after `finish-migration`, and the registry then refuses them | Closing the gate of any process with a local key under `transit`: every serving process closes at `finish-migration` |
| An insert-only table of script signatures | Re-signing without relaxing the immutability that the runner's trust rests on | A narrowed immutability trigger on `script_artifacts`: a security decision about the artifact bytes' only guard, with one more owner-role path to audit |
| A `SECURITY DEFINER` lock function called by the triggers | `FOR SHARE` needs the `UPDATE` privilege, which the runtime role must not hold | A column-level `UPDATE` grant to `sagat_app`, with the trigger as the only guard of the state row |
| A re-seal inventory | No passage finishes while another feature's sealed values remain | A list in the command's code, which a feature can forget and the database cannot check |

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

Two lanes that can run at the same time; where they touch the same file the order points below say which merges first.

- **Lane A, telemetry** (`crates/telemetry`, `apps/cli/src/telemetry.rs`, the telemetry start-up of the four servers
  in `apps/control-plane`, `apps/execution-engine`, `apps/sandbox-runner` and `apps/database-runner`,
  `infra/observability`, `infra/docker/compose.observability.yaml`, `.github/workflows/ci.yml`'s Collector step, the
  telemetry values of `infra/helm`): Tasks 015-2 to 015-9.
- **Lane B, keys** (`crates/domain/src/secrets.rs`, `crates/application` secret and script-signing paths,
  `crates/infra-secrets`, `crates/infra-postgres` migrations and stores, `crates/scheduler`, `crates/infra-connectors`,
  `crates/api-http`, `apps/control-plane` commands and wiring, `apps/execution-engine` wiring, `apps/sandbox-runner`
  trust refresh, every server's start for Task 015-25, `infra/openbao`, the OpenBao and key-authority values of
  `infra/helm`, the contracts, the error catalogue and the Studio): Tasks 015-10 to 015-21 and 015-23 to 015-35.
- Order points: Task 015-2's change to `redact.rs` (lane A) merges before Task 015-20's canary test (lane B). Task 015-3
  (lane A), which changes the public types of `sagat-telemetry` and the CLI, merges before Task 015-25 (lane B), which
  edits the start of the same four servers. Task 015-7 (lane A) needs the per-custody Agent and the `sagat/telemetry/`
  policy of Tasks 015-17 and 015-23 (lane B), so it merges after them; Task 015-35 (lane B) edits the same chart
  helpers and merges after Task 015-7. Task 015-26 adds design 040's mapping rows after design 040's mapping table is
  built. Task 015-22 closes both.

- [ ] T015-1 Write this design, decide its questions and record the delegate rows and the two architecture decisions.
  Refs: Requirement 015-001, Requirement 015-010
- [ ] T015-2 Extend `redact.rs` with the names and shapes of Requirement 015-007; add `LogSink` and the OTLP log export to
  `json_log::LogLayer`, with the trace and span ids read from `OtlpLayer`'s `Open` (made crate-visible), the bounded
  queue, drop counting and the severity floor; change `LogData` to optional ids and a `Severity` enumeration mapped to
  the OpenTelemetry severity numbers, and `encode_log` with it; unit tests for drop, scrub, correlation, a record with
  no span and each severity's number.
  Refs: Requirement 015-001, Requirement 015-002, Requirement 015-007
- [ ] T015-3 Read `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, `OTEL_LOGS_EXPORTER` (default `otlp`), `SAGAT_OTEL_LOGS_LEVEL` and
  `OTEL_EXPORTER_OTLP_HEADERS_FILE` in `OtlpConfig::from_lookup`; make `endpoint` the optional `traces_endpoint` and
  accept a logs-only configuration; add `HeaderSource` with the file's format, precedence, 8 KiB limit and reload of
  Technical Plan 1 (content digest checked every 30 seconds, and after a `401` at most every 5 seconds) and its counter;
  adapt `apps/cli/src/telemetry.rs`; refuse a header with a non-loopback `http` endpoint; mark header values sensitive;
  tests of every refusal, of the precedence, of a reload after a change and after a `401`, and of a bad file after a
  good one keeping the last good headers.
  Refs: Requirement 015-001, Requirement 015-006
- [ ] T015-4 Set the resource attributes of Requirement 015-005 on spans and logs from the sources of Technical Plan 1
  (the registry's instance id for the control plane and the engine, a random identifier for the others, `SAGAT_ENV`
  or `OTEL_RESOURCE_ATTRIBUTES` for the environment); refuse a tenant or namespace key in `OTEL_RESOURCE_ATTRIBUTES`; a
  test per process kind of the four attributes, and a test that no resource attribute names a tenant or namespace.
  Refs: Requirement 015-005
- [ ] T015-5 Write the production Collector configuration with TLS, bearer authentication, the memory limiter, one
  `otlphttp` exporter and the placeholders of Technical Plan 3, and the `transform` attribute filter generated from
  `redact.rs`; first prove against the pinned Collector that the statement form replaces a string and keeps a number
  (stop and return the design to review if not); the drift test and that behaviour test.
  Refs: Requirement 015-004
- [ ] T015-6 Add the `logs` and `metrics` pipelines and the Prometheus receiver (today's job names) to both
  configurations; in compose make the Collector the only scraper (`prometheusremotewrite`, Prometheus's
  `--web.enable-remote-write-receiver`, the platform jobs removed from `prometheus.yml`) and add the pinned Loki service
  and the Grafana data sources; extend the step `OpenTelemetry Collector validate` of `.github/workflows/ci.yml` to
  both files, with the placeholder environment and dummy certificate files.
  Refs: Requirement 015-003, Requirement 015-004
- [ ] T015-7 Add the chart values of Technical Plan 11 for telemetry (`telemetry.otlpEndpoint`, `telemetry.egress`,
  `telemetry.logsLevel`, `telemetry.headersSecret`, the Agent template for the Collector credential) to every process
  template, keep `databaseRunner.telemetry.*` as the deprecated form and refuse both forms together, with the
  NetworkPolicy egress to the Collector and ingress from it to the admin ports it scrapes; chart tests of each refusal.
  Refs: Requirement 015-001, Requirement 015-006
- [ ] T015-8 End to end on the compose observability profile: a run's trace in Jaeger, its metrics in Prometheus with
  `up{job="sagat-execution-engine"}` present once, and its log lines in Loki, each linked to its span by trace id and
  span id (Success criterion 015-001); the observability runbook updated, with the rule of one metrics path per store.
  Refs: Success criterion 015-001
- [ ] T015-9 Measure Non-functional requirement 015-001 with the Collector stopped, on a stated environment.
  Refs: Non-functional requirement 015-001
- [ ] T015-10 Add the ports of Technical Plan 4 (`EnvelopeCipher` in the domain; `EnvelopeKeys`, `ScriptSigner` and
  `IntentBinder` with boxed futures in the application), the three error kinds, `LocalScriptSigner` over today's key,
  `StoredValue::Transit(EnvelopeSealed)`; rename `SecretReader::resolve` to `resolve_secret` and add `resolve_secrets`;
  add the `AppError` variants of Technical Plan 12 and map every `resolve_secret` caller of Technical Plan 4 as that
  table says (the five sites that skip today corrected, each with a test seen failing first; inbound webhook
  verification answering a retryable `503`; the output scan failing the attempt on `Refused`); add the
  `secret-resolve-never-discards` rule to `scripts/check-architecture.py` with a negative fixture per forbidden pattern
  and a fixture proving an unrelated `.resolve(` is not matched, and its row, and the `transit` trigger's row, in
  `docs/engineering/architecture-principles.md`; delivery and the report path compare logical identity; the existing
  secret suites pass unchanged, and a test holds a delivery between its reads while a re-seal commits.
  Refs: Requirement 015-010, Requirement 015-011, Requirement 015-017
- [ ] T015-11 First prove against the pinned OpenBao 2.7.1 that `datakey` honours `key_version`, that `rotate`,
  `config` and `trim` take exactly the parameters of Requirement 015-016, that the key types and creation parameters of
  Technical Plan 7 are accepted (a derived `aes256-gcm96` key refuses a call without context; `hmac` with `key_size=32`
  gives a 32-byte binding) and that `keys/<name>` reports each version's creation time (stop and return the design to
  review if not).
  Then add `TransitEnvelopeKeys`, `TransitScriptSigner` and `TransitIntentBinder`: data keys with the derivation context
  (a golden test of its encoding), unwrap and batch unwrap, per-item rewrap, script signing over the full message,
  intent binding over metadata and a digest; the client per mount with cached lookup, per-attempt timeouts within the
  budget, the in-flight bounds per process and per tenant with their metrics, wiping types and typed errors; unit tests
  against a recorded fake.
  Refs: Requirement 015-011, Requirement 015-013, Requirement 015-014, Requirement 015-018
- [ ] T015-12 Write the migrations (`key_authority_state`, its owner-role functions not executable by `sagat_app` and
  its `ENABLE ALWAYS` trigger; the `SECURITY DEFINER` function `key_authority_share()` not executable by `sagat_app`,
  and `SECURITY DEFINER` trigger functions that call it; the `secret_versions` trigger on insert and on update of the
  sealed columns; `secrets.authority` widened; `wrapped_key`, `wrapped_key_version`, `data_key_issued_at` and the
  representation constraint with no key id on a `transit` row; the intent binding version and key version under the
  immutability trigger, and the binding-version trigger of Requirement 015-014; `script_artifact_signatures` with its
  constraints, its insert-only `ENABLE ALWAYS` triggers, its signer trigger and the copy of every existing artifact's
  signature, leaving `script_artifacts` and its immutability trigger untouched; `script_runner_trust_sightings`;
  `key_authority_reseal_inventory` with `secret_versions` as its first entry; `steps.lease_granted_at`; the entries of
  `storage-classification.json` of Technical Plan 5; a refusal when a row has `key_id = 'transit'`; a `down` that
  refuses while a `transit` row, a `transit` authority, a `transit` signature or a non-`local` mode exists) with the
  migration checklist and the privilege audit rows; `KeyRing::parse` refuses the id `transit`; a test runs the 0.2.x
  reader against a `transit` row and sees it refuse; a test as `sagat_app` seals a secret (proving the lock needs no
  `UPDATE` grant) and fails to call `key_authority_share()` directly; a barrier test holds a re-key between its data
  key and its commit while the floor rises and sees the commit refused, and one holds a seal's transaction open while
  `key_authority_set_mode` waits for it and then counts its row.
  Refs: Requirement 015-011, Requirement 015-013, Requirement 015-014
- [ ] T015-13 Add the unwrapped-key cache (4,096 entries of current versions, 512 per tenant, 300 seconds, a refresh
  every 10 seconds of entries older than 240 seconds, wiped on eviction, driven by the platform's clock) and its metrics,
  with the boundary tests of Success criterion 015-005.
  Refs: Requirement 015-017
- [ ] T015-14 Add the commands of Technical Plan 6 (`key-authority begin-migration`, `finish-migration`,
  `begin-return` and `finish-return`, `reseal-secrets --to transit` and `--to local`, `secrets rotate`,
  `advance-seal-version`, `rewrap`, `rekey`, `retire-key-versions`, `trim-key-versions`, and the rotate and retire
  commands of `sagat-scripts` and `sagat-intent`) with their guards (the snapshot attestation against Transit's
  creation time, the replay waits on the standbys of `pg_stat_replication` with `standbys: none` recorded when there
  is none, `N` at most `seal_version` for retirement, the recount of trim), each command's audit record in its own
  transaction, the three steps (started, Transit call, completed) of every command that changes Transit and the rerun
  that reconciles a `started` record, and the operator's token for the key changes; `reseal-secrets` over every
  `ResealTarget`, setting `secrets.authority`; tests of every refusal, of a crash between each Transit call and its
  outcome (the rerun does not rotate twice), of a retirement refused for `N` above `seal_version`, of a count that is
  not zero leaving the floor raised, and of a secret whose last version is re-sealed changing its authority.
  Refs: Requirement 015-012, Requirement 015-015
- [ ] T015-15 Sign scripts through Transit at a version published 5 minutes earlier; extend `/script-runner/signing-key`
  to list the versions at or above the minimum and the legacy local key until `scripts resign` reports zero; write each
  new signature as a row of `script_artifact_signatures` (signer, key id `transit-v<N>`, key version, the 64 raw bytes)
  and serve from `/script-runner/artifacts/{digest}` the newest trusted one with `signer` and `keyVersion`; the sandbox
  runner sends `Sagat-Runner-Trust` on every request, refreshes every 5 minutes and on an unknown version (a retryable
  refusal), verifies a `transit` signature by its key version and a `local` one by its key id, records the verifying
  key in its artifact cache and evicts on refresh; record the sightings in `script_runner_trust_sightings` and gate
  `begin-migration` on them and on the operator's attestation; add `scripts resign` as an insert; tests that a
  signature for another tenant or language is refused, that a retired version is refused without a restart, from the
  cache too (Success criterion 015-006), that `begin-migration` is refused after a request with no header or with
  `configured-v1`, that a rerun of `scripts resign` inserts nothing, and that an `UPDATE` of an artifact or a signature
  is refused for every role.
  Refs: Requirement 015-013, Success criterion 015-006
- [ ] T015-16 Bind feature 017's intents with Transit HMAC (`sha2-256`) under binding version 2; verify a replay against
  the stored version, outside the provider lock; verify version 1 with the file key until none is pending; change the
  chart's validation and environment; a test replays an intent across a rotation.
  Refs: Requirement 015-014
- [ ] T015-17 Extend the bootstrap scripts of `infra/openbao/` with Transit, the three keys and this design's policies,
  for the development OpenBao of `infra/docker/compose.database.yaml` and for Kubernetes, with the configured mount
  names validated, the keys created and configured with exactly the parameters of Technical Plan 7 and an existing key
  that differs refused, and the installation of every policy fragment of `infra/openbao/policies/<feature>/`; write
  `scripts/check-openbao-policies.py` (the exact allow-list test of Requirement 015-016, reading each `.allow.json`
  beside its policy, with negative fixtures for each refusal) and run it in the continuous integration workflow and in
  `/verify`; add the operator-only snapshot policy and job under `infra/openbao/operator-only/`, referenced from
  `infra/openbao/production-values.yaml`, which the checker verifies separately (exactly one `sys/` grant, bound by no
  platform role).
  Refs: Requirement 015-016, Requirement 015-019, Requirement 015-020
- [ ] T015-18 Add the capabilities `key-authority-transit-v1` (listed only with a Transit configuration),
  `key-authority-return-v1` (listed only with both the ring and Transit) and `key-authority-ring-absent-v1` (listed only
  with no local key) to the instance registry's closed capability list for the control plane and the execution engine;
  read the mode at registration and after every heartbeat and apply the table of Requirement 015-010 (a closed gate,
  ADR-0082, or the hand-over warning and its gauge); tests of each cell of that table, and the two rollout orders of
  Technical Plan 10 run under load with a rolling restart at each configuration step, no gate closing and no
  resolution failing, a process with the ring refused after `key-authority-ring-absent-v1` is recorded, and
  `begin-return` refused while it is.
  Refs: Requirement 015-010, Requirement 015-015
- [ ] T015-19 Integration tests on a real OpenBao: rotation, rewrap, recount and retire with no restart (Success
  criterion 015-002), cross-tenant unwrap refused (Success criterion 015-003), migration under load (Success criterion
  015-004), outage with the cache, the output scan, inbound webhook verification and the managed-database reader
  (Success criterion 015-005).
  Refs: Success criterion 015-002, Success criterion 015-003, Success criterion 015-004, Success criterion 015-005
- [ ] T015-20 The canary test: marked key material, tokens, contexts and wrapped keys, under ordinary and unlucky field
  names, never in a log, span, metric, audit record or error, with both authorities.
  Refs: Non-functional requirement 015-004
- [ ] T015-21 Measure Non-functional requirements 015-002 and 015-003 on a stated environment and write the dated report.
  Refs: Non-functional requirement 015-002, Non-functional requirement 015-003
- [ ] T015-22 Write the runbooks (`docs/runbooks/observability.md` for the Collector, `docs/runbooks/key-authority.md`
  for OpenBao: install, the operator-only snapshot job and key custody, rotate, rewrap, retire, rekey after a
  compromise, the two rollout orders of Technical Plan 10, outage, paired restore, and the role setup that grants the
  schema-owning role `pg_monitor`), the quickstart of CUJ 2 with the failure scenario of CUJ 3, the reference of the
  variables and chart values of Technical Plan 11 (written before the code of Tasks 015-29 and 015-35), and update
  `docs/traceability.md`, the roadmap row and the release notes (the logs export on by default, the KV outcome labels).
  Refs: Requirement 015-012, Requirement 015-015, Requirement 015-020
- [ ] T015-23 The Helm chart: one OpenBao Agent per custody where a pod needs more than one, roles per (process kind,
  custody), and the database runner's Collector header from `telemetry.headersSecret`; chart tests of each.
  Refs: Requirement 015-016
- [ ] T015-24 The paired restore test (Success criterion 015-007) in `integration_key_authority`.
  Refs: Requirement 015-020, Success criterion 015-007
- [ ] T015-25 Disable core dumps at the start of every server process through `rustix` (`RLIMIT_CORE`, and
  `PR_SET_DUMPABLE` on Linux), with a test that reads the process's state on Linux.
  Refs: Non-functional requirement 015-004
- [ ] T015-26 The contract changes: `Secret.provider` gains `transit` and `keyId` is null for `openbao` and `transit`
  (`specs/001-platform-foundation/contracts/openapi.yaml`), with `SecretView.provider` derived from the version's
  custody instead of its key id; `SigningKey` extended additively (`keyId` and `publicKey` stay the current key; `keys`
  lists every trusted version with its publication time, `minimumVersion` and the legacy local key); `ArtifactView`
  gains `signer` and `keyVersion`; the request header `Sagat-Runner-Trust` (a shared parameter on every
  `/script-runner/` operation); the three entries of Technical Plan 12 in `sdks/error-catalog.json` and its four JSON
  copies (`sdks/go/error-catalog.json`, `sdks/rust/src/error-catalog.json`,
  `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/error-catalog.json`,
  `sdks/java/sdk/src/main/resources/dev/sagatoctopusorchestrator/sdk/error-catalog.json`), with the TypeScript and
  Python copies generated by `sdks/typescript/scripts/sync-shared.mjs` and `sdks/python/scripts/sync_shared.py`; the
  new audit actions in design 040's mapping table; contract and conformance tests, the Studio types regenerated.
  Refs: Requirement 015-011, Requirement 015-013, Requirement 015-017
- [ ] T015-27 The custody routing of Requirement 015-011 in `put_secret` for every mode, with a test of each cell of its
  table (the field rules included, and the cells with no provider configured).
  Refs: Requirement 015-011
- [ ] T015-28 The Studio's Secrets form (`frontend/apps/studio/src/features/secrets/`): send `requiredProvider: openbao`
  only when "Keep in OpenBao KV" is chosen (offered only when the provider is configured, preselected for a secret of
  `openbao` custody), accept `local`, `transit` and `openbao` in the answer and show the custody in the list; the
  connection forms keep sending it; component tests of each choice and of each custody in the answer, and the
  feature's `README.md` updated.
  Refs: Requirement 015-011
- [ ] T015-29 Wire the configuration of Technical Plan 11 into `apps/control-plane` and `apps/execution-engine`: parse
  and validate each variable (a configuration error for each bad value, and for the operator's token file on a serving
  process), build one Transit client per mount and the three adapters, and hand them to `SecretService`,
  `ManagedSecretService`, the `WorkerService` vault, the scheduler's deliveries, the events service, the connector
  supervisor and the script service; tests of each refusal and a test per holder that a `transit` row is opened
  through Transit when the configuration is present and is `Unavailable` when it is not.
  Refs: Requirement 015-010, Requirement 015-011, Requirement 015-017
- [ ] T015-30 Limit the output scan to the versions an attempt could have received: set `steps.lease_granted_at` in
  both lease producers, add `secret_versions_since` to both stores, scan through `SecretReader::resolve_secrets` with
  one `unwrap_batch` per report; tests that a report with five uncached `transit` secrets makes one Transit call, that
  a version older than the lease start is not opened, that a lease granted before the upgrade scans every version, and
  that a version created during the attempt is scanned.
  Refs: Requirement 015-017
- [ ] T015-31 Accept `transit` as a source of feature 017's `migrate`: the predicate of
  `crates/infra-postgres/src/managed_secrets.rs` and the full `SecretReader` in `ManagedSecretService` instead of
  `SecretReader::local`; a test moves a `transit` secret to KV and opens it there, and one refuses the move with
  Transit unavailable, writing nothing.
  Refs: Requirement 015-011
- [ ] T015-32 Add feature 032's standby regions to `advance-seal-version` (`--standby-restored-at <region>=<time>` for
  every region of feature 032's list), `rekey` and `retire-key-versions` (each region's replay past the position),
  once feature 032 provides its list; tests with two standby regions, one lagging.
  Refs: Requirement 015-012
  Owner-gate: OD-2026-11-27-150
- [ ] T015-33 Move the KV client's outcomes to the one set of Requirement 015-018 (`crates/infra-secrets/src/openbao.rs`);
  add the alert rules of Technical Plan 9 with their promtool cases in `infra/observability/tests/key_authority_test.yaml`,
  the gauges `secret_versions_by_custody` (sampled every 60 seconds by the control plane's metrics upkeep) and
  `key_authority_mode`, and the dashboard row; tests of each outcome's label.
  Refs: Requirement 015-017, Requirement 015-018
- [ ] T015-34 Build the re-seal inventory of Technical Plan 5: the trait `ResealTarget` in
  `crates/application/src/key_authority.rs`, the `secret_versions` entry (its count function, registration and target),
  the sum of every registered count in `key_authority_set_mode`, and the test that every `ResealTarget` has a registered
  count and the reverse; a test registers a second entry with one `local` value left and sees `finish-migration`
  refused.
  Refs: Requirement 015-015
- [ ] T015-35 The chart's key-authority values of Technical Plan 11 (`openbao.transit.*`, `keyAuthority.localKeys`,
  `networkPolicy.customerKeyServices` with the helper `sagat.networkPolicy.validateCustomerKeyServices`) and their
  validation, rendered into the control plane and the engine; chart tests of each refusal, including an egress peer
  that overlaps a customer key service by one address and an IPv6 block.
  Refs: Requirement 015-010, Requirement 015-021

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the three
checkers, the unit tests of the crates it changes and the suites it touches, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`
where it adds a statement. A test is seen failing for the right reason before the code that passes it.

- Lane A: `cargo test -p sagat-telemetry --locked`; the Collector validation tests; the attribute-filter drift test; the
  end-to-end check of Task 015-8 on the compose observability profile; a hand mutation (send a log record without its
  span id) that the correlation test catches.
- Lane B: `cargo test -p sagat-infra-secrets --locked`; `python3 scripts/check-openbao-policies.py`; the chart's
  contract test (`contract_scalability_ha`) with the refusals of Tasks 015-7 and 015-35; the Studio's
  `npm --prefix frontend test` for Task 015-28; a new target `integration_key_authority` that starts its own
  OpenBao container (rule 16 of `AGENTS.md`); the existing `integration_database_connections` unchanged under both
  authorities; the migrations' up, down and privilege audit; hand mutations (unwrap without the tenant context; put the
  Transit version into the associated data; sign a bare digest) that the cross-tenant, rewrap and signature tests catch.
