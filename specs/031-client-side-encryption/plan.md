# Implementation Plan: Client-side payload encryption

**Branch**: `feature/031-client-side-encryption` | **Date**: 2026-10-26 | **Spec**: [spec.md](spec.md)

**Input**: [spec.md](spec.md), [research.md](research.md), decision
[ADR-0066](../../docs/adr/ADR-0066-client-side-payload-encryption.md) (Proposed).

Status: Proposed — not started.

## Summary

Make encoding a declared, pinned property of the workflow's carrier schemas; check it at publication against every
server-evaluated expression and every consumer feature; validate envelopes structurally on every write path and refuse
plaintext without storing it; give clients and workers a plan of what to encode and decode; implement envelope v1,
codecs and key providers in the five SDKs under one conformance suite; bind every envelope to its tenant, namespace
and location and keep envelopes exactly where declared, across documents too; keep the cross-draft registry of
encoded-field positions; let the Studio and the CLI decode through a customer codec server that authenticates the
person with the customer's own identity provider, the platform assertion only narrowing; disclose envelopes under the
016 levels; enable the feature only when the whole fleet can enforce it; and offer a key-usage inventory. The server
never holds a codec key, never decodes and never calls a codec server.

## Technical Context

**Language/Version**: Rust 1.98.0 (`rust-toolchain.toml`); the locked TypeScript, Go, Python, .NET and Java toolchains;
React/TypeScript for the Studio; Go for the reference codec server example. No toolchain change.

**Primary Dependencies**: server — existing `serde_json` (adapters only), `jsonschema` 0.58 (`crates/infra-schema`),
`p256` (already a dependency of `crates/api-http`; added to `crates/infra-secrets` for the ES256 signer adapter), SQLx, Axum, tonic. No cipher or
decompression API in the feature's server modules (ADR-0066). SDKs — each language's standard AES-GCM implementation
(Node `crypto`, Go `crypto/cipher`, Python `cryptography`, .NET `System.Security.Cryptography.AesGcm`, Java
`javax.crypto` AES/GCM), a zstd and a gzip codec per language, pinned in the existing lockfiles; versions are selected and
pinned in T031-040 after a licence and advisory check, not here.

**Storage**: PostgreSQL. One column on `workflow_versions`, two on `worker_sessions`, one on `event_deliveries`, four new
tables ([data-model.md](data-model.md)). No change to `payloads`, `run_events`, `runs`, `blobs`.

**Testing**: domain unit/property tests; adapter tests over a defect corpus and envelope vectors; real PostgreSQL
integration, fault and runtime-role suites; REST/proto/IR contract tests; five-SDK conformance (vectors, 5×5 interop,
runtime, faults, gating); codec-server black-box suite; Studio component tests and a real Chromium journey; canary suite;
a benchmark.

**Target Platform**: existing single-region Linux containers and Compose; customer-run KMS and codec servers.

**Project Type**: modular monolith, five SDKs, CLI, web Studio.

**Performance Goals**: bound, then measure (NFR-031-PERF). No latency or throughput promise.

**Constraints**: domain free of serde_json/crypto; `begin_safely` for every new transaction (ADR-0014); audit in the
decision's transaction; namespace admission charged once (ADR-0027); payload-free telemetry (ADR-0017, D1549).

**Scale/Scope**: 8 user stories, 36 FR, 7 NFR, 14 SC; 10 REST operations and one operator command, 1 proto minor,
1 permission, 9 audit actions, 7 tables, 5 columns, 1 cross-draft registry.

## Architecture and crate placement

