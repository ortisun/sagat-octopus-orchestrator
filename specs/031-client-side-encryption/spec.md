# Feature Specification: Client-side payload encryption

**Feature Branch**: `feature/031-client-side-encryption`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: SDK payload codecs (the counterpart of Temporal's data converter and payload codec): encrypt selected
fields or whole payloads on the client side with customer-managed keys (OpenBao Transit or a cloud KMS), in a
versioned envelope with key identifiers and rotation, plus a compression codec; a customer-hosted codec server so
that the Studio can decode in the browser while the platform never holds a key; the engine treats encoded values as
opaque, and publication rejects expressions, search attributes, analytics declarations, cache keys and forms over
encoded fields; the interplay with per-field classification (ADR-0033); five-SDK conformance, with the codec ordered
relative to the SDK interceptors of feature 028.

## Context and delivery boundary

Today every run input, step output, signal and run result is stored as plaintext JSON in `payloads`
(content-addressed, at most 256 KiB) or as an opaque blob (ADR-0023). Feature 016 decides who may *see* a value
(ADR-0033), but anyone who can read the platform's database, backups, object storage or a future export can read
every value (016 G279). Some customers must keep regulated data (card data, health data, personal data under a
contract) unreadable to the operator of the orchestration platform, including its administrators.

This feature lets the author of a workflow declare which locations of its payloads are **encoded**, lets the five
SDKs encrypt (and optionally compress) those locations inside customer processes with keys held only by the
customer, and makes the platform enforce three things it can enforce without a key: that it never accepts plaintext
where encoding is declared, that it never tries to evaluate what it cannot read, and that ciphertext is delivered
only to callers the classification allows. It adds the annotation `x-sagat-encoding` and its pinned
publication checks, the versioned envelope `{"$encoded": {...}}`, write-path validation, SDK data converters with
key providers in five languages, payload and lease plans, a customer-hosted codec server protocol (the customer's
identity provider authenticates the person; a platform-signed assertion only narrows) for the Studio and the CLI,
envelope disclosure under classification, the cross-draft registry of encoded-field positions, and a key-usage
inventory.

What it leaves alone, and says so: the structure-only inspection of ADR-0028 (an envelope is an object like any
other), the payload-free run, journal, graph and stream reads, the semantics of every node, the secret store of
features 009/017 (codec keys are never platform secrets), and every published version that does not use the
annotation. Retention and erasure stay undecided (G69, ADR-0023); destroying a customer key makes its envelopes
unreadable, which this feature documents but does not present as compliance.

### Threat model

Protected: payload values at declared locations against anyone who reads the platform's PostgreSQL, blobs, backups,
logs, traces, metrics, audit, exports or analytics sinks; against platform operators, a compromised control plane and
tenant administrators, **because the codec server authenticates every person with the customer's own identity
provider** — the platform-signed assertion (whose key the control plane holds) can only narrow a request, never
authorize one (D1558); against other tenants and namespaces, because every envelope is bound to its tenant, namespace
and location (D1556). Also protected: accidental plaintext — a misconfigured client or worker that sends plaintext to
a declared location is refused before it becomes a payload (two stated exceptions: an inbox event stored at admission,
an uploaded blob awaiting the ADR-0023 sweep).

Not protected (stated, not hidden): customer clients, workers and the codec server, which hold keys or plaintext; a
person the customer's identity provider admits to the codec server; a malicious worker, which can copy plaintext
anywhere; the Studio code the platform serves, which receives decoded values in the browser of a legitimately
authenticated person (G1522); metadata the platform sees (structure, lengths, timing, level, tenant, namespace and
location of each envelope, key reference, codec chain; G1521, G1540); replay of a valid envelope into another run of
the same namespace and location (G1520); and the content of blobs beyond a header check (G1523). Envelopes prove
possession of a key, not authorship.

## Domain vocabulary

- **Encoding**: `encrypted` (an AEAD cipher is in the envelope's codec chain, compression optional) or `compressed`
  (any envelope; compression only, or compression plus encryption).
- **Encoded location**: a location of a carrier schema that carries `x-sagat-encoding`, written as a pointer pattern
  (RFC 6901 JSON Pointer in which a `*` segment means every element of an array).
- **Envelope**: the JSON object `{"$encoded": {...}}` defined in [contracts/envelope.md](contracts/envelope.md) that
  replaces the plaintext value at an encoded location.
- **Payload plan**: for a pinned workflow version, the list of encoded locations per carrier with their encoding and
  effective level; derived at publication, immutable with the version.
- **Codec**: a transformation stage of an SDK data converter (`zstd`, `gzip`, `pad`, `aes-256-gcm`); the converter is
  the only SDK stage that transforms payload bytes (coordinator C5).
- **Envelope context** (`ctx`): the tenant id, namespace id and pointer pattern an envelope is bound to.
- **Key provider**: the customer-side component that generates and unwraps data keys under a customer key
  (`kid`, optional version `kv`).
- **Codec server**: a customer-hosted HTTPS service implementing [contracts/codec-server.openapi.yaml](contracts/codec-server.openapi.yaml).
- **Codec assertion**: a short-lived, single-use token signed by the platform that narrows a codec request to a tenant,
  a namespace, a purpose and a maximum level; it never authenticates the person.
- **Registry**: [contracts/encoded-field-registry.md](contracts/encoded-field-registry.md), the list of every position,
  carrier and write path of the IR and the parity drafts that touch encoded values (coordinator C7).
- **Carriage position**: an expression position where the engine copies a value without interpreting it.
- **Encoding rules version**: the interpreter version stored with a workflow version (`0` inert, `1` this feature).

## User Scenarios & Testing *(mandatory)*

### US01 — Declare encoded fields and keep the engine away from them (Priority: P1)

An author marks the card and the national identifier of an order workflow as `encrypted`, publishes, and is told at
publication — not at the first run — about every expression, consumer or node that would need to read them.

**Why this priority**: the engine evaluates the IR server-side; an undeclared ciphertext would surface as a runtime
expression failure. Declaration and its publication checks are the foundation of every other story.

**Independent Test**: publish one sound document and one document per defect class of
[contracts/workflow-encoding.md](contracts/workflow-encoding.md); the sound one publishes with a stable digest and a
plan, each defect is refused with its code and pointer, and a pre-feature identical document stays inert.

**Acceptance Scenarios**:

1. **Given** an `inputSchema` with `/card` and `/customer/nationalId` marked `encrypted`, **when** the author validates
   and publishes, **then** the version stores `encodingRules: 1`, and `GET .../payload-plan` lists both pointers with
   their encoding and effective level.
2. **Given** a `condition` whose expression is `input.card.brand == "visa"`, **when** validated, **then** publication is
   refused with `ENCODING_READ_BY_EXPRESSION` at the expression's path naming `/card`.
3. **Given** `child.input = input` and `end.outputExpr = input`, **when** validated, **then** both are accepted as
   carriage; `end.outputExpr = input` is refused with `ENCODING_CARRIAGE_MISMATCH` if the workflow `outputSchema` does
   not declare `/card` and `/customer/nationalId` encoded too, and `end.outputExpr = input.customer` is refused because it
   would move `/customer/nationalId` to `/nationalId` (an envelope is bound to its pointer).
4. **Given** an annotation under `additionalProperties`, inside `allOf`, on a `script` node's `outputSchema`, inside
   declared state or nested under another encoded location, **when** validated, **then** each is refused with its code
   (`ENCODING_UNSUPPORTED_LOCATION`, `ENCODING_NOT_PRODUCIBLE`, `ENCODING_NOT_APPLICABLE`, `ENCODING_NESTED`).
5. **Given** a search attribute (020), an analytics declaration (020), a cache or concurrency key (022), a fairness key
   (019), a generated form (026), an agent context (025) or a Query/Update (014) that reads an encoded location, **when**
   the document is published with those features enabled, **then** it is refused with the code the registry assigns.
6. **Given** a parent whose `child.input` carries `/card` into a child version that does not declare `/card` encoded
   (or has `encodingRules: 0`), **when** the child is admitted, **then** the parent step fails with
   `ENCODING_CARRIAGE_MISMATCH` and no child run exists; the same holds for a 030 handler and a 022 `fromRun` pin.
7. **Given** a version published before this feature that already carried the key `x-sagat-encoding`, **when** the
   identical document is published again, **then** the answer is that version with `encodingRules: 0` and the warning
   `ENCODING_INERT_VERSION`; nothing is enforced for its runs.

### US02 — Encrypt in clients and workers with customer-managed keys (Priority: P1)

A developer configures the SDK client and worker with a data converter that encrypts under a key in the customer's
OpenBao Transit; the run starts with ciphertext, the worker handler receives plaintext, and its output is encrypted
where the node declares it. The platform stores only envelopes at declared locations.

**Why this priority**: it is the value of the feature; without it nothing is protected.

**Independent Test**: run the shared `payload-encryption` scenario in each of the five SDKs against the real platform
and a real OpenBao Transit; capture the wire at the transport hook and the PostgreSQL rows; seed plaintext canaries.

**Acceptance Scenarios**:

1. **Given** a client with an encryption codec and the plan of version 3, **when** it starts a run, **then** the
   request carries envelopes at `/card` and `/customer/nationalId`, the run is admitted, and no canary appears in any
   PostgreSQL row, log, span, metric or audit record.
2. **Given** a worker with the same key provider, **when** it takes the task, **then** the lease names the pointers to
   decode and encode, the handler receives plaintext, and `CompleteStep` carries envelopes at the node's declared
   output locations with `lvl` equal to their effective level.
3. **Given** a client or worker without a codec that sends plaintext to a declared location, **when** the start or the
   completion arrives, **then** the start is refused with `422 VALIDATION_FAILED` (`details.reason = ENCODING_REQUIRED`,
   the pointer pattern, no value) and the completion fails the step with `ENCODING_REQUIRED` (not retryable); in both
   cases the plaintext is not stored.
4. **Given** a worker that negotiated an older protocol minor or advertised no codec, **when** a task whose step
   document holds envelopes or whose node declares encoded output is ready, **then** that worker is never leased the
   task, which stays `READY` without consuming an attempt, and a codec-capable worker completes it.
5. **Given** a TypeScript client that encrypted the input, **when** Go, Python, C# and Java workers decode it, **then**
   each obtains byte-identical plaintext; a tampered header or ciphertext fails authentication in all five.
6. **Given** a handler whose output contains a platform secret resolved in the same attempt, **when** the SDK is about
   to encode it, **then** the step fails locally with `SECRET_IN_OUTPUT` and nothing is sent.
7. **Given** an envelope copied from a run of namespace N1 into a start request of namespace N2, **when** it arrives,
   **then** it is refused with `ENVELOPE_CONTEXT_MISMATCH`; an envelope placed at a location the version does not declare
   is refused with `ENCODING_CARRIAGE_MISMATCH`.
8. **Given** a handler of a node with encoded input that records heartbeat checkpoint details (019) or appends stream
   events (026) to a channel without an encoded schema, **when** they arrive, **then** they are refused with
   `ENCODING_REQUIRED` and the lease is still renewed.

### US03 — Rotate keys and survive key and KMS failures (Priority: P1)

A security administrator rotates the customer key, later disables an old version, and the KMS has an outage. Runs
keep working where they can, fail with typed, bounded errors where they cannot, and plaintext is never the fallback.

**Why this priority**: key rotation and KMS availability are where encryption features lose data or leak it.

**Independent Test**: rotate the Transit key between encode and decode, disable an old version with
`min_decryption_version`, stop Transit, use an unknown `kid`, expire the caches; observe step outcomes, retries,
the Studio and the canaries.

**Acceptance Scenarios**:

1. **Given** an input encrypted under key version 3 and a rotation to version 4, **when** the worker decodes, **then** it
   succeeds (the KMS still decrypts version 3) and the output is encrypted under version 4.
2. **Given** a version disabled at the KMS, **when** a worker decodes an envelope of that version after its unwrap cache
   expired, **then** the step fails with `PAYLOAD_KEY_UNAVAILABLE` (not retryable) and follows its `onError` or failure
   policy; no plaintext or ciphertext appears in the failure.
3. **Given** the KMS is unreachable, **when** a worker host notices it, **then** it stops claiming tasks with a payload
   plan (they stay `READY`, no attempt spent); a task already claimed fails with `PAYLOAD_CODEC_UNAVAILABLE` (retryable)
   under its ordinary retry budget; **when** a client must encode a start, **then** the SDK raises the same code before
   sending anything and no run exists.
4. **Given** a compensation whose `steps.<node>.output` envelope cannot be decoded, **when** it runs, **then** it fails
   like any compensation and reaches the existing manual-review path; nothing is assumed undone.
5. **Given** retained runs, **when** the administrator asks which key versions their envelopes use, **then** the
   key-usage inventory (US08) answers per `kid`/`kv` without values.

### US04 — Read encrypted values in the Studio through a customer codec server (Priority: P2)

An operator opens a run, asks to decode, and the Studio sends the envelopes it was allowed to receive to the
customer's codec server — where the operator is signed in with the customer's own identity provider — and shows the
plaintext in the page only. When the codec server is down or the person has no customer session, the run view still
works and says why the values are not shown.

**Why this priority**: diagnosis needs values; the browser path keeps the platform away from plaintext.

**Independent Test**: a real Chromium journey with the reference codec server: configure the endpoint, decode, close,
inspect DOM/storage/query cache/network; repeat with the codec stopped, returning 401/403, refusing CORS, timing out
and lacking a key.

**Acceptance Scenarios**:

1. **Given** an operator deployment whose `SAGAT_STUDIO_CODEC_ORIGINS` lists `https://codec.example`, **when** an
   administrator sets the namespace codec endpoint to `https://codec.example/sagat` with `levelCeiling: internal`,
   **then** it is saved with audit; an endpoint whose origin is not listed is refused with `CODEC_DESTINATION_DENIED`;
   `levelCeiling: pii` is accepted only from an administrator who holds the reveal grant.
2. **Given** a disclosed run input with two `encoded` nodes and a person signed in to the codec server, **when** the
   operator presses "Decode with codec server" and confirms, **then** the Studio obtains an assertion, sends one batch
   with the customer session and the assertion, renders the plaintext with its level marker, and clears it when the
   view, run, namespace or session changes.
3. **Given** a platform administrator with no customer session who mints or replays an assertion and posts envelopes
   to the codec server, **when** the codec server answers, **then** every request is `401`: the assertion alone
   authorizes nothing, and a second use of the same assertion is refused.
4. **Given** the codec server is down, answers 401/403, fails CORS, times out or reports a key error for one envelope,
   **when** decoding is requested, **then** each affected node shows a distinct, non-sensitive state and the rest of the
   page keeps working; no request with a value goes to the platform.
5. **Given** any user, **when** the page is inspected after decoding, **then** no decoded value is found in
   `localStorage`, `sessionStorage`, IndexedDB, the query cache or the platform network log, and none remains in the
   DOM after the view closes.
6. **Given** a workflow whose input has encoded fields, **when** a user opens Start run or Send signal, **then** the
   Studio says those fields must be encoded by an SDK or the CLI and does not submit plaintext for them.

### US05 — Classification decides who receives ciphertext (Priority: P2)

Ciphertext is still data. The levels of ADR-0033 decide who may receive an envelope, and the codec server is told the
highest level the caller may see, so the two gates agree.

**Why this priority**: without it, any `run:read` holder could collect envelopes and ask a codec server for them.

**Independent Test**: the audience matrix of 016 extended with `includeEnvelopes` and assertion levels, as a REST
contract test, plus the reference codec server's refusal of envelopes above `maxLevel`.

**Acceptance Scenarios**:

1. **Given** an encoded location declared `public` with a `pii` child, **when** published, **then** its effective level is
   `pii` (the strictest of its subtree) and `validate` warns `ENCODING_LEVELS_COLLAPSED`.
2. **Given** a developer without a reveal grant, **when** they disclose with `includeEnvelopes: true`, **then** `internal`
   envelopes are returned as `encoded` nodes, `pii` ones are absent (or `withheld` for an inspector), `secret` ones never.
3. **Given** a revealer and an endpoint whose `levelCeiling` is `pii`, **when** they request an assertion with
   `reveal: ["pii"]`, **then** it carries `maxLevel: "pii"`, one `codec.assertion_issued` audit record is committed
   before it is returned, and repeats never collapse; after the ceiling's setter loses the grant, the request is
   refused with `CODEC_CEILING_UNCONFIRMED`.
4. **Given** an assertion with `maxLevel: "internal"`, **when** it is presented to the reference codec server with a
   `pii` envelope, **then** that envelope is refused with `LEVEL_NOT_PERMITTED`.
5. **Given** a write whose envelope `lvl` differs from the location's effective level, **when** it arrives, **then** it is
   refused with `ENVELOPE_LEVEL_MISMATCH` and not stored.
6. **Given** a request without `includeEnvelopes`, **when** it is answered, **then** the response uses only node kinds
   that existed before this feature.

### US06 — Compress large values (Priority: P2)

A developer compresses a large document before encryption, or compresses a non-sensitive large value only, to stay
under the inline limit; values that still do not fit travel as one encrypted blob.

**Why this priority**: envelopes grow payloads by about a third; compression and the blob path keep runs inside the
existing limits.

**Independent Test**: compress-then-encrypt and compress-only values near 256 KiB; a value over the limit through the
blob path; a decompression bomb; a blob without the envelope header.

**Acceptance Scenarios**:

1. **Given** a codec chain `zstd` then `aes-256-gcm`, **when** a value is encoded, **then** the envelope lists
   `["zstd", "pad", "aes-256-gcm"]` (the compressed bytes padded to a size bucket) and every SDK decodes it to the same
   bytes.
2. **Given** a location marked `compressed`, **when** a compression-only envelope arrives, **then** it is accepted and
   remains opaque to the engine; **given** a location marked `encrypted`, a compression-only envelope is refused with
   `ENCODING_REQUIRED`.
3. **Given** an encoded output over 256 KiB, **when** the worker completes, **then** the SDK uploads one root envelope as
   a blob of type `application/vnd.sagat.envelope+json`; a blob of that type whose first bytes are not an envelope
   header is refused while streaming.
4. **Given** an envelope that decompresses past the decoded-size ceiling, **when** an SDK decodes it, **then** it stops
   at the ceiling with `PAYLOAD_DECODED_TOO_LARGE` and never allocates beyond it.

### US07 — Decode and encode from the CLI (Priority: P3)

An engineer reads the decoded input of a run and starts a run with encoded input from a terminal, without copying a
key to the terminal.

**Why this priority**: useful for operations; the SDKs already cover programmatic use.

**Independent Test**: `sagat workflow inspect <RUN> --values --decode` and `sagat workflow run <NAME> --encode` against the
reference codec server, with the codec up and down.

**Acceptance Scenarios**:

1. **Given** a configured codec endpoint and a customer-issued codec token, **when** `inspect --values --decode` runs,
   **then** the CLI obtains an assertion, calls the codec server directly with both, prints decoded values with control
   characters neutralised, and prints
   `[encoded: <level>, not decoded: <reason>]` for an envelope it could not decode.
2. **Given** `workflow run --encode` with a plaintext input file, **when** it runs, **then** the CLI fetches the plan, asks the
   codec server's `/encode` for the declared locations, and starts the run with the returned envelopes and a pinned
   `versionId`; with the codec server down it exits with a typed error and starts nothing.

### US08 — Know which keys retained data depends on (Priority: P3)

Before disabling a key version, an administrator scans a namespace and learns how many retained envelopes use each
key reference, and the oldest and newest payload that does.

**Why this priority**: rotation is forward-only (D1536); this is the evidence a key can be retired.

**Independent Test**: start a scan over runs written under three key versions, kill the scanning engine mid-way,
restart, and compare counts with a direct count.

**Acceptance Scenarios**:

1. **Given** `codec:manage`, **when** an administrator starts a scan, **then** one scan per namespace runs under a lease,
   resumes after a crash without double counting, and reports counts per (`kid`, `kv`, algorithm, encoding).
2. **Given** a second scan request while one runs, **when** it arrives, **then** it is refused with
   `KEY_USAGE_SCAN_RUNNING`; **given** a member without `codec:manage`, it is refused `403`.

### Edge Cases

- An encoded location absent from a payload is valid; a present `null` is plaintext and refused (SDKs encode `null`).
- An array wildcard location over an empty array; over 1,024 elements (ceiling: `ENVELOPE_INVALID`, reason too many
  envelopes); arrays of arrays up to four wildcards.
- An envelope whose `v` is unknown, whose `codecs` repeats a stage or puts compression after the cipher, whose header
  exceeds 2 KiB, or whose `data` is not canonical base64: `ENVELOPE_INVALID`.
- An envelope-shaped object at an undeclared location of a rules-1 carrier: refused (`ENCODING_CARRIAGE_MISMATCH`); in a
  rules-0 carrier it is ordinary JSON. SDKs decode only planned pointers.
- An envelope moved to another element of the same array, or replayed into another run of the same namespace at the
  same location: not detected (G1520).
- An expression `has(input.card)`: allowed (presence only); `size(input.card)`: refused; `input.items[i].card` with a
  computed `i`: treated as every element.
- A rerun, reset or child that inherits the input: envelopes are copied by digest and nothing is re-encrypted; the copy
  is admitted only if the target's pinned plan declares the same locations (FR-031-005).
- The active version changes between the client's plan fetch and its start: the SDK pins `versionId`; without a pin a
  mismatch is refused, never silently stored.
- Two retries of the same start, one re-encoded after a client restart: the second returns the original run (D1535),
  unless the content fingerprint of D1669 shows the plaintext changed (`409`).
- A rolling upgrade with an engine or control-plane replica older than this feature: publication with encoding stays
  refused (`ENCODING_NOT_ACTIVATED`) until every live replica reports the capability (FR-031-036).
- Timeout and signal race on a `waitEvent` with an encoded signal payload: the first commit wins; the envelope is never
  inspected; the loser is audited as today.
- A codec response with extra or unknown ids, non-JSON or over its ceiling is discarded whole
  (`CODEC_RESPONSE_INVALID`); envelopes beyond the disclosure envelope budget are `withheld` (`RESPONSE_BUDGET`) for
  an inspector, omitted otherwise, with `truncated: true`.
- An endpoint deleted, or its origin removed from `SAGAT_STUDIO_CODEC_ORIGINS`, while a page is open: the next
  assertion is refused (`409 CODEC_ENDPOINT_NOT_CONFIGURED` or `CODEC_DESTINATION_DENIED`).

## Requirements *(mandatory)*

### Functional Requirements

**Declaration and publication**

- **FR-031-001**: An author MUST be able to annotate a carrier schema location with `"x-sagat-encoding"` whose value is
  exactly `"encrypted"` or `"compressed"`. Carriers are the workflow `inputSchema`, the workflow `outputSchema`, the
  node `outputSchema` of `activity`, `child` and `waitEvent`, and the side-channel carriers of FR-031-035 (019
  `heartbeatSchema` of `activity`, 026 `streams.<name>.schema`), exactly as the
  [registry](contracts/encoded-field-registry.md) lists them. Anywhere else is refused (`ENCODING_NOT_APPLICABLE`),
  including declared state, Query/Update argument and result schemas, `humanApproval.formSchema`, 030 operation schemas,
  triggers and metadata. A `script`, `database` or any other platform-executed node MUST NOT declare encoded output
  (`ENCODING_NOT_PRODUCIBLE`). The key is additive inside IR v1 (ADR-0032).
- **FR-031-002**: An encoded location MUST be the carrier root, a member of `properties`, or `items` (one array wildcard
  per `items`), reached from the root through those keywords only. Refused, with the JSON pointer of the annotation:
  under `additionalProperties`, `patternProperties`, `prefixItems`, `allOf`, `anyOf`, `oneOf`, `$defs` or a `$ref`
  target, or on a property named `*` (`ENCODING_UNSUPPORTED_LOCATION`); under another encoded location
  (`ENCODING_NESTED`); more than 64 locations per carrier, more than 4 wildcards or 16 segments per location, or a carrier
  over the 016 bounds (`ENCODING_TOO_COMPLEX`); a value other than the two strings (`ENCODING_INVALID_VALUE`).
- **FR-031-003**: The effective level of an encoded location MUST be the strictest of its effective level under the
  016 rules and every level declared or implied (undeclared `pii`) in its subtree. `validate` MUST report each encoded
  location with its encoding and level, and warn `ENCODING_LEVELS_COLLAPSED` when the subtree mixed levels.
- **FR-031-004**: Publication MUST analyse every server-evaluated expression the registry lists (IR v1 and the parity
  drafts) and refuse with the code the registry assigns (`ENCODING_READ_BY_EXPRESSION` by default) and the expression's
  path; a position not in the registry is refused for documents with encoded locations: (a) an access path that descends into an encoded location;
  (b) an access path that ends at an encoded location or at a container holding one, unless the expression is exactly
  that path in a carriage position (`child.input`, `end.outputExpr`, `return.result`, `continueAsNew.input`, and
  `forEach.items` when the path ends at an array that holds encoded locations rather than at one) or the argument of
  `has()`. A non-literal index matches every element.
- **FR-031-005**: Carriage MUST follow the location rule of
  [contracts/workflow-encoding.md](contracts/workflow-encoding.md#4-carriage-the-location-rule): within one document the
  target's encoded locations, re-rooted, equal the source's with equal pointer patterns (`ENCODING_CARRIAGE_MISMATCH`
  at publication); at every cross-document admission in the same namespace (child, 022 failure-handler child, 022
  `fromRun` pin, reset or rerun into another version) the engine maps the source's pinned plan through the carriage path
  and refuses with `ENCODING_CARRIAGE_MISMATCH` unless the target's pinned plan declares every mapped location with the
  same encoding and pattern, whatever the target's `encodingRules`; carriage into another namespace (030) is refused when
  the source holds an encoded location (D1667). Unpinned cross-document targets and event-triggered starts produce the
  warnings `ENCODING_CARRIAGE_UNVERIFIED` and `ENCODING_TRIGGER_UNVERIFIED`. A planless write (trigger, REST start,
  signal, inbox event) into a version with `encodingRules: 0` is not checked and admits envelope-shaped data as ordinary
  JSON (G1665).
- **FR-031-006**: Feature 031 MUST own the [registry](contracts/encoded-field-registry.md) of positions, carriers and
  write paths of the IR and of the parity drafts (coordinator C7) and expose one encoded-location check that those
  features call at publication; they MUST refuse a declaration that reads an encoded location with the registry's code —
  `ENCODING_READ_BY_SEARCH_ATTRIBUTE`, `ENCODING_READ_BY_ANALYTICS` (020, carriage included), `ENCODING_READ_BY_CACHE_KEY`,
  `ENCODING_READ_BY_CONCURRENCY_KEY` (022), `ENCODING_READ_BY_FAIRNESS_KEY` (019), `ENCODING_READ_BY_FORM` (026),
  `ENCODING_READ_BY_DECLARED_OPERATION` (014), `ENCODING_NOT_PRODUCIBLE` (027 write mappings and other platform- or
  app-produced values), `ENCODING_READ_BY_EXPRESSION` (021, 024, 025, 027 predicates and the rest) — and define no
  encrypted-field code of their own. A consumer that cannot call the check MUST refuse any document with encoded
  locations. A draft that adds a position registers it in the same delivery.
- **FR-031-007**: The interpretation MUST be pinned: `workflow_versions.encoding_rules` is `0` for every version published
  before this feature (the key is inert) and `1` for versions published under it; every version the API returns carries
  `encodingRules`. Republishing an identical document returns the existing version with its own rules and the warning
  `ENCODING_INERT_VERSION` when rules are `0`. A version diff MUST list an added encoding as `ENCODING_ADDED` and a removed
  or changed one as `ENCODING_REMOVED` (attention findings).
- **FR-031-008**: The platform MUST provide `GET /v1/workflows/{workflowId}/versions/{versionNumber}/payload-plan`
  (`definition:read` or `run:start`) returning the version id, `encodingRules`, the tenant and namespace ids that every
  envelope for this version is bound to and, per carrier, the pointer pattern, encoding and effective level of each
  encoded location; never a schema fragment beyond these.

**Envelope and enforcement**

- **FR-031-009**: Envelope v1 MUST be exactly the closed, bounded JSON object of [contracts/envelope.md](contracts/envelope.md):
  a closed codec chain (`zstd` or `gzip`, then `pad`, then `aes-256-gcm`), level, the context `ctx` (tenant id, namespace
  id, pointer pattern), key reference and nonce when encrypted, base64 `data`; AES-256-GCM with a 96-bit random nonce and
  the RFC 8785 canonical form of every member except `data` as associated data, so `ctx` is authenticated. Parsing is
  strict everywhere (duplicate members refused). Decoders MUST refuse an unknown version and a `ctx` that differs from
  their context.
- **FR-031-010**: On every write path the registry lists (REST start inline and `inputBlobRef`, `signalRun`,
  `signalExecutionChain`, child and handler admission, step completion inline and blob, the run result at `end`,
  `earlyReturn`, `continueAsNew`, inbox-event matching of a `waitEvent`, trigger and schedule starts, mocks, standalone
  inputs, side channels) into a carrier of a version with `encodingRules: 1`, the server MUST check each present value at
  an encoded location: a value that is not an envelope, or an envelope without a cipher at an `encrypted` location, is
  `ENCODING_REQUIRED`; a malformed or unsupported envelope is `ENVELOPE_INVALID`; an envelope whose `lvl` differs from
  the effective level is `ENVELOPE_LEVEL_MISMATCH`; an envelope whose `ctx` is not the run's tenant, namespace and the
  location's pattern is `ENVELOPE_CONTEXT_MISMATCH`; and an envelope at a location the carrier does not declare is
  `ENCODING_CARRIAGE_MISMATCH` (one bounded walk of the value). The server MUST NOT decode `data`, decompress or decrypt, and
  MUST NOT persist a refused value in payloads, the journal, audit, dead letters, logs or errors. An inbox event's data
  is stored at admission as today, before any wait is known (G1524); the refusal keeps it from becoming a step output.
- **FR-031-011**: Refusals MUST use these outcomes: REST writes `422 VALIDATION_FAILED` with the reason and the pointer
  pattern, no value; completion: the step fails non-retryably with the reason as failure code, the report is
  acknowledged and the original idempotency key replays that outcome; child or handler admission: the parent step fails
  with the reason and no child or handler run exists; `end`/`continueAsNew`: the run fails with the reason; inbox event:
  the wait is not satisfied and the delivery is recorded with the reason; a trigger or schedule start: no run, audit
  `run.encoding_refused`; a side channel: the checkpoint or stream event is refused and the lease is still renewed.
- **FR-031-012**: An encoded carrier delivered as a blob MUST be one root envelope uploaded with content type
  `application/vnd.sagat.envelope+json`; while streaming, the server MUST verify that the first 4 KiB parse as a valid
  envelope header written before `data`, and refuse the upload otherwise. A blob of another content type bound to an
  encoded location is refused at bind or completion; its bytes, already uploaded, stay uncommitted until the ADR-0023
  sweep of unused uploads removes them. The platform states this detects accidents and does not prove the remaining
  bytes are ciphertext (G1523).
- **FR-031-013**: For every keyed write that carries envelopes (`startRun`, `signalRun`, `signalExecutionChain`, 018
  Signal-/Update-with-Start and schedule writes, 022 standalone starts, 026 run-and-wait starts) the idempotency
  fingerprint MUST replace each envelope by the digest of its `v`, `codecs`, `cty`, `lvl`, `kid` and `ctx`, and include
  the optional `X-Sagat-Content-Fingerprint` (a keyed digest of the plaintext from the customer's key provider, D1669),
  so a retry re-encoded after a client restart replays the original outcome and, when the fingerprint is sent, changed
  content under a reused key is `409`. SDKs MUST encode once per logical request, reusing the encoded body for every
  transport retry of that request, and once per attempt for a report.
- **FR-031-014**: No server crate MAY hold, resolve or accept codec key material, decrypt, decompress or proxy
  encode/decode requests. The platform MUST NOT make any server-side call to a codec server.
- **FR-031-015**: A lease (and a 019 push invocation) MUST carry the payload plan of its task: the tenant and namespace
  ids, the pointers to decode in the step document with the pattern each envelope must be bound to, and the pointers to
  encode in the output and side channels, with encoding and level. A task whose decode **or** output plan is non-empty
  MUST be leased only to a session (or push target) that negotiated the codec protocol minor and advertised a payload
  codec; otherwise it stays `READY` without consuming an attempt, re-checked in the locked claim transaction. A worker
  without a codec never receives an envelope.

**SDKs**

- **FR-031-016**: Each SDK MUST offer a data converter option on the client and the worker host with an ordered codec
  chain, applied at the stage closest to the transport (after JSON serialization when sending, first when receiving). It
  is the only SDK stage that transforms payload bytes, blob envelopes included; 028 interceptors observe decoded values,
  MUST NOT transform payload bytes, and none can be placed between codec and transport (coordinator C5; 031 depends on
  028 only for this ordering).
- **FR-031-017**: Encoding MUST be plan-driven: the client uses the payload plan of the pinned version for a start; the
  worker uses the lease plan for its output and side channels; `ctx` always comes from the plan. Decoding MUST touch
  only planned pointers and check `ctx`. `codec.encodeValue(value, {encoding, level, pointer})` (for a declared
  `waitEvent` location receiving signal or event data) and `codec.decodeValue(envelope, {pointer})` are offered for
  explicit use.
- **FR-031-018**: Each SDK MUST define a key provider interface (generate data key for `kid`; unwrap data key for `kid`,
  `kv`, wrapped bytes) and ship an OpenBao Transit provider for the customer's own Transit and a static provider that
  refuses to start unless explicitly marked for test use. No provider MAY read platform secrets or `ctx.secrets()`.
  Data-key reuse and unwrap caches MUST be bounded by uses, age and entries within the ceilings of the plan; zero
  disables them. While its key provider reports unhealthy, a worker host MUST stop claiming tasks with a payload plan
  (D1665).
- **FR-031-019**: Before encoding an output, a checkpoint or a stream event, the worker host MUST scan the plaintext for
  values the attempt resolved through `ResolveSecrets` and fail with `SECRET_IN_OUTPUT` instead of sending.
- **FR-031-020**: SDK failures MUST use the canonical error type with the new codes `PAYLOAD_ENCODE_FAILED`,
  `PAYLOAD_DECODE_FAILED`, `PAYLOAD_KEY_UNAVAILABLE`, `PAYLOAD_DECODED_TOO_LARGE` (not retryable) and
  `PAYLOAD_CODEC_UNAVAILABLE` (retryable), reported as `FailStep` codes by the worker host; messages never contain a
  value, ciphertext, key, wrapped key or KMS response body.
- **FR-031-021**: Decoders MUST bound the decoded size (default 4 MiB, hard 64 MiB) while decompressing, refuse headers
  over 2 KiB, duplicate members and envelopes outside the closed format, and wipe data keys from memory when evicted
  where the language allows. Encoders MUST pad compressed plaintext to the size buckets of the envelope contract before
  encryption unless explicitly disabled.
- **FR-031-022**: The five SDKs MUST pass shared conformance: byte-identical builder documents with the annotation; a
  5×5 interoperability matrix in which every SDK decodes the shared envelope vectors and every other SDK's encodings to
  identical bytes; a runtime scenario whose wire capture shows envelopes only and whose goldens are equal; the fault
  cases of FR-031-020.

**Codec server, Studio and CLI**

- **FR-031-023**: The codec server protocol MUST be the contract of [contracts/codec-server.openapi.yaml](contracts/codec-server.openapi.yaml):
  `POST /decode`, optional `POST /encode`, `GET /capabilities`; batches with per-item results; `Cache-Control: no-store`;
  CORS with credentials for the Studio origin. A codec server MUST authenticate the person with the customer's own
  identity provider (session cookie or customer bearer token) and MUST treat the platform assertion only as a narrowing:
  refuse a request without either; check `sagat_tenant` and `sagat_namespace_id` against its configuration and each
  envelope's `ctx`; accept each `jti` once; refuse envelopes above `sagat_max_level`, keys not mapped to that namespace
  and expired or mis-audienced assertions; on `/encode`, require `level ≤ sagat_max_level` and take `ctx` from the
  assertion. The repository MUST ship a reference implementation and a black-box conformance suite that any codec
  server can run.
- **FR-031-024**: The platform MUST provide `GET|PUT|DELETE /v1/namespaces/{namespaceId}/codec-endpoint` (read with
  `run:read`, change with the new `codec:manage`, granted to `admin` only) holding an HTTPS URL whose origin is in the
  operator list `SAGAT_STUDIO_CODEC_ORIGINS` (HTTP only for loopback in development; otherwise
  `CODEC_DESTINATION_DENIED`), a `levelCeiling` (`public`, `internal`, `pii`; `pii` set only by an administrator holding
  the 016 reveal grant, rechecked at every `pii` assertion — coordinator C9), a revision with optimistic concurrency and
  audit in the same transaction. The Studio CSP `connect-src` MUST include exactly the listed origins; the list is
  deployment-wide, so every tenant's Studio page may reach every listed origin — accepted and recorded in ADR-0062's
  combined Studio-origin refinement (coordinator C15).
- **FR-031-025**: The platform MUST provide `POST /v1/namespaces/{namespaceId}/codec-assertions` returning an ES256 JWT
  with the claims of [contracts/codec-assertion.md](contracts/codec-assertion.md) — audience bound to the endpoint,
  tenant id, namespace id, subject, `purpose` (`decode` needs `run:read`, `encode` needs `run:start`), `maxLevel` per
  FR-031-028 capped by the endpoint ceiling, a single-use `jti`, and a lifetime fixed by the deployment (default 60 s,
  at most 300 s; no request may change it); `GET /v1/codec-jwks` publishes the current and previous keys. Issuance is
  audited in its transaction before the token is returned; a `pii` assertion requires `reveal: ["pii"]`, the 016 grant
  and a confirmed `pii` ceiling, and never collapses. Responses are `Cache-Control: private, no-store`.
- **FR-031-026**: The Studio MUST decode only nodes the server returned as `encoded`, only on an explicit confirmed action,
  in batches within the ceilings, directly to the configured endpoint with the customer session (`credentials:
  include`) and one fresh assertion per batch in `X-Sagat-Codec-Assertion`; render plaintext as text with a level marker
  that is not colour-only; keep it in ephemeral component state; clear it and abort requests when the view, run,
  namespace, tenant or session changes; never send a decoded value or an envelope to the platform; show distinct states
  for codec unavailable, customer session required, unauthorized, CORS refused, timeout, invalid response, level not
  permitted, context mismatch and key unavailable.
  Start-run and signal views MUST NOT submit plaintext for encoded locations (D1544).
- **FR-031-027**: `sagat workflow inspect <RUN> --values --decode` and `sagat workflow run <NAME> --encode` MUST use the namespace
  codec endpoint with the customer's own bearer token (`--codec-token-file`) and a platform assertion, neutralise control
  and bidirectional characters, never write decoded values to a file unless `--output` is given, and exit with a typed
  error when the codec server fails. The CLI holds no key.

**Classification interplay**

- **FR-031-028**: Under `encodingRules: 1`, disclosure (016) MUST validate a payload against the encoded view of its
  carrier (each encoded location's subschema replaced by the envelope schema), MUST return an envelope as
  `{kind: "encoded", encoding, level, pointer, envelope}` only when the request sets `includeEnvelopes: true` and the
  caller may receive that level (016 audience matrix; `secret` never), and otherwise apply the existing
  omission/`withheld` rules with reason `ENCODED`; a withheld encoded node carries no size. Envelopes are returned whole within a 256 KiB envelope budget per response (response ceiling
  320 KiB when envelopes are included). An assertion's `maxLevel` is `public` for `run:read`, `internal` with
  `payload:read-internal`, `pii` only with the grant and an explicit reveal, and never above the endpoint's ceiling.

**Inventory and cross-cutting**

- **FR-031-029**: An administrator with `codec:manage` MUST be able to start, read, list and cancel a key-usage scan of a
  namespace; the scan runs in the engine under a lease, at most one per namespace, in bounded batches whose progress
  and aggregates commit together, and reports per (`kid`, `kv`, algorithm, encoding) the envelope count and the oldest
  and newest payload creation time. Results contain no value and are retained for at most 50 scans per namespace.
- **FR-031-030**: Every new read and write MUST be scoped by the authenticated tenant in SQL; a foreign workflow, run,
  endpoint or scan is the opaque `404`; namespace API admission is charged once after authorization and never debits a
  foreign namespace; an assertion names exactly the caller's tenant and namespace.
- **FR-031-031**: The feature MUST emit the spans, fixed-label metrics and audit actions of the plan; logs, spans,
  metrics, audit records and error bodies MUST NOT contain a value, `data`, `iv`, `wk`, `kid`, `kv`, an assertion or a
  codec-server response; metric labels come from closed sets only.
- **FR-031-032**: Every new resource MUST have the defaults and hard ceilings of the plan; effective values are the
  minimum of request, namespace and deployment; zero refuses rather than meaning unlimited; admission fails before work.
  Encoded payload bytes count against the existing retained-content quotas (ADR-0027) without exemption.
- **FR-031-033**: IR, OpenAPI, worker protocol and AsyncAPI changes MUST be additive; versions with `encodingRules: 0`,
  their runs, older workers, older Studio builds and older SDKs MUST keep working; the canonical contracts are updated
  from the fragments in `contracts/` in the same delivery.
- **FR-031-034**: The delivery MUST ship `docs/guides/client-side-encryption.md` (five languages), the runbook
  `docs/runbooks/payload-encryption.md` (key rotation, KMS outage, codec-server outage, lost key, origin change, scan,
  activation and rolling upgrade, customer session setup), regenerated `docs/reference/` pages, and the happy/failure
  demo of `quickstart.md`.
- **FR-031-035**: For an attempt whose payload plan is non-empty, worker-written side channels — 019 checkpoint details
  (heartbeat and final failure report) and 026 stream appends — MUST be accepted only when the node's `heartbeatSchema`
  or the channel's `schema` is a carrier of a rules-1 version, and then pass the checks of FR-031-010; otherwise they are
  refused with `ENCODING_REQUIRED` without blocking lease renewal. Delivered checkpoints are part of the next lease's
  decode plan.
- **FR-031-036**: Publication of a document that declares encoding MUST be refused (`ENCODING_NOT_ACTIVATED`) until an
  audited operator command activates the feature through feature 018's instance-capability registry (FR-018-052), which
  succeeds only when every live control-plane and execution-engine instance reports `payload-encoding-v1`; once the
  requirement is recorded, a registered instance without it does not serve (018). Live instances without the capability
  are reported by the gauge `payload_encoding_incapable_members`. Validation of already published rules-1 versions
  never depends on activation.

### Non-Functional Requirements

- **NFR-031-SEC**: Zero plaintext canaries from declared locations in any platform store, log, span, metric, audit
  record, error or export, on accepted and refused writes, with only the two stated exceptions of SC-031-001; zero
  platform credentials sent to a codec server; zero decodes granted to a request without a customer session; zero
  cross-tenant or cross-namespace access to endpoints, assertions, plans, scans or envelopes. Each is release-blocking.
- **NFR-031-DUR**: No change to transition, scheduling, timer, subscription or outbox atomicity; validation happens inside
  the existing transactions; replay of the journal equals the projection with encoded payloads.
- **NFR-031-COMP**: Envelope v1 decodable by every future SDK release; additive contracts; pinned interpretation.
- **NFR-031-OBS**: Every refusal and codec operation is visible through closed-label metrics, spans and audit without
  sensitive data; SDK codec spans use each SDK's existing tracing hook.
- **NFR-031-PERF**: Measure on a stated environment the validation cost per completion (p50/p95/p99, 64 locations,
  256 KiB), retained-bytes growth, and SDK encode/decode latency with and without key caches. No unmeasured target.
- **NFR-031-A11Y**: Studio states and actions meet the Studio's WCAG 2.2 AA goal, keyboard path and text level markers,
  checked by tools and a keyboard walkthrough.
- **NFR-031-OPS**: Reproducible Compose profile with OpenBao Transit and the reference codec server for development
  only; production uses customer-run KMS and codec servers.

### Key Entities

- **Encoded location** and **payload plan** (pinned with the version).
- **Envelope v1** (wire and storage format; never parsed beyond its header by the server).
- **Namespace codec endpoint** (URL, origin, level ceiling and its setter, revision).
- **Codec assertion** (signed narrowing claims; single use; not stored, audited).
- **Encoded-field registry** (positions, carriers and write paths across drafts).
- **Encoding activation** and the capability of each engine and control-plane replica.
- **Key-usage scan** and its results.
- **Codec capability** of a worker session (negotiated minor plus advertised codec).
- **Audit actions**: `codec.endpoint_set`, `codec.endpoint_removed`, `codec.assertion_issued`, `codec.assertion_denied`,
  `payload.key_usage_scan_started`, `payload.key_usage_scan_finished`, `run.encoding_refused`,
  `payload.encoding_activated`, `payload.encoding_deactivated`.

## Success Criteria *(mandatory)*

- **SC-031-001**: Plaintext canaries placed at every declared location on every write path of FR-031-010 are absent
  from 100% of captured PostgreSQL rows (except the inbox row an event has at admission), committed blob objects
  (refused uploads are gone after the ADR-0023 sweep), logs, spans, metrics, audit records, error bodies and dead
  letters, both when the write is accepted (encoded) and when it is refused (plaintext), including 019 checkpoints and
  026 stream events of nodes with encoded locations. (FR-031-010..012, 014, 031, 035)
- **SC-031-002**: Each publication defect class is refused with its code and pointer; the sound corpus publishes with a
  stable digest on a second publish; an identical pre-feature document returns `encodingRules: 0`; the plan endpoint
  returns exactly the corpus plans; every registry row has a fixture that its consumer's check refuses with the
  registry's code (rows of drafts not yet merged run when they merge). (FR-031-001..008)
- **SC-031-003**: The five SDKs produce byte-identical annotated documents, and the 5×5 interoperability matrix decodes
  every vector to identical bytes; every tampered vector fails in all five. (FR-031-009, 016..018, 021, 022)
- **SC-031-004**: The runtime scenario passes in all five SDKs with equal goldens; the wire capture contains envelopes and
  no plaintext at declared locations. (FR-031-015..017, 022)
- **SC-031-005**: Rotation, disabled version, KMS outage, unknown `kid`, cache expiry and a secret in an output each
  produce the specified typed outcome and retry count; zero plaintext is persisted in any of them. (FR-031-018..021)
- **SC-031-006**: Crash at each commit point of the plan, duplicate start, signal and completion (identical and re-encoded bytes), lost
  broker hint, timeout vs signal with an encoded payload, failed compensation with an unavailable key and restart each
  yield the specified single outcome, and the journal replays to the projection. (FR-031-010, 011, 013, 029)
- **SC-031-007**: A real Chromium journey with the reference codec server decodes, and each codec failure state of
  FR-031-026 appears; no decoded value remains in DOM after close, browser storage, the query cache or platform traffic;
  accessibility checks pass; the CLI decodes and encodes through the same codec server and exits with a typed error when
  it is down. (FR-031-024, 026, 027)
- **SC-031-008**: The audience contract test (six roles × four levels × grant × reveal × `includeEnvelopes`) returns the
  specified node per cell; assertion `maxLevel` matches (capped by the ceiling); the reference codec server refuses
  every envelope above it, every request without a customer session, every replayed `jti` and every envelope whose
  `ctx` differs from the assertion. (FR-031-023, 025, 028)
- **SC-031-009**: Cross-tenant and cross-namespace attempts on endpoints, assertions, plans and scans are opaque `404` or
  `403` and leave the other namespace's rate bucket unchanged; an assertion of namespace A is refused by a codec server
  scoped to namespace B. (FR-031-030)
- **SC-031-010**: Exhausting each ceiling of the plan yields its typed refusal before work starts. (FR-031-032)
- **SC-031-011**: The codec-server black-box suite passes against the reference implementation; the merged OpenAPI,
  proto, IR schema and AsyncAPI pass their compatibility checks; the quickstart demo runs end to end with recorded output.
  (FR-031-023, 033, 034)
- **SC-031-012**: A benchmark report states the environment and reports the measurements of NFR-031-PERF and every failure.
  (FR-031-032, NFR-031-PERF)
- **SC-031-013**: In a mixed fleet, an older-minor or codec-less worker never receives a task with a decode or output
  plan; the task keeps its attempt count until a codec-capable worker completes it; in a rolling upgrade, activation is
  refused while any live replica lacks the capability, and publication with encoding is refused until activation.
  (FR-031-015, 036)
- **SC-031-014**: The carriage fault suite refuses, with `ENCODING_CARRIAGE_MISMATCH` or `ENVELOPE_CONTEXT_MISMATCH` and
  no run or step output created, each of: a child (rules 0 and rules 1 without the location), a 022 `fromRun` pin, a
  reset into another version, a 030 handler, a trigger start with an undeclared envelope, a re-rooted carriage, and an
  envelope pasted from another namespace or location; a crash between the mapping check and the admission commit leaves
  no child. (FR-031-005, 010, 011)

## Assumptions

- Customers run their own KMS (OpenBao Transit or a cloud KMS) and, for the Studio, their own codec server; the
  platform operator approves codec origins. Development uses a disposable Transit and the reference codec server.
- A worker or client that holds keys is trusted with plaintext by its owner; a malicious worker is outside the model.
- 016 classification, its disclosure endpoint and the reveal grants are available (merged in 0.2.0).
- Envelopes are JSON values in `payloads` like any other; no storage format changes.

## Dependencies

- **Existing:** 001 (contracts, identity), 002 (journal, payloads, blobs, run result), 003 (compensation, manual review),
  004 (signals, inbox matching, dead letters), 006 (SDKs, conformance), 007 (Studio, CSP), 008 (telemetry), 009 (roles,
  audit, quotas), 011 (publication, diff), 012 (CLI), 014 (IR primitives, Query/Update), 016 (classification,
  disclosure), 017 (OpenBao precedent); ADR-0002, 0014, 0016, 0017, 0023, 0027, 0028, 0031, 0032, 0033, 0035.
- **New features (drafts in progress):** every draft in the [registry](contracts/encoded-field-registry.md) (018, 019,
  020, 021, 022, 024, 025, 026, 027, 030) cites it and uses its codes (coordinator C7); 028 only for interceptor ordering
  (C5: 031 owns the only payload-transforming stage); 019 and 026 for the protocol minor and the side-channel carriers
  (`heartbeatSchema`, stream channel schemas; C6 amendment of FR-016-002); 018 for the instance-capability registry
  (FR-018-052, ADR-0050 decision 9) through which activation works; 020 for the C9 egress rule by envelope level; 027/ADR-0062 for the combined Studio CSP (C15); 032 for the key
  inventory entry of the assertion keys (C11); 030 encoded values do not cross namespaces (D1667).
- **Draft not required:** feature 015 (OpenBao Transit for platform keys) would hold the assertion signing key when it
  lands (D1555).

## Out of Scope

- Server-side decryption, a decode proxy, platform-held customer keys, a "trusted executor" with customer keys (G1529).
- Encoding of search attributes, workflow ids, memo-like metadata, failure codes, journal facts or audit records.
- Re-encryption (rewrap) of retained payloads; deterministic or searchable encryption.
- Studio encoding (start/signal forms over encoded fields); per-user codec endpoint overrides.
- Field-level encoding of event data without a declared `waitEvent` and of declared Query/Update results (G1524,
  G1530); carriage of encoded values across namespaces (030, G1536).
- First-party cloud KMS providers unless O1520 decides otherwise; a supported codec-server product (O1522).
- Retention, erasure and data residency policy (G69, G283, G284).

## Owner Questions

See [research.md](research.md#owner-questions): O1520 (first-party key providers, default OpenBao Transit plus test
provider), O1521 (codec authentication: the customer's identity provider authenticates the person and the platform
assertion only narrows — adopted by the coordinator on 2026-10-27, pending the owner), O1522 (reference codec server
status, default example plus conformance). The decision this specification proposes is
[ADR-0066](../../docs/adr/ADR-0066-client-side-payload-encryption.md) (Proposed).
