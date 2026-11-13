# Research: client-side payload encryption

Baseline investigation: 2026-10-26, worktree `feature/018-032-parity-specs` at `70f09435` (release 0.2.0).
Documentation only; nothing below is implemented. Decisions use D1520–D1559, gaps G1520–G1559 and owner
questions O1520–O1559 (the range allocated to feature 031), plus the overflow D1665–D1669 / G1665–G1669 the
coordinator allocated for the review round of 2026-10-27.

## Baseline verified in this repository

- **Payload storage.** `payloads` is content-addressed by `(tenant_id, digest)`, `content bytea`, at most
  262,144 bytes (`crates/infra-postgres/migrations/20261009000100_execution.up.sql`). Runs, journal events and
  leases reference payloads by digest. A run retains at most 4 MiB of step outputs and signal payloads
  (`docs/architecture/worker-runtime-protocol.md`, FR-002-008). Larger values use the blob claim-check of
  ADR-0023 (`{"$blob": {...}}` descriptors, `WriteBlob`/`ReadBlob`, 64 MiB per blob).
- **The engine reads payloads.** Expressions (`crates/infra-expr/src/eval.rs`, `Compiled::variables` returns
  top-level variables only) are evaluated server-side in `condition`, `switch` cases, `forEach`, `while`,
  `repeatUntil`, `child.input`, `end.outputExpr`, `return.result`, `continueAsNew`, `waitEvent.correlationExpr`,
  database parameters, declared state and Query/Update (`specs/001-platform-foundation/contracts/workflow.schema.json`).
  An activity receives the whole step document `{run, node, input, steps}` (`sdks/SPEC.md` §6.3).
- **Classification exists** (feature 016, ADR-0033): `x-sagat-classification` on the carriers `inputSchema`,
  `outputSchema` and the node `outputSchema` of `activity`, `script`, `waitEvent`, `child`; the walker is
  `crates/infra-schema/src/classification/walk.rs`; the interpretation is pinned by
  `workflow_versions.classification_rules` (`20261201000000_payload_classification.up.sql`); disclosure is
  `POST /v1/runs/{runId}/payload-disclosures` (`crates/application/src/payload_disclosure.rs`). G279 records that
  payload content is not encrypted; G275 that a guessable payload can be confirmed from its digest; G276 that
  there is no taint tracking; G282 that events have no schema carrier.
- **Secrets.** Platform secrets are references resolved per attempt (`ResolveSecrets`); a completion that
  contains a resolved value fails with `SECRET_IN_OUTPUT` (`crates/application/src/worker/output.rs`,
  `crates/domain/src/execution/event.rs`). Feature 017 / ADR-0035 integrated OpenBao KV v2 for credentials
  (`crates/infra-secrets/src/openbao.rs`). The platform-wide OpenBao/Transit draft (roadmap feature 015) lives in the
  sibling worktree `sagat-octopus-orchestrator-013-otel-openbao`, `specs/013-otel-openbao/spec.md` (FR-013-008:
  Transit for platform keys); it is unmerged and not a dependency of this feature.
- **Studio.** Same-origin app (ADR-0016) with a static CSP built in `crates/api-http/src/studio.rs` (`policy`):
  `connect-src 'self'` plus the OIDC issuer origin only. Payload values are shown by
  `frontend/apps/studio/src/features/runs/PayloadDisclosure.tsx`.
- **Signing.** The control plane signs script artifacts with Ed25519 (`crates/application/src/script_signing.rs`,
  `SAGAT_SCRIPT_SIGNING_KEY`); `p256` is already a dependency of `crates/api-http`.
- **Worker protocol.** Minor 1.6 is current (database runner); `TaskLease` uses fields 1–18 and
  `WorkerCapabilities` fields 1–5 at the base commit (`specs/001-platform-foundation/contracts/worker.proto`).
  The server gates leases by negotiated minor (1.3 blob gating, `accepted_protocol_minor` on `worker_sessions`).