| Concern | Placement |
|---|---|
| Encoding, pointer, plan, envelope verdict, audience, scan state | `crates/domain/src/payload_encoding.rs` (new) |
| Failure codes, permission, audit actions | `crates/domain/src/execution/event.rs`, `crates/domain/src/identity.rs`, `crates/domain/src/audit.rs` |
| Ports and use cases: plan service, envelope check port, endpoint service, assertion service, scan service | `crates/application/src/payload_encoding.rs`, `crates/application/src/codec_assertions.rs` (new) |
| Write-path hooks (location rule, `ctx`, undeclared-envelope walk) | `crates/application/src/runs.rs` (start, child admission with source-plan mapping), `crates/application/src/worker/output.rs` (completion), `crates/application/src/execution.rs` (end, continueAsNew), `crates/application/src/events.rs` (signals, inbox match), `crates/application/src/worker.rs` (heartbeat side channel when 019 lands); the other drafts' write paths call the same `EnvelopeCheck` port |
| Registry | `specs/031-client-side-encryption/contracts/encoded-field-registry.md`, moved into `docs/reference/workflow-document.md` at merge; one fixture per row |
| Fleet activation | `apps/control-plane/src/cli.rs` (`payload-encoding activate|deactivate`) over 018's instance-capability registry (registration, heartbeat and activation transaction are 018's); the capability value `payload-encoding-v1` in both binaries; gauge in `crates/telemetry/src/metrics.rs` |
| Lease plan, claim gating | `crates/application/src/worker.rs`, `crates/infra-postgres/src/workers.rs`, `crates/infra-postgres/src/execution.rs` |
| Expression access paths | `crates/infra-expr/src/paths.rs` (new), exported beside `Compiled::variables` |
| Publication check, plan derivation, envelope parser | `crates/infra-ir/src/encoding_check.rs`, `crates/infra-ir/src/envelope.rs` (new); wired into `validated_document` of `crates/infra-ir/src/lib.rs` and exposed through the `IrCodec` port as `encoding_analysis`, beside `classification_analysis` |
| Encoded view for disclosure | `crates/infra-schema/src/classification/walk.rs`, `crates/application/src/payload_disclosure.rs` |
| Stores | `crates/infra-postgres/src/payload_encoding.rs` (new), migration, `storage-classification.json` |
| REST | `crates/api-http/src/routes/payload_encoding.rs` (new), `crates/api-http/src/studio.rs` (CSP), `crates/application/src/api_admission.rs` (new operations) |
| gRPC | `crates/api-grpc/src/service.rs`, `crates/api-grpc/src/blobs.rs` (envelope header check while streaming) |
| ES256 signer | `crates/infra-secrets/src/codec_signer.rs` (new adapter behind a `CodecAssertionSigner` port; key from `SAGAT_CODEC_ASSERTION_KEYS`) |
| Scan loop | `crates/scheduler/src/passes.rs` (a leased pass, `engine_leases` pattern) |
| CLI | `apps/cli/src/workflow.rs` (`inspect --decode`), `apps/cli/src/codec.rs` (new, HTTP to the codec server) |
| Studio | `frontend/apps/studio/src/features/codec/` (new), `frontend/apps/studio/src/features/runs/PayloadDisclosure.tsx`, `StartRun.tsx`, `SendSignal.tsx` |
| SDKs | `sdks/typescript/src/codec/`, `sdks/go/codec/`, `sdks/python/sagat_octopus_orchestrator/codec/`, `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Codecs/`, `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/codec/` |
| Reference codec server | `examples/codec-server-go/` (example, O1522) |

`scripts/check-architecture.py` gains a source rule: the feature's server modules listed above import no cipher or
decompression API. No empty mapper: the wire types of the new routes are the one necessary boundary.

## Transaction boundaries (what commits together)