- **SDKs.** Five SDKs with builders that emit RFC 8785 canonical JSON and one conformance suite
  (`tests/contract/sdk-conformance/`, ADR-0015); a `classification` scenario already proves byte-identical
  annotated schemas. There is no codec, data converter or interceptor today; interceptors are feature 028.
- **Not found:** any encryption of payload values, any codec, any envelope format, any codec-server concept.

## External semantics consulted

- Temporal Payload Codec — <https://docs.temporal.io/payload-codec>: a codec is a bytes-to-bytes transformation on
  a list of payloads, applied after the payload converter when serializing and in reverse when deserializing;
  AES-GCM-256 samples in Go, Java, Python and TypeScript.
- Temporal Codec Server — <https://docs.temporal.io/codec-server>: `POST /encode` and `POST /decode` with
  `{"payloads": [...]}`; `X-Namespace` header; the Web UI either passes the user's access token or relies on the
  codec server's own cookie session ("include cross-origin credentials"); "Anyone who can call it has effective
  decrypt access."
- Temporal data encryption — <https://docs.temporal.io/production-deployment/data-encryption>: CORS must allow
  the UI origin, `POST, GET, OPTIONS` and `Authorization`; failure messages are not encoded by default; browsers
  may block private-network codec servers.
- Temporal Cloud namespace codec settings — <https://docs.temporal.io/cloud/tcld/namespace>: one HTTPS endpoint per
  namespace with `pass-access-token` and `include-credentials` flags, both off by default.
- OpenBao Transit — <https://openbao.org/docs/secrets/transit/>: encryption as a service that does not store the
  data; key versions with `min_decryption_version`; ciphertext prefixed `vault:vN:`; data-key generation for
  envelope encryption; AES-GCM keys should rotate before about 2^32 encryptions per version.

What differs here: Temporal's server never interprets payloads because workflow code runs in workers; this
engine evaluates a declarative IR server-side, so encryption must be **declared** for publication to refuse
expressions the engine could never evaluate. Temporal concepts tied to code replay (deterministic data
converters inside the workflow sandbox) do not apply.

## Decisions

- **D1520 — Declared, plan-driven encoding.** A carrier schema location is marked
  `"x-sagat-encoding": "encrypted" | "compressed"`. SDKs encode only declared locations; "encode every payload"
  is not a supported mode, because the engine reads undeclared values.
- **D1521 — Carriers and positions.** Allowed on workflow `inputSchema`, `outputSchema`, and node `outputSchema` of
  `activity`, `child`, `waitEvent`; at the carrier root, under `properties` and under `items` (array wildcard).
  Refused under `additionalProperties`, `patternProperties`, `prefixItems`, `allOf/anyOf/oneOf`, `$defs`/`$ref`
  targets, a property named `*`, nested encodings, and outside the three carriers. A `script`, `database` or other
  platform-executed node cannot declare encoded output (`ENCODING_NOT_PRODUCIBLE`): it holds no key.
- **D1522 — Envelope v1.** `{"$encoded": {v, codecs, cty, lvl, ctx, key?, iv?, data}}`: AES-256-GCM over a data key
  (DEK) wrapped by a customer key-encryption key (KEK); optional `zstd`/`gzip`, then `pad`, before encryption; AAD is the
  RFC 8785 canonical form of every member except `data`, so the context `ctx` is authenticated (D1556). Stored payloads
  are immutable, so every SDK release MUST decode v1 forever (ADR-0066).
- **D1523 — Compression-only envelopes** (`codecs: ["zstd"]`, `x-sagat-encoding: "compressed"`) are opaque to the
  engine exactly like encrypted ones and are decoded only by customer components.
- **D1524 — Structural validation only.** At declared locations the server checks a present value is a closed,
  bounded envelope of the declared kind; it never decodes base64 `data`, decompresses or decrypts. Plaintext or a
  malformed envelope is refused and not persisted (inbox rows stored at admission and uploaded blobs awaiting the
  ADR-0023 sweep are the stated exceptions).
- **D1525 — Read rules.** For every server-evaluated expression: an access path that descends into an encoded
  location is refused; a path that ends at an encoded location or at a container holding one is allowed only as the
  whole expression in a carriage position (`child.input`, `end.outputExpr`, `return.result`, `continueAsNew.input`,
  `forEach.items`) or as the argument of `has()`; a non-literal index is a wildcard.
- **D1526 — Carriage consistency.** Within one document (`end.outputExpr`, 022 `earlyReturn.result` →
  `outputSchema`; `continueAsNew.input` → `inputSchema`) the target declaration must equal the source, pointer patterns
  included (`ENCODING_CARRIAGE_MISMATCH`); across documents the admission check of D1557 decides (revised 2026-10-27).
- **D1527 — Shared check and registry.** One publication check (`crates/infra-ir/src/encoding_check.rs`) is the only
  authority, and feature 031 owns the cross-draft registry of positions, carriers and write paths
  (`contracts/encoded-field-registry.md`, coordinator C7). 014, 018, 019, 020, 021, 022, 024, 025, 026 and 030 cite it
  and use only its codes (`ENCODING_READ_BY_*`, `ENCODING_CARRIAGE_MISMATCH`, `ENCODING_NOT_PRODUCIBLE`); a position not
  registered is refused for documents with encoded locations (fail closed).
- **D1528 — Separate pin.** `workflow_versions.encoding_rules` (0 inert, 1 this feature) beside
  `classification_rules`; republishing an identical pre-feature document returns the old version with 0.
- **D1529 — Level of an envelope.** The effective level of an encoded location is the strictest level in its
  subtree (inherited, declared, and undeclared-`pii`), because a decode reveals the whole subtree. The envelope's
  `lvl` MUST equal it; the server checks it at write.
- **D1530 — Disclosure opt-in.** `payload-disclosures` returns envelopes as a new node kind `encoded` only when the
  request sets `includeEnvelopes: true`; otherwise encoded locations follow the existing `withheld`/omitted rules
  with reason `ENCODED`, so older clients never meet the new kind.
- **D1531 — Browser decoding, never server decoding.** The Studio calls the customer-hosted codec server directly;
  the platform never proxies, stores or receives plaintext from it.
- **D1532 — Codec authentication (superseded by D1558 on 2026-10-27).** The first proposal made a platform-signed
  assertion sufficient by default. The security review showed that the control plane holds its signing key, so an
  operator could mint assertions and decrypt offline; D1558 replaces it.
- **D1533 — Destinations.** An operator lists approved codec origins (`SAGAT_STUDIO_CODEC_ORIGINS`), which the Studio
  CSP `connect-src` includes deployment-wide — every tenant's Studio page may reach every listed origin, which is
  accepted and recorded in ADR-0062's combined Studio-origin refinement (coordinator C15); a namespace administrator
  (`codec:manage`) picks one endpoint whose origin is listed, or is refused `CODEC_DESTINATION_DENIED` (C16).
- **D1534 — Lease plan and gating.** A lease carries the decode pointers of its step document and the encode pointers
  of its output (and checkpoint and stream plans). A task whose node has a non-empty decode **or** output plan is leased
  only to a session that negotiated the codec minor and advertised a codec; otherwise it stays `READY` without consuming
  an attempt (pattern of 1.3 blob gating). A worker without a codec never receives an envelope (revised 2026-10-27,
  architecture review S2).
- **D1535 — Idempotency.** Every keyed write carrying envelopes (startRun, signalRun, signalExecutionChain, 018
  with-start and schedules, 022 standalone, 026 run-and-wait) fingerprints each envelope as the digest of
  `{v, codecs, cty, lvl, kid, ctx}` plus the optional content fingerprint of D1669; SDKs encode once per logical request
  (reusing the body for every transport retry of it) and once per attempt.