| Operation | One transaction | Outside it |
|---|---|---|
| Publish | version row with `encoding_rules`, `workflow_version_encodings` rows, pointer history, audit (ADR-0018) | plan derivation and checks run before the transaction on the immutable document |
| REST start | envelope checks (pure, before writes) → run, payload, journal, initial tasks, outbox, idempotency receipt with the header fingerprint (D1535), audit | none |
| Completion | fence check, envelope checks, output payload or refusal failure, journal, next tasks/timers, outbox, receipt | the refused bytes are dropped in memory; nothing is written for them |
| Child admission, `end`, `continueAsNew` | existing transitions with the checks inside (the source-plan mapping of D1557 is pure and runs on the pinned plans before any child row is written); a refusal commits the parent failure in the same transition | none |
| Signal (`signalRun`, `signalExecutionChain`) | check against each woken wait's plan before admission; refusal = no write; receipt fingerprint per FR-031-013 | none |
| Side channel (019 checkpoint, 026 stream append) | lease renewal commits regardless; the checkpoint or stream row is stored only when its checks pass (same transaction as today's side-channel write) | none |
| Activation | 018's activation transaction (epoch lock, live-instance check, requirement row, epoch bump) plus this feature's activation row and `payload.encoding_activated` audit | none |
| Inbox match | delivery row (with `refusal_code` when refused) in the dispatcher's existing transaction (ADR-0012) | none |
| Endpoint PUT/DELETE | row (revision CAS, ceiling and setter), grant check for a `pii` ceiling, and `codec.endpoint_*` audit | none |
| Assertion | authorization reads (membership, grant, endpoint, the `pii` ceiling setter's current grant) and `codec.assertion_issued` audit commit | signing after commit; a crash after commit leaves an audit record and no token (acceptable: audit precedes disclosure) |
| Scan batch | scanned rows' aggregates upserted, cursor and counters advanced, lease renewed | reading payload rows happens inside the same short transaction (bounded batch) |
| Scan start/cancel | scan row, retention trim, audit | none |

The run lock order of features 002/014/017 is unchanged: envelope checks take no lock and do no I/O.

## Authorization matrix

| Operation | viewer | developer | operator | admin | worker / runner / database-runner |
|---|---|---|---|---|---|
| `getPayloadPlan` (`definition:read` or `run:start`) | yes | yes | yes | yes | — |
| `getCodecEndpoint` | yes (`run:read`) | yes | yes | yes | — |
| `put/deleteCodecEndpoint` (`codec:manage`) | — | — | — | yes (`levelCeiling: pii` also needs the reveal grant) | — |
| assertion `decode` (`run:read`) | yes (max `public`) | yes (max `internal`) | yes (max `internal`) | yes (max `internal`) | — |
| assertion `encode` (`run:start`) | — | yes | yes | yes | — |
| assertion with `reveal: ["pii"]` | — | grant + read-internal | grant + read-internal | grant + read-internal | — |
| disclosure with `includeEnvelopes` | 016 matrix | 016 matrix | 016 matrix | 016 matrix | — |
| key-usage scans (`codec:manage`) | — | — | — | yes | — |
| lease with `payload_plan` | — | — | — | — | yes worker with codec capability; never `database-runner` or `runner` for planned output |
| `payload-encoding activate/deactivate` | operator command only (deployment credentials), audited | | | | |

An assertion never authorizes a decode by itself: the customer's codec server authenticates the person and intersects
its own policy with the assertion (D1558), so platform roles bound what the platform helps a person request, not what
they can decrypt. Foreign resources are the opaque `404`; denials use the existing restricted denial audit. A grant
revoked between issuance and use cannot recall an issued assertion; its lifetime (default 60 s, at most 300 s) and its
single use bound that window (stated in the runbook).

## Finite ceilings

| Resource | Default | Hard ceiling |
|---|---:|---:|
| Encoded locations per carrier | 32 | 64 |
| `*` segments / segments per location | 2 / 16 | 4 / 16 |
| Envelopes validated per payload | 1,024 | 1,024 |
| Envelope header bytes | 2 KiB | 2 KiB |
| `kid` / `kv` / `wk` | 128 B / 64 B / 1 KiB | same |
| Inline encoded payload | 256 KiB (existing) | 256 KiB |
| Blob header prefix parsed | 4 KiB | 4 KiB |
| SDK decoded bytes per envelope | 4 MiB | 64 MiB |
| SDK data-key reuse: uses / age | 10,000 / 5 min | 1,048,576 / 15 min |
| SDK unwrap cache: entries / TTL | 256 / 5 min | 1,024 / 15 min |
| Codec endpoints per namespace | 1 | 1 |
| Operator codec origins | 4 | 16 |
| Assertion lifetime (deployment setting `SAGAT_CODEC_ASSERTION_TTL_SECS`; no request override) | 60 s | 300 s |
| Padding bucket (compressed plaintext) | next power of two ≥ 256 B up to 64 KiB, then 64 KiB multiples | same |
| Assertions per subject per namespace | 30 / min | 120 / min |
| Codec request: items / bytes (Studio, CLI) | 32 / 1 MiB | 64 / 4 MiB |
| Codec response bytes | 4 MiB | 16 MiB |
| Codec call timeout | 10 s | 30 s |
| Envelopes decoded per Studio view | 256 | 1,024 |
| Disclosure envelope bytes per response | 256 KiB | 256 KiB (response 320 KiB) |
| Key-usage scans active per namespace / tenant | 1 / 2 | 1 / 4 |
| Scan batch runs / payloads | 100 / 500 | 1,000 / 2,000 |
| Scan payloads per job | 1,000,000 | 10,000,000 |
| Scan result rows | 1,024 | 1,024 |
| Retained scans per namespace | 50 | 50 |

Effective value = min(request, namespace setting, deployment ceiling); namespace settings use the existing quota
resources of 009/ADR-0027 (new resources `codec_assertions_per_minute`, `key_usage_scans_active`); zero refuses.
Encoded bytes count against `tenant_content_bytes` and the run's 4 MiB. Defaults are design values to measure (T031-062).

## Failure and recovery matrix

| Failure | Expected behaviour | Test |
|---|---|---|
| Plaintext at an encoded location (any path) | Refused with its reason; not persisted (inbox row at admission and unswept uploads excepted) | T031-022, T031-023 |
| Crash before/after start commit | No run / replay returns the run; re-encoded retry replays (D1535) | T031-029 |
| Crash between worker encode and report | Lease expires; new attempt re-encodes; one output commits (fencing) | T031-029 |
| Lost `CompleteStep` response | Retry with identical bytes → `already_applied` | T031-029 |
| Duplicate start with changed plaintext under same key | Same header → original run (G1526); different `kid` or level → `409` | T031-029 |
| Lost broker hint | Reconciler re-emits; gated task still only to codec sessions | T031-030 |
| Timeout vs signal with encoded payload | First commit wins; envelope never inspected; loser audited | T031-030 |
| KMS unavailable | Worker `PAYLOAD_CODEC_UNAVAILABLE` retryable; client raises before sending | T031-042 |
| Key version disabled / unknown `kid` | `PAYLOAD_KEY_UNAVAILABLE`, step fails non-retryably | T031-042 |
| Compensation cannot decode | Compensation fails → existing retry, then manual review | T031-043 |
| Codec server down / 401 / CORS / timeout | Studio and CLI show the state; platform unaffected | T031-052, T031-054, T031-057 |
| Assertion audit insert fails | No token returned, `503` | T031-047 |
| Signing key missing | Assertion endpoint `503`; Studio "codec not available" | T031-047 |
| Engine crash during scan batch | Lease expires, another engine resumes at the committed cursor; no double count | T031-058 |
| Older/no-codec worker only | Planned task stays `READY`, attempts unchanged | T031-030 |
| Engine restart mid-run with envelopes | No change; journal replay equals projection | T031-030 |
| Quota exhausted by envelope growth | Existing `QUOTA_EXCEEDED` / `STORAGE_QUOTA_EXCEEDED` paths | T031-031 |
| Envelope carried into a child (rules 0, or rules 1 without the location), a pin, a reset into another version or a 030 handler | Parent step fails `ENCODING_CARRIAGE_MISMATCH`; no child or handler run | T031-066 |
| Crash between the carriage mapping check and the admission commit | Nothing committed; the retry repeats the check | T031-066 |
| Envelope pasted from another namespace or location | `ENVELOPE_CONTEXT_MISMATCH` at write; decoders refuse it too | T031-066 |
| Plaintext checkpoint or stream event from a node with encoded locations | Refused `ENCODING_REQUIRED`; lease still renewed | T031-067 |
| Platform-minted or replayed assertion without a customer session | Codec server `401`; nothing decoded | T031-048 |
| `pii` ceiling setter loses the grant | `pii` assertions refused `CODEC_CEILING_UNCONFIRMED`; endpoint shown unconfirmed | T031-049 |
| Rolling upgrade with an incapable replica | Activation refused; publication with encoding refused; gauge and alert | T031-068 |
| Long KMS outage | Hosts stop claiming planned tasks; claimed tasks retry under their budget; runbook: pause runs | T031-042 |
| Re-encoded signal retry after client restart | Header (and content) fingerprint replays the original outcome | T031-029 |

## Observability

Spans (closed attributes; no value, pointer value, `kid`): `payload.encoding.validate` (`sagat.carrier`, `sagat.outcome`),
`payload.encoding.plan`, `codec.assertion.issue` (`sagat.purpose`, `sagat.max_level`, `sagat.outcome`),
`codec.endpoint.change`, `payload.key_usage.scan_batch`. SDK spans `sagat.codec.encode` / `sagat.codec.decode` through
each SDK's tracing hook (C#: `ActivitySource` `SagatOctopusOrchestrator.Sdk`).

Metrics (labels from closed sets only):

| Metric | Labels |
|---|---|
| `payload_envelope_checks_total` | `carrier` (input, output, node, heartbeat, stream), `outcome` (accepted, encoding_required, envelope_invalid, level_mismatch, context_mismatch, carriage_mismatch) |
| `payload_encoding_incapable_members` (gauge, live `serving_instances` without the capability) | `kind` (engine, control_plane) |
| `payload_envelope_check_duration_seconds` | `carrier` |
| `publication_encoding_findings_total` | `code` (closed set of contracts/workflow-encoding.md) |
| `codec_assertions_total` | `purpose`, `max_level`, `outcome` (issued, denied, ceiling_unconfirmed, unavailable) |
| `codec_endpoint_changes_total` | `action` (set, removed) |
| `payload_key_usage_scans_total` | `outcome` |
| `worker_leases_withheld_for_codec_total` | `reason` (minor, capability) |

Audit: `codec.endpoint_set`, `codec.endpoint_removed`, `codec.assertion_issued`, `codec.assertion_denied`,
`payload.key_usage_scan_started`, `payload.key_usage_scan_finished`, `run.encoding_refused`, `payload.encoding_activated`,
`payload.encoding_deactivated`; reasons are server-built closed strings. Alerts: a platform rule fires while
`payload_encoding_incapable_members > 0` after activation; a rule on a sustained rise of `envelope_invalid` or
`context_mismatch` is documented in the runbook as optional. Logs: refusals log code, carrier and run id only.

## Migration, compatibility and rollout

- Additive migration; down refuses when encoded versions or rows exist (data-model.md §4).
- Contracts additive (contracts/README.md). Older Studio: never receives `encoded` nodes (opt-in). Older SDK: ignores
  `payload_plan` (it is never leased planned output) and `encodingRules`. Older server: ignores the annotation (rules 0
  semantics) — a deployment rolled back below this feature MUST NOT run versions published with rules 1; the rollback
  procedure disables publication with rules 1 first and drains such runs (ADR-0032 point 8 pattern).
- Rolling upgrade (architecture review S3): every engine and control-plane process registers its capabilities in
  018's instance-capability registry (FR-018-052); publication with encoding is refused (`ENCODING_NOT_ACTIVATED`) until
  the audited operator command `payload-encoding activate` succeeds, which requires every live instance of both kinds
  to list `payload-encoding-v1` and records the requirement; afterwards a registered instance without it does not serve.
  Validation of already published rules-1 versions never depends on activation. A binary older than 018's registry does
  not register, so it is neither blocked nor seen (G1537); the runbook forbids starting one. No per-process environment
  flag decides enablement.
- Assertion keys are entries of 032's cell key inventory (C11); the Studio CSP change is recorded in ADR-0062's combined
  Studio-origin refinement (C15).
- No data migration; no rewrite of payloads.

## Constitution Check

Design checks, not implementation approvals or test results.

| Principle | Before research | After design | Evidence / future gate |
|---|---|---|---|
| I — durable atomic transitions | Pass | Pass | Checks run inside existing transitions; refusals reuse failure paths; scan batches commit progress with aggregates. T031-024..030, T031-058. |
| II — pure domain, inward dependencies | Pass | Pass | Domain types pure; parsers, walks and signer are adapters behind ports; architecture source rule. T031-011, T031-059. |
| III — at-least-once effects | Pass | Pass | No new external effect from the server; SDK KMS calls are customer effects; reports reuse idempotency with identical bytes. T031-029. |
| IV — pinned, compatible | Pass | Pass | `encoding_rules` pinned; envelope v1 permanent; additive IR/OpenAPI/proto; five-SDK conformance. T031-005..009, T031-033..039. |
| V — isolation of code, tenant and secrets | Pass | Pass | No codec key on the platform; no decode proxy; the customer's identity provider authenticates every decode (the assertion only narrows); envelopes bound to tenant, namespace and location; tenant-scoped tables; canary and carriage suites release-blocking. T031-023, T031-048, T031-066. |
| VI — observability and audit | Pass | Pass | Spans, closed metrics, seven audit actions, typed errors without values. T031-032. |
| VII — inbox/outbox | Pass | Pass | Inbox match records refusals as deliveries; no outbox payload change. T031-027. |
| VIII — failure tested, honest | Pass | Pass | Rule-9 matrix above, key rotation, codec-server outage, carriage across documents and rolling upgrade included; vectors reviewed before goldens. T031-004, T031-029..031, T031-042..043, T031-054, T031-066..068. |

**Governance gate**: ADR-0066 (Proposed) must be accepted by the owner before implementation; O1520–O1522 answered.

## Complexity Tracking

| Choice | Why needed | Simpler alternative rejected because |
|---|---|---|
| Expression access-path analysis | The engine must refuse what it cannot read at publication | Runtime-only failure moves errors to production and can derive keys from ciphertext |
| Lease plan plus claim gating | Workers must know what to encode; plaintext must never be produced for a planned output | Walking payloads for envelope shapes is ambiguous (forged shapes) and costs every lease |
| Platform-signed assertion in addition to the customer session | Codec servers need the tenant, namespace and level the platform allowed, which a customer session alone does not know | Assertion alone lets the operator decrypt (ADR-0066 option 5); passing the access token exposes a platform credential (option 6) |
| `ctx` binding and the location rule | v1 is permanent; transplants across namespaces or fields must fail | A later v2 would leave v1 writable forever (ADR-0066 option 7) |
| Fleet activation with member capabilities | An older replica would accept plaintext at declared locations | A per-process flag cannot see the other replicas |
| Key-usage scan job | Rotation is forward-only; operators need evidence | Write-path counters would serialize completions on one row per key |

## Delivery sequence

1. Contracts, domain, migration (T031-001..015).
2. US01 publication and plan (T031-016..021); US02 write-path enforcement, gating, lease plan, SDKs and conformance (T031-022..039), carriage, side channels and activation (T031-066..068).
3. US03 rotation and failure suites (T031-040..044).
4. US05 disclosure interplay and assertions (T031-045..048); US04 codec endpoint, reference codec server, Studio (T031-049..054).
5. US06 compression and blob path (T031-055..056); US07 CLI (T031-057); US08 scan (T031-058).
6. Gates: architecture rule, compose overlay, docs, benchmark, demo, verification, traceability (T031-059..065, T031-069).

## Verification plan

Each checkpoint records its commands and results in a `verification.md` of this feature; nothing is ticked without them.
Required evidence: SC-031-001..014. Commands follow AGENTS.md: the new targets
(`cargo test -p sagat-tests --test integration_client_side_encryption`, `fault_…`, `contract_…`, `e2e_…`,
`perf_…`), `SAGAT_TEST_AS_RUNTIME_ROLE=1` reruns, `SAGAT_SDK_SCENARIO=payload-encryption… cargo test -p sagat-tests --test
sdk_conformance --locked -- --ignored`, the codec-server black-box suite against `examples/codec-server-go/`, Studio
`npm run check:api` and the Chromium journey, `python3 scripts/check-architecture.py`. Real OpenBao Transit runs in a
disposable container started by the test fixture (AGENTS.md rule 16), pinned like the 017 fixture.

## Project Structure

```text
specs/031-client-side-encryption/
├── spec.md  research.md  plan.md  data-model.md  quickstart.md  tasks.md
├── checklists/requirements.md
└── contracts/  README.md  envelope.md  envelope.schema.json  workflow-encoding.md  openapi-fragment.yaml
                codec-server.openapi.yaml  codec-assertion.md  worker-protocol.md  sdk-codec.md  errors.md
docs/adr/ADR-0066-client-side-payload-encryption.md
```