- **D1536 — Forward-only rotation.** Retained envelopes are never rewritten (content addressing, journal digests);
  rotation affects new writes only; key versions must stay decrypt-capable while data is retained. Destroying a key
  makes its envelopes unreadable (customer-side crypto-shredding), with no compliance claim.
- **D1537 — Key providers.** A `KeyProvider` interface, an OpenBao Transit provider (the customer's Transit, data-key
  generation and decrypt) and a static provider for tests; cloud KMS through the interface (O1520). No provider
  reads platform secrets or `ctx.secrets()`.
- **D1538 — Bounded key caches.** DEK reuse and unwrap caches are bounded by uses, age and entries; revocation takes
  effect within the cache lifetime, not instantly (G1528).
- **D1539 — Local secret scan.** Ciphertext defeats the server's `SECRET_IN_OUTPUT` scan, so the SDK scans plaintext
  against the attempt's resolved secrets before encoding and fails the step locally with `SECRET_IN_OUTPUT`.
- **D1540 — Pipeline position.** The data converter is the only SDK stage that transforms payload bytes, blob envelopes
  included, and is closest to the transport; 028 interceptors observe and decorate calls and MUST NOT transform payload
  bytes; 031 depends on 028 only for interceptor ordering (coordinator C5). Conformance observes the wire at the
  existing transport hook.
- **D1541 — Large values.** A carrier whose encoded form exceeds 256 KiB is sent as one root envelope in a blob with
  content type `application/vnd.sagat.envelope+json`; the server checks the header prefix while streaming
  (accident detection, not proof, G1523).
- **D1542 — Signals and events.** A `waitEvent` `outputSchema` may declare encoding. Signals are validated at
  admission (422); an inbox event that does not conform (plaintext, wrong `ctx`, envelope at an undeclared location)
  does not satisfy the wait and its delivery is recorded as refused; a trigger cannot start a run with such input
  (no run, audited). A client encodes event or signal data with `encodeValue` for the declared wait location.
- **D1543 — Declared operations.** Query/Update expressions may neither read nor carry encoded values in v1 (G1530).
- **D1544 — Studio scope.** Studio decodes; it does not encode. Start and signal forms explain that encoded fields
  require an SDK or the CLI.
- **D1545 — CLI.** The CLI uses the namespace codec endpoint for `--decode`/`--encode` with the customer's own bearer
  token (`--codec-token-file`, issued by the customer's identity provider) and a platform assertion; it holds no key and
  no cipher code.
- **D1546 — Key inventory.** An administrator-started, leased, resumable scan counts envelopes per key reference in a
  namespace's retained payloads; no write-path counter (a hot row per key would serialize completions).
- **D1547 — Reference codec server.** Shipped as a Go example (`examples/codec-server-go/`) plus a black-box
  conformance suite; not a supported product artifact in the first delivery (O1522).
- **D1548 — Inner annotations.** Classification annotations inside an encoded subtree are kept (they describe the
  plaintext) but collapse to the envelope level server-side; `validate` warns `ENCODING_LEVELS_COLLAPSED`.
- **D1549 — Telemetry vocabulary.** Logs, spans, metrics and audit carry only the closed `v`, algorithm and encoding;
  never `kid`, `kv`, `wk`, `iv`, `data` or sizes per value. `kid` is shown only in the administrator inventory.
- **D1550 — Permissions.** New `codec:manage` (admin). Assertions need `run:read`; `encode` assertions need
  `run:start`; a `pii` assertion needs the 016 reveal grant and an explicit `reveal: ["pii"]`, audited and never
  collapsed.
- **D1551 — Errors.** No new `Error.code`; new `details.reason` values, step failure codes and SDK catalog entries
  (contracts/errors.md).
- **D1552 — ADR.** The hard-to-reverse choices (key custody, declaration and opacity, envelope v1, browser decoding
  with assertion) are ADR-0066, Proposed.
- **D1553 — Test layout.** `tests/{contract,integration,fault,e2e,performance}/client-side-encryption/` with targets
  `contract_client_side_encryption`, `integration_client_side_encryption`, `fault_client_side_encryption`,
  `e2e_client_side_encryption`, `perf_client_side_encryption`; SDK scenarios `payload-encryption*`.
- **D1554 — Protocol numbering.** The codec minor is the next additive minor at merge time (1.7 at the base commit),
  coordinated with feature 019; field numbers are the next free ones at merge (19 for `TaskLease`, 6 for
  `WorkerCapabilities` at `70f09435`).
- **D1555 — Assertion signing key.** A dedicated ES256 key configured like the existing platform keys
  (`SAGAT_CODEC_ASSERTION_KEYS`, current plus previous; issuer `SAGAT_CODEC_ASSERTION_ISSUER`); `GET /v1/codec-jwks` publishes public keys; it moves to
  OpenBao Transit when feature 015 lands. ES256 was chosen over EdDSA for JWT library coverage in the five ecosystems.
  The keys are entries of feature 032's cell key inventory (coordinator C11).

### Review round 2026-10-27 (architecture review 031 B1, S2–S5; security review 031 B1, S1, S2, M1–M6; 019 M8;
### coordinator decisions on the blocking findings and C5, C7, C9, C11, C15, C16)

- **D1556 — Envelope v1 binds its context.** The header carries `ctx = {t: tenant id, n: namespace id, p: pointer pattern
  of the location in its carrier}`, authenticated as AAD. The server checks `ctx` at every write
  (`ENVELOPE_CONTEXT_MISMATCH`); SDK decoders check it against the lease and codec servers against the assertion. An
  envelope from namespace N1 pasted into N2 (a database operator, a shared `kid`) is therefore refused by N2's decoders.
  Consequence: carriage keeps bytes only for the same tenant, namespace and pattern; a re-rooted or cross-namespace move
  needs a customer worker that re-encodes, or is refused. Binding was preferred to a later v2 because v1 is permanent and
  a v2 would leave v1 writable. Parsing is strict (duplicate members refused); `pad` buckets the compressed length.
- **D1557 — Envelopes exist exactly where declared.** Under `encoding_rules: 1` every write refuses an envelope at a
  location the receiving carrier does not declare (`ENCODING_CARRIAGE_MISMATCH`, one bounded walk of the ≤ 256 KiB value).
  At cross-document carriage (child admission, 022 handler child and `fromRun` pins, reset/rerun into another version,
  030 handler admission) the engine also maps the source's pinned plan through the carriage path and refuses unless the
  target's pinned plan declares every mapped location with the same encoding and pattern — whatever the target's
  rules, so a rules-0 child never receives ciphertext. Fault test T031-066.
- **D1558 — The customer authenticates the person; the assertion only narrows (coordinator decision on security B1).**
  A codec server MUST authenticate the person with the customer's own identity provider (a session cookie for the
  Studio, a customer bearer token for the CLI). The platform assertion travels in `X-Sagat-Codec-Assertion`, is
  single-use (`jti`), lasts the deployment TTL (default 60 s, at most 300 s; no client override) and narrows tenant,
  namespace id, purpose and level; it never authorizes alone. `/encode` requires `level ≤ sagat_max_level` and takes
  `ctx` from the assertion. The endpoint configuration has no "assertion only" mode.
- **D1559 — Fleet activation.** Publication with `x-sagat-encoding` is refused (`ENCODING_NOT_ACTIVATED`) until an
  audited operator command activates the feature through feature 018's instance-capability registry (FR-018-052,
  ADR-0050 decision 9; 031 defines no member table), which succeeds only when every live control-plane and
  execution-engine instance lists `payload-encoding-v1`, and records the requirement so that a registered instance
  without it stops serving. This replaces the per-process environment flag; validation of already published rules-1
  versions never depends on it (revised 2026-10-27 on the coordinator's alignment).
- **D1665 — KMS outage.** A worker host whose key provider reports unhealthy stops claiming tasks with a payload plan;
  they stay `READY` without spending attempts. Claimed tasks fail `PAYLOAD_CODEC_UNAVAILABLE` (retryable) under their
  ordinary budget; the runbook recommends pausing affected runs during a long outage.
- **D1666 — Side channels.** 019 `heartbeatSchema` (activity) and 026 stream channel `schema` are carriers. For an
  attempt with a non-empty payload plan, checkpoint details and stream appends are accepted only when that schema is a
  carrier of a rules-1 version and the value passes the write-path checks; otherwise `ENCODING_REQUIRED` (019 replaces its
  `REFUSED_ENCODING`). This closes the plaintext channel the security review found (S2); plaintext in undeclared fields of
  a declared channel stays the author's responsibility (G1541).
- **D1667 — No encoded values across namespaces in v1.** 030 operation `inputSchema`/`outputSchema` refuse the
  annotation, a handler version that declares encoding cannot be bound, and `serviceCall.input`/`compensate.input`
  that would carry an encoded location are refused (`ENCODING_CARRIAGE_MISMATCH`). With `ctx` binding, bytes cannot
  cross namespaces, and no platform component may re-encode (G1536).
- **D1668 — Codec endpoint ceiling (coordinator C9).** An endpoint declares `levelCeiling` (`public`, `internal`,
  `pii`); `pii` can be set only by an administrator holding the 016 reveal grant, recorded as setter; every `pii`
  assertion rechecks that grant and is refused (`CODEC_CEILING_UNCONFIRMED`, endpoint shown unconfirmed) after its loss;
  every assertion is audited; `secret` is never allowed. Assertion `maxLevel` = min(caller audience, ceiling).
- **D1669 — Content fingerprint for keyed writes (security M6, optional).** A key provider that offers a keyed digest
  (Transit HMAC, static derived key) lets the SDK send `X-Sagat-Content-Fingerprint` = keyed digest of the canonical
  plaintext; the server folds it into the request fingerprint only (never stored alone or logged), so a retry with
  changed content under a reused key is `409` again. Without it, D1535's header fingerprint applies (G1526).

## Gaps

| ID | Gap | Handling | Status |
|---|---|---|---|
| G1520 | `ctx` binds tenant, namespace and pointer pattern (D1556), but an envelope can still be replayed into another run of the same namespace at the same pattern, or moved between elements of one array | Accepted for v1; a run or element binding would break reruns and resets | Accepted |
| G1521 | Structure, lengths, timing, levels, key references, context and codec chain are visible to the platform; compression can amplify length leakage | Documented threat model; compression is opt-in and padded to buckets (G1540) | Accepted |
| G1522 | Browser decoding trusts the Studio code the platform serves | Documented; a deployment that cannot trust it configures no codec endpoint and decodes in customer tools | Accepted |
| G1523 | Blob content at an encoded location is checked by header prefix only; a plaintext blob refused at bind or completion stays in the store, uncommitted, until the ADR-0023 sweep (24 h) | Accident detection; ADR-0023 keeps blobs opaque; runbook tells operators the sweep window | Accepted |
| G1524 | Events without a `waitEvent` declaration and trigger filters cannot use encoded data; plaintext event data aimed at a declared wait is still retained in the inbox for its deduplication window, because no wait is known at admission; no event schema registry (G282) | Filters fail at evaluation as today; refusal at match keeps it out of step outputs; documented | Open |
| G1525 | No taint tracking: a worker can copy decoded plaintext into undeclared fields, search attributes, logs or streams (extends G276) | Guide and review | Accepted |
| G1526 | Key reuse with different content cannot be detected at encoded locations (D1535) | Documented trade-off for restart-safe replay | Accepted |
| G1527 | Fresh nonces defeat content-address deduplication and grow retained bytes | Counted against ADR-0027 quotas; measured by T031-062 | Open |
| G1528 | Revocation reaches customer processes only after their key caches expire | Bounded by the cache ceilings | Accepted |
| G1529 | Platform-executed steps (script, database, managed operations, AI agents) cannot read encoded values | Out of scope; no trusted-executor mode | Open |
| G1530 | Declared Query/Update results cannot carry envelopes (Workflow Streams can, through a declared channel schema, D1666) | Refused at publication | Open |
| G1531 | No first-party cloud KMS providers if O1520 keeps its default | Interface plus conformance vectors | Open |
| G1532 | The key inventory is a point-in-time scan, not a live count | Documented | Accepted |
| G1533 | The SDK-side secret scan covers only secrets resolved in the attempt | Documented | Accepted |
| G1534 | Payload retention is undecided (G69, ADR-0023), so key versions must be kept indefinitely | Runbook rule: disable, never destroy, while data may exist | Open |
| G1535 | Archive and analytics exports (020) carry ciphertext; reading them needs the customer codec | 020 must export envelopes unchanged and never decode; C9 applies by `lvl` | Open |
| G1536 | Encoded values cannot cross namespaces (030), and no platform component may re-encode | Needs a delegation design (customer key policy for a pair of namespaces plus a re-encoding step) | Open |
| G1537 | A binary older than 018's instance-capability registry does not register, so after activation it is neither refused by the registry nor seen by it, and it does not enforce | Runbook ("never start an older replica after activation"); registered instances without the capability are blocked by 018's rule and counted by `payload_encoding_incapable_members` | Accepted |
| G1538 | Moving an encoded value to another pointer needs a customer worker step that decodes and re-encodes | Documented in the guide; publication says where | Accepted |
| G1539 | Browsers that block third-party cookies can break `codec-session` when the codec origin is another site than the Studio | Host the codec server on the Studio's site, or decode in customer tools; no platform token fallback | Open |
| G1540 | Padding buckets narrow, but do not remove, the compressed-length signal; timing remains | Documented; compression stays opt-in | Accepted |
| G1541 | A declared side channel (checkpoint, stream) can still carry plaintext in its undeclared fields (taint, extends G1525) | Author responsibility; guide and review | Accepted |
| G1665 | A planless write — event, webhook or message trigger start, REST start, signal or inbox match — into a version with `encoding_rules = 0` (published before this feature) is not checked: envelope-shaped data is admitted as ordinary JSON. Consequence: that legacy version's expressions, keys and filters evaluate the envelope object (conditions over its shape, keys derived from ciphertext, or `EXPRESSION_ERROR`); no plaintext is exposed, but behaviour is wrong and silent. The only signal is the publication warning `ENCODING_TRIGGER_UNVERIFIED` on encoded senders | Rules 0 cannot be reinterpreted (D1528). Republishing the target with any change moves it to rules 1, where the undeclared-envelope walk refuses such data; the runbook's report (T031-061) lists active rules-0 versions with event, webhook or message triggers; publishers must not send envelopes to legacy consumers | Accepted |

## Owner questions

- **O1520 — Which key providers ship first-party?** Options: (a) OpenBao Transit plus a test-only static provider in
  all five SDKs, cloud KMS through the interface; (b) also AWS KMS; (c) AWS, Google Cloud and Azure KMS.
  **Recommended default: (a).** Each first-party provider multiplies five-language maintenance and live-service
  tests; the interface plus shared vectors lets customers bring their own.
- **O1521 — How the codec server authenticates the person.** Options: (a) the customer's own identity provider
  (`codec-session` cookie or customer bearer token) is required, and the platform assertion only narrows tenant,
  namespace, purpose and level; (b) the platform assertion alone is sufficient; (c) pass the user's platform access
  token (Temporal style). **Recommended default: (a)**, adopted as the coordinator's decision on 2026-10-27 and pending
  the owner. (b) lets a platform operator, or a compromised control plane, mint assertions and decrypt envelopes copied
  from the database; (c) hands a credential valid at the platform API to a third-party origin.
- **O1522 — Is the reference codec server a supported artifact?** Options: (a) example plus black-box conformance;
  (b) a released, signed container image with security support. **Recommended default: (a)** for the first
  delivery; (b) adds a supported product that holds customer keys.
