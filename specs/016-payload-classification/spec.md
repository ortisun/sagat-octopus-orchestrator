# Feature Specification: Per-field payload classification

**Feature Branch**: `feature/016-payload-classification`
**Created**: 2026-10-16
**Status**: Implementation in progress; integrated release gates pending. The direction is accepted in [ADR-0033](../../docs/adr/ADR-0033-payload-values-are-classified-per-field.md); this set turns it into requirements, a plan and tasks. The four product choices were answered by the owner on 2026-10-17 ([research.md](./research.md#product-decisions-answered-2026-10-17)); nothing in this set is an open product question, and what is still undecided is listed there.
**Input**: Deliver ADR-0033: the author classifies the values of a workflow as `public`, `internal`, `pii` or `secret` on the JSON Schemas of the workflow, and the platform discloses a value only to a principal the level allows.

## Context and delivery boundary

Today no REST call returns the value of a run payload. The run, its journal and its runtime graph carry
a `payloadRef` digest and a failure code; an Admin or Operator may inspect the *structure* of the run
input or of one journal payload (`POST /v1/runs/{runId}/payload-inspections`, policy
`structure-only-v1`, [ADR-0028](../../docs/adr/ADR-0028-authorized-structural-payload-inspection.md));
a declared Query or Update result is disclosed on its own terms (ADR-0031, feature 014). Operators
asked to see values to diagnose a run, and the run result that G21 stores
(`runs.result_digest`, on `bugfix/run-result`, not yet merged) has no disclosure rule. ADR-0033 decides the rule; it does not build it.

This feature builds it and nothing else. It adds:

1. an annotation, `x-sagat-classification`, on the JSON Schemas the workflow IR already carries, plus
   an optional `outputSchema` on the nodes that produce an output (the IR has no schema for them
   today);
2. publication checks that make the annotation sound, and a pin of the interpretation to the
   published version;
3. a new, audited disclosure operation, `POST /v1/runs/{runId}/payload-disclosures`, policy
   `classified-v1`, beside the structural one, which stays as it is;
4. the permissions that gate the levels, and a way to grant `payload:reveal-pii` while the built-in
   roles stay six;
5. the same disclosure in the Studio run views and in `sagat workflow inspect`;
6. a canary suite that proves a `secret`, unmarked or non-validating value crosses no boundary;
7. a restriction of the event values that REST returns today: `getInboxEvent` `data` and connector
   dead-letter `body` and `headers` stop being returned to plain `event:read` holders and require the
   same reveal grant (US08).

What it leaves alone, and says so: declared Query and Update results (ADR-0031, unchanged), the
structure-only inspection endpoint (ADR-0028, unchanged), the ordinary run, journal, graph and stream
reads (payload-free, unchanged), the worker protocol, and retention per customer or regulator
(ADR-0033, point 6). One existing surface disclosed values before this feature: the event inbox
(`getInboxEvent` returned `data`, connector dead letters returned `body` and `headers`, both to any
holder of `event:read`, viewers included). That contradicted the premise of ADR-0033, and the owner
decided on 2026-10-17 (D302) to restrict it in this increment: the values now need the reveal grant,
everyone else gets structure only. The restriction is a breaking change for readers of those fields
(FR-016-028). Field-by-field classification of events waits for event type schemas (G282); until then
an event's data is `pii` as a whole.

## Domain vocabulary

- **Level**: one of `public`, `internal`, `pii`, `secret`, ordered from least to most strict.
- **Carrier**: a JSON Schema in the workflow document where the annotation is read. The set is closed:
  the workflow `inputSchema`, the workflow `outputSchema` (the schema of the run result) and the new
  optional `outputSchema` of an `activity`, `script`, `waitEvent` or `child` node.
- **Effective level**: the level of one value after the rules of FR-016-003, computed by walking the
  value against its carrier.
- **Disclosure**: returning a value to a caller. Structure (types, counts) is not disclosure; it keeps
  the ADR-0028 rules and its own permission.
- **Reveal**: a disclosure of `pii` values. It is requested explicitly, needs `payload:reveal-pii` and
  is audited every time.
- **Classification rules version**: the version of the interpreter that was current when a workflow
  version was published; it is stored with the version so that a later release cannot reinterpret an
  old document (FR-016-005).

## User Scenarios & Testing *(mandatory)*

### US01 — Classify the values of a workflow when authoring it (Priority: P1)

An author marks the fields of the input, the result and the step outputs with a level, publishes, and
knows that the classification of that version never changes.

**Why this priority**: nothing can be disclosed without it, and a wrong annotation must fail at
publication, not at the first disclosure.

**Independent Test**: publish a document with annotations of every level and every defect class
(unknown level, a descendant looser than its parent, an annotation under an unsupported keyword, a
`$ref` leaving the schema, an annotation in a non-carrier location); the sound document publishes with a
stable digest, each defect is refused with a precise path, and a later publish does not change what a
run of version 1 discloses.

**Acceptance Scenarios**:

1. **Given** an `inputSchema` whose root is `public`, `amount` is `internal` and `customer` is `pii`
   with a `secret` child, **when** the author validates and publishes, **then** it is accepted and
   `validate` lists the effective level of every declared path.
2. **Given** a `pii` object with a child annotated `public`, **when** the author validates, **then**
   the document is refused with `CLASSIFICATION_LOOSENED` at the child's path; the same through a `$ref`.
3. **Given** a root with no annotation and a child annotated `public`, **when** validated, **then** it
   is refused with the same code and a message that says the unmarked root is `pii`.
4. **Given** an annotation inside a `patternProperties` branch, inside the schema of a Query result or
   inside the declared state, **when** validated, **then** it is refused (`CLASSIFICATION_UNSUPPORTED_KEYWORD`
   or `CLASSIFICATION_NOT_APPLICABLE`) rather than silently ignored.
5. **Given** version 1 with a `public` field and version 2 that makes it `pii`, **when** a run of
   version 1 is disclosed after version 2 is active, **then** the field is still `public`; in the
   reverse edit (version 2 makes a `pii` field `public`) the field stays `pii` for every run of version 1.
6. **Given** a version published before this feature, **when** any of its runs is disclosed, **then**
   no value is returned to anyone, exactly as today (classification rules version 0). **And given** the
   author publishes the identical document again, **then** the answer is that same version
   (`created: false`) and says `classificationRules: 0`; a changed document is published as a new
   version under the current rules.
7. **Given** a diff between two versions that relaxes or tightens a level, **when** it is requested,
   **then** it lists an attention finding for each (`CLASSIFICATION_RELAXED`, `CLASSIFICATION_TIGHTENED`).

### US02 — Read the public and internal values of a run (Priority: P1)

An operator or developer diagnosing a run reads the values the author allowed them to read, in the REST
API and in `sagat workflow inspect`, and a viewer reads only what is `public`.

**Why this priority**: it is the operational need that started ADR-0033.

**Independent Test**: run a workflow whose input and result mix `public`, `internal`, `pii` and `secret`
fields, then disclose both as each of the six built-in roles and compare with the matrix of
[contracts/README.md](./contracts/README.md#audience-matrix).

**Acceptance Scenarios**:

1. **Given** a finished run and a viewer, **when** the viewer discloses the run input, **then** only
   `public` values are returned; nothing says that other fields exist.
2. **Given** the same run and a developer, **when** they disclose the input, **then** `public` and
   `internal` values are returned and the `pii` and `secret` fields are absent.
3. **Given** an operator (who also holds `run:payload:inspect`), **when** they disclose the input,
   **then** `public` and `internal` values are returned and the `pii` and `secret` fields appear as
   withheld nodes with their type and level but no value and no undeclared key.
4. **Given** a run that reached `end`, **when** the result is disclosed (`source: "result"`), **then**
   the same rules apply with the workflow `outputSchema`; a run that ended without a result answers
   `unavailable`.
5. **Given** a `worker` or `runner` identity, **when** it calls the operation, **then** it is refused
   with `FORBIDDEN` (it holds no `run:read`).
6. **Given** a payload that does not validate against its schema, **when** a viewer or developer
   discloses it, **then** the answer is `unavailable` (`NOT_DISCLOSABLE`) with no hint why; an operator
   gets the ADR-0028 structure with `reason: SCHEMA_MISMATCH` and no value.
7. **Given** `sagat workflow inspect <RUN> --values`, **when** it runs as a developer, **then** it
   prints the disclosed values of the input and the result with control characters neutralised, and
   prints `[withheld: pii]` where the server withheld.

### US03 — Reveal personal data under an explicit, audited grant (Priority: P1)

A principal who holds `payload:reveal-pii` in a namespace asks to reveal the `pii` values of a run;
every reveal is audited before any value leaves the server, and an administrator can grant and revoke
the right.

**Why this priority**: the sensitive half of ADR-0033; without it `pii` is simply unreadable.

**Independent Test**: grant, reveal, revoke, reveal again; count audit records; kill the audit write and
confirm no value is returned.

**Acceptance Scenarios**:

1. **Given** a developer without a grant, **when** they send `reveal: ["pii"]`, **then** the answer is
   `403 FORBIDDEN` with `details.reason = PII_REVEAL_NOT_GRANTED`, no value is returned and a denial is
   audited (best effort).
2. **Given** an administrator who grants the right to a developer in a namespace (`PUT
   .../payload-reveal-grants/{subject}`), **when** the developer reveals, **then** `public`, `internal`
   and `pii` values are returned, `secret` is not, and one `payload.pii_revealed` audit record exists
   that names the actor, run, selector and trace but no value, key or digest.
3. **Given** the same developer who sends no `reveal` field, **when** they disclose, **then** no `pii`
   value is returned (an operator who holds the grant would see the node withheld with
   `REVEAL_NOT_REQUESTED`) and no reveal is audited.
4. **Given** two identical reveals within a minute, **when** both succeed, **then** two audit records
   exist (a reveal never collapses).
5. **Given** the audit insert fails or its transaction does not commit, **when** a reveal is requested,
   **then** no value is returned and the answer is `503 UNAVAILABLE`.
6. **Given** a revoked grant, **when** the next request arrives, **then** `pii` is refused as in
   scenario 1; a request already past its authorization check may complete (the check and the read
   share one transaction, so there is no later window).
7. **Given** an administrator, **when** they have not been granted the right, **then** they cannot
   reveal: `admin` does not hold `payload:reveal-pii` by role. They may grant it to themselves; that
   grant is audited like any other (D306).
8. **Given** a grant in namespace A, **when** the subject reveals a run of namespace B, **then** it is
   refused; **given** a subject whose membership is removed, **then** its grants are removed with it.

### US04 — A secret, an unmarked value or an invalid value never leaves (Priority: P1)

Whatever the caller asks for, no `secret` value, no value governed by no annotation without a reveal and no
value of a payload that fails its schema crosses a boundary: response, error, log, span, metric, audit
record, stream, CLI output, Studio DOM, browser storage.

**Why this priority**: constitution V is release-blocking for secret exposure; this is the proof.

**Independent Test**: seed canary strings at every level, in values, in object keys, in array members and
in a payload that violates its schema; exercise every surface; assert no canary of a forbidden class is
found in any captured byte, for every role including admin with a reveal grant.

**Acceptance Scenarios**:

1. **Given** a `secret` field holding a canary and a caller with every permission and a grant, **when**
   any disclosure, inspection, journal, graph, stream, telemetry, audit, CLI or Studio surface is read,
   **then** the canary is absent from every response and every captured log, span and metric.
2. **Given** a schema with no annotation at all, **when** a developer discloses a conforming payload,
   **then** no value is returned (everything is `pii`); a revealer sees the values.
3. **Given** an undeclared property under a `public` object (the schema allows additional
   properties), **when** a viewer discloses, **then** the extra value is not returned (it is `pii`).
4. **Given** a validation failure, **when** the response and every log of the request are read, **then**
   none contains the instance value, the instance path or a parser or validator message.
5. **Given** a rerun, a child run or a retry, **when** their payloads are disclosed, **then** each is
   judged by the classification of the version its own run pins, never by the parent's.
6. **Given** an object key that is data (an undeclared key) holding a canary, **when** a caller who
   cannot see the value discloses, **then** the key is absent too.

### US05 — Read the output of a step (Priority: P2)

An operator reads the classified output of one step by journal sequence (an activity, a script, a
signal that woke a `waitEvent`, a reconciliation, a child's result), judged by the `outputSchema` the
author put on that node.

**Why this priority**: the run result is not enough to diagnose a failing step, but node-level schemas
are new IR and the first increment can ship without them.

**Independent Test**: publish a workflow whose `charge` activity has an annotated `outputSchema`, run it,
and disclose the `step.completed` sequence as each role.

**Acceptance Scenarios**:

1. **Given** an activity with an `outputSchema`, **when** `{source: "journal", sequence}` names its
   `step.completed` event, **then** the output is judged by that schema and the node's classification.
2. **Given** an activity with no `outputSchema`, **when** its output is disclosed, **then** an operator
   gets the ADR-0028 structure with `reason: NO_SCHEMA`, anyone else `unavailable`.
3. **Given** a sequence whose event carries no payload, **when** disclosed, **then** the answer is
   `unavailable` with `NO_RETAINED_PAYLOAD`; no other event's payload is ever substituted.
4. **Given** an output stored as an external blob, **when** disclosed, **then** the answer is
   `external` with `EXTERNAL_CONTENT_NOT_INSPECTED`; the blob is not read.
5. **Given** a `child` node with `outputSchema`, **when** the parent's step completes with the child's
   result, **then** the parent's output is judged by the parent's node schema, not the child's.

### US06 — See the values in the Studio run view (Priority: P2)

A person in the run view sees, in the same panel as the structural inspection, the values their role
allows, a clear marker for each level, and an explicit "Reveal personal data" action only when they hold
the right.

**Why this priority**: the Studio is how most operators look at a run (feature 007).

**Independent Test**: Studio component tests plus a real Chromium journey against the real API and
engine, with canaries, as a viewer, a developer, an operator and a revealer.

**Acceptance Scenarios**:

1. **Given** a run with mixed levels and a developer, **when** they open the input, **then** `public`
   and `internal` values show with a level marker that is not colour-only, and `pii` and `secret` are absent.
2. **Given** a revealer, **when** they press "Reveal personal data" and confirm, **then** a request with
   `reveal: ["pii"]` is sent, the values show, and leaving the view or changing the run, tenant or
   session clears them.
3. **Given** any user, **when** the page is inspected, **then** the DOM, `localStorage`,
   `sessionStorage`, IndexedDB, the query cache and the network log contain no value the server
   withheld, and no revealed value persists after the view closes.
4. **Given** a Studio built before this feature talking to a newer server, **when** it runs, **then**
   it keeps working (the new operation is additive and the old structural one is unchanged).

### US07 — Review the classification before it is published (Priority: P3)

An author, a reviewer and a CLI user see the effective level of every declared path, the paths that
inherit an open level, and what changed against the previous version.

**Why this priority**: inheritance makes a root annotation broad; a report keeps that visible.

**Independent Test**: `validate` on a document with an annotated root and unannotated children lists each
path with its level and a warning for each inherited path; the Studio settings panel and `sagat workflow
validate` show the same.

**Acceptance Scenarios**:

1. **Given** a `public` root with five unannotated declared children, **when** validated, **then** five
   `CLASSIFICATION_INHERITED_OPEN` warnings name them (not errors).
2. **Given** a publication request under an approvals policy (feature 011), **when** the reviewer opens
   the diff, **then** relaxations and tightenings of levels are listed as attention findings.

### US08 — Event values need the reveal grant (Priority: P1)

A person who reads an inbox event or a connector dead letter sees its status, attempts and structure, and
sees the values only when they hold the reveal grant of the namespace and ask for them; every such reveal
is audited.

**Why this priority**: the event inbox is the one surface that returned values to every viewer; the
owner decided to close it now (D302) rather than wait for event schemas.

**Independent Test**: publish an event and set aside a dead letter that carry canaries; read both as a
viewer, an operator, a developer without a grant and a developer with a grant, with and without
`reveal=pii`; count the audit records; read an event of another tenant and one of another namespace.

**Acceptance Scenarios**:

1. **Given** an inbox event and a caller who holds `event:read` but no reveal grant, **when** they call
   `getInboxEvent` with no `reveal`, **then** the answer has the same status, attempts, last error and
   deliveries as before, no `data`, `dataDisclosure: "structure-only"`, the data size and the ADR-0028
   structure of the data (types and counts, no key, no scalar).
2. **Given** a developer holding the grant in the event's namespace, **when** they call it with
   `reveal=pii`, **then** `data` is returned whole (events have no field classification yet, so all of it
   is `pii`), `dataDisclosure` is `revealed`, the response is `Cache-Control: private, no-store` and one
   `payload.pii_revealed` record exists for the event, with no value in it.
3. **Given** a caller without the grant, **when** they send `reveal=pii`, **then** the answer is `403
   FORBIDDEN` with `details.reason = PII_REVEAL_NOT_GRANTED`; the same for a `viewer` who holds a grant
   (the role lacks `payload:read-internal`, so the grant is inert).
4. **Given** a connector dead letter, **when** it is listed, **then** no item carries `body` or
   `headers`; each carries its size, `headerCount` and, when the body is bounded JSON, its structure
   (otherwise `opaque`), and the list ignores `reveal`.
5. **Given** one dead letter and the grant, **when** it is read with `reveal=pii`, **then** `body` and
   `headers` are returned and one `payload.pii_revealed` record names the connector and the letter; without
   the grant the same call is the `403` of scenario 3.
6. **Given** an event, connector or dead letter of another tenant, or a grant held for another
   namespace, **when** it is read, **then** the answer is the opaque `404` or the `403` above, and the
   attempt leaves the other namespace's rate bucket unchanged.
7. **Given** an integration that read `data` or `body` as `event:read` before, **when** it runs against
   the new server, **then** it receives the structure answer and no value; the change is listed in the
   release notes and the API reference.
8. **Given** redrive or discard of a dead letter (`event:manage`), **when** they run, **then** they behave
   as before and return no value.

### Edge Cases

- A payload larger than 256 KiB, a schema larger than 64 KiB, depth beyond 16, a `$ref` cycle, a
  `$ref` to a remote or another carrier: refused at publication (`CLASSIFICATION_TOO_COMPLEX`,
  `CLASSIFICATION_UNRESOLVED_REF`) or, for an old stored document, answered `structure-only`
  (`SCHEMA_UNUSABLE`).
- A payload that validates only by a branch of `oneOf` or `anyOf`: the strictest level across all
  branches applies at that location, whichever branch matched.
- An empty object or array, `null`, a string longer than 4 KiB (truncated with its length), a number
  beyond the response budget.
- A revealer's grant revoked, or the membership deleted, between the authorization check and the read:
  both are one transaction; the outcome is the state at that transaction's snapshot.
- A rerun (`RERUN_AS_NEW`) of a run of an old version: the rerun pins the same version and so the same
  classification.
- A document with the annotation key in a place the engine of an older release does not interpret: that
  release ignores it (the base node schema allows extra keys), and classification rules version 0
  never reads it.
- Two concurrent reveals of the same payload: two audit records, no shared state.
- A workflow whose `outputSchema` is absent: the result is `NO_SCHEMA` (structure for an operator,
  nothing for others).
- Unicode, control characters and bidirectional text in values and keys: returned as JSON strings
  unchanged by the server; the CLI neutralises them, the Studio renders them as text.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-016-001**: An author MUST be able to annotate any node of a carrier schema with
  `"x-sagat-classification"`, whose value is exactly one of `"public"`, `"internal"`, `"pii"`,
  `"secret"`. The level is ordered `public < internal < pii < secret`.
- **FR-016-002**: The carriers MUST be exactly: the workflow `inputSchema`; the workflow `outputSchema`
  (the schema of the run result); and an optional `outputSchema` on `activity`, `script`, `waitEvent`
  and `child` nodes. The new node property is additive inside IR `v1` (ADR-0032 keeps additive keys in
  the same `apiVersion`); no `apiVersion` change, and no new key on any other node.
- **FR-016-003**: The effective level of a value MUST be computed by walking it against its carrier: an
  annotated schema node sets the level of the values it governs; an unannotated declared node inherits
  its parent's effective level; the root of a carrier with no annotation is `pii`; a value no schema
  node declares (an undeclared property, an item beyond `prefixItems` with no `items`, anything under a
  keyword outside the supported set) is at least `pii` (the effective level is the stricter of the
  inherited level and `pii`); when several subschemas (`allOf`, `anyOf`, `oneOf`, `$ref`) govern one
  location, the strictest annotation among all of them applies, without choosing a branch by the data.
  The supported keywords are `type`, `properties`, `additionalProperties` (as a schema or boolean),
  `items`, `prefixItems`, `$ref` to a pointer inside the same carrier, `$defs`, `allOf`, `anyOf`,
  `oneOf`. A descendant never lowers the effective level (see FR-016-004 and the runtime clamp).
- **FR-016-004**: Publication and `validate` MUST reject, with the problem code and the JSON pointer of
  the offender: an unknown level (`CLASSIFICATION_INVALID_LEVEL`); a node that declares a level looser
  than its effective parent level, inline or through `$ref` (`CLASSIFICATION_LOOSENED`); an annotation
  under any key the walker does not read, that is any key other than `properties`, `items`,
  `prefixItems`, `additionalProperties`, `allOf`, `anyOf`, `oneOf`, `$defs` or a local `$ref` target
  (`CLASSIFICATION_UNSUPPORTED_KEYWORD`, D660); a `$ref` that is
  remote, absolute, to another carrier or unresolvable (`CLASSIFICATION_UNRESOLVED_REF`); an annotation
  anywhere but a carrier, including declared state and the schemas of Query and Update
  (`CLASSIFICATION_NOT_APPLICABLE`); and a carrier over the bounds of FR-016-024
  (`CLASSIFICATION_TOO_COMPLEX`). A declared child that inherits an open level (`public` or `internal`)
  from an annotated container MUST produce the warning `CLASSIFICATION_INHERITED_OPEN`, whichever
  keyword declares it (`properties`, `prefixItems`, `items` or `additionalProperties`). Only errors are
  listed in `details.issues` of a refused document; warnings belong to the validation report.
- **FR-016-005**: The classification MUST be immutable with the published version and pinned by every
  run. A version stores the classification rules version in force when it was published
  (`workflow_versions.classification_rules`; `0` for every version published before this feature, which
  reads no annotation). No operation edits the document or the stored rules version of a version. Every version the API
  returns carries it as `classificationRules`. Publishing the document of an existing version returns
  that version (found by workflow and digest, `created: false`) with its own rules, so an unchanged
  legacy document stays at `0`; a changed document is published under the current rules (D661).
- **FR-016-006**: The platform MUST provide `POST /v1/runs/{runId}/payload-disclosures` with the closed
  body `{"source":"input"}`, `{"source":"result"}` or `{"source":"journal","sequence":N}`, and the
  optional `"reveal":["pii"]`. Unknown fields are refused. There is no client-supplied tenant,
  namespace, digest, blob id, JSON path or level override. Successful answers carry
  `Cache-Control: private, no-store`. The ADR-0028 endpoint and its response MUST NOT change.
- **FR-016-007**: The operation MUST disclose a value only to the audience of its effective level:
  `public` to any principal that holds `run:read`; `internal` to a principal that also holds
  `payload:read-internal` (granted to `developer`, `operator`, `admin`); `pii` only when the request
  asks to reveal `pii`, the caller holds `payload:reveal-pii` in the run's namespace and also holds
  `payload:read-internal` (no principal sees a stricter level than a looser one); `secret` to nobody,
  whatever the request.
- **FR-016-008**: A value MUST be disclosed only if the whole payload validates against its carrier
  schema, checked with a validity test that produces no message (never an error iterator whose text
  repeats the instance). Otherwise the answer is `structure-only` (ADR-0028 structure, no scalar and no
  key) for a caller that holds `run:payload:inspect`, and `unavailable` with `NOT_DISCLOSABLE` for any
  other caller. The same holds when the payload has no carrier (`NO_SCHEMA`), when the version has
  classification rules `0` (`LEGACY_VERSION`) and when the stored schema cannot be compiled
  (`SCHEMA_UNUSABLE`).
- **FR-016-009**: A node the caller may not read MUST be omitted for a caller without
  `run:payload:inspect`, and represented as a `withheld` node with its type, level and reason (no
  value, and no key unless the key is declared in `properties`) for a caller who holds it. A `secret`
  node follows the same split: it is omitted for a caller without `run:payload:inspect` and withheld,
  with the reason `SECRET`, for one who holds it; its value is shown to nobody. Because levels never
  loosen, a withheld node hides its whole subtree.
- **FR-016-010**: The catalog MUST gain `payload:read-internal` and `payload:reveal-pii`. The built-in
  roles remain six and no custom role is added (G39). `payload:read-internal` is granted to `developer`,
  `operator` and `admin`. `payload:reveal-pii` is granted to no role, `admin` included; it is held by a
  principal through a grant (FR-016-011), and is always combined with `run:read`.
- **FR-016-011**: A principal that holds `membership:manage` MUST be able to list, grant and revoke
  `payload:reveal-pii` for a subject of the tenant in one namespace
  (`/v1/namespaces/{namespaceId}/payload-reveal-grants[/{subject}]`), and the CLI MUST offer the same.
  Each change is audited in the transaction that makes it and is effective on the next request. A grant
  is removed with its membership, and when the role of the membership changes (a demotion that is
  undone later does not bring the grant back; an administrator grants again, D663), and is refused for a subject whose role lacks `payload:read-internal`
  (such a grant would be inert). At most 256 grants exist per namespace. The mechanism, a per-namespace
  grant list managed by administrators, was decided by the owner on 2026-10-17 (D275); an administrator
  may grant it to themselves, audited (D306).
- **FR-016-012**: Every reveal MUST be audited as `payload.pii_revealed`, with actor, tenant, run, the
  server-built reason `classified-v1:<selector>:reveal=pii` and the trace id, committed in the same
  transaction that selects the payload and before any byte is returned; it never collapses repeats.
  A disclosure that returns at least one `internal` value is audited as `payload.disclosed` and
  collapses identical repeats for 300 seconds, as ADR-0028 does. A refused reveal is audited as
  `payload.reveal_denied`, best effort, never authorizing on failure. No audit record holds a value, a
  key, a digest, a schema or free text from the caller.
- **FR-016-013**: `source: "result"` MUST disclose the run result stored for G21 (`runs.result_digest`),
  judged by the workflow `outputSchema`; a run without a result is `unavailable`
  (`NO_RETAINED_PAYLOAD`). This selector depends on the G21 change being merged (see Assumptions).
- **FR-016-014**: `source: "journal"` MUST resolve only the payload of the named sequence, as ADR-0028
  does, and judge it by the `outputSchema` of the node of that event; it MUST NOT substitute another
  event's or attempt's payload. Events that carry a payload are `step.completed`, `wait.satisfied`,
  `reconciliation.concluded` and G21's `child.finished`. The latter uses only the exact parent's
  `child` node output schema, never the child's workflow schema or another node kind.
- **FR-016-015**: An external (blob) payload MUST NOT be read for disclosure; the answer is `external`
  with `EXTERNAL_CONTENT_NOT_INSPECTED` and the catalog size, as ADR-0028 does.
- **FR-016-016**: The following MUST remain unchanged by this feature: `getRun`, `listRunJournalEvents`,
  the runtime graph, graph changes, the stream, `getRunTelemetry`, `listRunAttempts` (all payload-free),
  `inspectRunPayload` (structure-only), and the declared Query/Update results of ADR-0031. A Query or
  Update result is never filtered by this feature, and a classification annotation inside its result
  schema is a publication error (`CLASSIFICATION_NOT_APPLICABLE`) so that nobody believes it protects
  anything there. The event inbox and the connector dead letters are the one exception and change as FR-016-026 and
  FR-016-027 say.
- **FR-016-017**: Telemetry, structured logs, metrics, spans, audit records, error bodies and the
  `Debug` output of every type that carries a payload MUST NOT contain a value, a key of a payload or a
  schema fragment, at any level. Metric labels MUST come from closed sets and never include a tenant,
  run, node, subject, key or value.
- **FR-016-018**: The Studio MUST render only the server's projection, never receive a value the server
  withheld, mark levels with text or shape in addition to colour, ask for a reveal only through an
  explicit confirmed action available when the caller may reveal, abort pending requests and clear
  revealed values when the run, event, tenant or session changes, and keep values out of every browser
  store and persisted query cache.
- **FR-016-019**: `sagat workflow inspect <RUN>` MUST keep its current output and gain
  `--values [--reveal-pii] [--sequence N]`, which uses the operation, neutralises control and
  bidirectional characters, and prints `[withheld: <level>]` for a withheld node. `sagat workflow
  validate` MUST print the classification report. `sagat namespace reveal-grants list|grant|revoke`
  covers FR-016-011.
- **FR-016-020**: The five SDK workflow builders MUST accept the new node-level `outputSchema` and the
  workflow `outputSchema`, and produce byte-identical canonical documents for the same classified
  workflow (a shared conformance scenario). No annotation helper is added: the annotation is a key of
  the free-form schema map every builder already accepts.
- **FR-016-021**: Existing published versions MUST keep today's behaviour (no value disclosed), existing
  runs MUST keep running unchanged, the OpenAPI and IR changes MUST be additive, and a client or Studio
  built before this feature MUST keep working.
  Restoration of immutable versions MUST NOT reapply newer classification authoring checks.
  An extension accepted by an older publisher remains inert for business execution; rules `0`
  stays opaque. Syntax, graph, expression and execution capability checks remain mandatory.
- **FR-016-022**: Errors MUST reuse the closed `Error.code` set and add only `details.reason` values:
  `PII_REVEAL_NOT_GRANTED`, `CLASSIFICATION`, `UNKNOWN_SUBJECT`, `ROLE_CANNOT_REVEAL`, `GRANT_LIMIT`. Result states
  (`structure-only`, `unavailable`, `external` and their reasons) are not errors.
- **FR-016-023**: Every read and write MUST be scoped by the authenticated tenant in its SQL join; a
  missing or foreign run, sequence or subject is the same opaque `404`; a cross-tenant attempt is
  audited through the existing restricted path; the namespace API admission of ADR-0027 is charged
  once, after authorization and run resolution, and never debits a foreign namespace.
- **FR-016-024**: Classification MUST be evaluated per response within fixed bounds: payload 256 KiB;
  schema 64 KiB per carrier; schema nesting 32; `$ref` expansions 1,000 per walk; response depth 16;
  1,024 nodes; 128 children per container; strings cut at 4 KiB; 128 KiB of disclosed string bytes;
  response 256 KiB; request body 1 KiB. Compiled validators are cached per process by
  `(workflow version id, carrier)` with a bounded size. These are design bounds to measure (SC-016-008),
  not capacity promises.
- **FR-016-025**: A version diff MUST list a relaxation (a path's effective level becomes less strict)
  as `CLASSIFICATION_RELAXED` and a tightening as `CLASSIFICATION_TIGHTENED`, both as attention findings
  (not breaking). A side that has no schema for a location (a carrier added or dropped), or that was
  published under classification rules `0`, discloses nothing and counts as `undisclosed`, as strict as
  `secret`: adding an `outputSchema` that makes a field `public` relaxes it. The first draft of a
  workflow is not compared. A carrier that cannot be walked completely is reported once
  (`CLASSIFICATION_TOO_COMPLEX`, `_UNRESOLVED_REF` or `_UNSUPPORTED_KEYWORD`) and none of its levels is
  compared; findings of one kind past 100 are counted in `MORE_FINDINGS`. Each side is walked once, so
  the cost grows with the size of the carriers (D664).

- **FR-016-026**: `getInboxEvent` MUST return the `data` of the event only when the request carries
  `reveal=pii` and the caller holds `event:read`, `payload:read-internal` and `payload:reveal-pii` in the
  event's namespace. The reveal is audited as `payload.pii_revealed` (`ResourceType::Event`, id
  `source/event id`, reason `event-data:reveal=pii`) in the transaction that reads the payload, before the
  bytes return, and never collapses. In every other case the answer has no `data` and carries
  `dataDisclosure: "structure-only"`, `dataSizeBytes` and `dataStructure` (the ADR-0028 structure with
  its bounds: no key, no scalar; `{"kind": "opaque"}` when the projector does not accept the data,
  which is not a refusal of the caller, D665). `reveal=pii` without the right is `403 FORBIDDEN` with
  `details.reason = PII_REVEAL_NOT_GRANTED`. Status, attempts, last error and deliveries are unchanged.
  Both answers are `Cache-Control: private, no-store`. Event data has no classification carrier, so a
  reveal returns all of it, `secret`-looking content included (G291, G282).
- **FR-016-027**: `getConnectorDeadLetter` and `listConnectorDeadLetters` MUST return neither `body` nor
  `headers` unless the single-resource call carries `reveal=pii` under the conditions of FR-016-026
  (audited, `ResourceType::Connector`, reason `dead-letter:<letter id>:reveal=pii`). Otherwise each
  letter carries `bodyDisclosure: "structure-only"`, `bodySizeBytes`, `bodyStructure` (the ADR-0028
  structure when the body is bounded JSON, otherwise `{"kind": "opaque"}`) and `headerCount`. The list
  never returns a body or header, with or without the grant (a page can hold 200 letters of 64 KiB), and
  ignores `reveal`. `position`, `reason`, `truncated`, `receivedAt` and the letter id are unchanged;
  `redriveConnectorDeadLetter` and `discardConnectorDeadLetter` are unchanged and return no value.
- **FR-016-028**: The restriction of FR-016-026 and FR-016-027 is a breaking change for any client that
  reads `data`, `body` or `headers` as a plain `event:read` holder: they receive the structure answer. It
  is documented in the OpenAPI description of the three operations (`data`, `body` and `headers` leave
  the `required` lists and become conditional), in `docs/runbooks/events.md`, in `docs/reference/api.md`
  and in the release notes. There is no transitional switch. A client that needs the values holds the
  grant and sends `reveal=pii`. Nothing that writes events (publish, signal, webhook, connector) changes.
- **FR-016-029**: Event and dead-letter reads MUST be scoped by the authenticated tenant in their SQL; an
  event, connector or letter of another tenant is the opaque `404`, and a namespace of another tenant
  leaves a `CrossTenantAccessDenied` record against the tenant that probed, for `getInboxEvent`,
  `getConnectorDeadLetter` and `listConnectorDeadLetters` (constitution V, D666); the grant is looked up for the event's
  or connector's own namespace, so a grant in another namespace reveals nothing; namespace API admission is
  charged as today (`ApiOperation::EventRead`) and never debits a foreign namespace.

### Non-functional requirements

- **NFR-016-001 (security)**: fail closed everywhere. No caller can learn that a field they may not
  read exists, except through the ADR-0028 structure permission. A `secret`, an unmarked value without
  a reveal, a value of a non-validating payload and a value of a legacy version are never returned.
  Authorization fails closed; an audit failure returns no value.
- **NFR-016-002 (consistency and durability)**: the payload read, the grant check and the audit insert
  share one short transaction (`begin_safely`, ADR-0014); bytes leave only after commit. A grant or
  revocation commits with its audit. Nothing here changes a state transition, the scheduler, the outbox
  or a timer. The classification is read from an immutable document, so a concurrent publication cannot
  change what an open request returns.
- **NFR-016-003 (performance)**: evaluation cost per response is bounded by FR-016-024 and measured on a
  stated environment: p50/p95/p99 for the largest payload and schema, validator cache hit ratio,
  allocations and response size. No latency promise is made before the measurement.
- **NFR-016-004 (compatibility)**: additive IR and OpenAPI; rules version pinning; five-SDK parity;
  old Studio and old clients unaffected; migration is additive and reversible.
- **NFR-016-005 (observability)**: a span per disclosure with closed attributes, a counter by closed
  outcome and level, a duration histogram, a cache counter, audit as in FR-016-012; none carries a value.
- **NFR-016-006 (accessibility)**: the Studio views meet the same accessibility standard as the rest of
  the Studio (keyboard path, labels, level not by colour alone), checked by tools and a keyboard
  walkthrough, not claimed from assistive-technology use.

### Key Entities

- **Classification level**: the four-value ordered type; pure domain.
- **Carrier schema**: a schema in the pinned document where the annotation is read.
- **Effective level map**: the per-location result of the walk; derived per response, never stored.
- **Classification rules version**: a small integer stored with each workflow version.
- **Reveal grant**: `(tenant, namespace, subject)`, who may reveal `pii` in a namespace.
- **Disclosure request and result**: selector, reveal flag, and the discriminated result of
  [contracts/README.md](./contracts/README.md).
- **Audit actions**: `payload.disclosed`, `payload.pii_revealed`, `payload.reveal_denied`,
  `payload.reveal_granted`, `payload.reveal_revoked`.
- **Event view**: the answer of `getInboxEvent` and of the dead-letter reads: unchanged metadata plus
  either the revealed values or their size and structure (FR-016-026, FR-016-027).

## Success Criteria *(mandatory)*

- **SC-016-001**: Canary values of every forbidden class (`secret`; unmarked or undeclared without a
  reveal; values of a non-validating payload; values of a legacy version; keys of withheld nodes) are
  absent from 100% of captured responses, errors, logs, spans, metrics, audit records, streams, CLI
  output and Studio DOM, storage and network traffic in the real PostgreSQL/API/Chromium suites, for
  every built-in role and with a grant. Event data, dead-letter bodies and dead-letter headers are
  forbidden classes for every caller who has not sent `reveal=pii` with the grant.
- **SC-016-002**: The audience matrix (six roles x four levels x reveal requested or not x grant or not)
  produces exactly the specified result in a contract test, one row per cell.
- **SC-016-003**: Every successful reveal has exactly one `payload.pii_revealed` record committed
  before the response; an injected audit failure or crash before commit returns no value.
- **SC-016-004**: Every run of a version published before this feature discloses no value to any role,
  and the document bytes and digest of those versions are unchanged by the migration.
- **SC-016-005**: Each defect class of FR-016-004 is refused with its code and pointer; the accepted
  corpus is stable (the same digest on a second publish); a version's disclosure is unchanged by a
  later publication.
- **SC-016-006**: Cross-tenant and cross-namespace attempts (run, sequence, subject, grant) are opaque
  `404` or `403` as specified, are audited through the restricted path and leave the other namespace's
  rate bucket unchanged.
- **SC-016-007**: The five SDKs build the shared classified workflow to byte-identical canonical
  documents (the shared conformance scenario passes in TypeScript, Go, Python, C# and Java).
- **SC-016-008**: A benchmark report states the environment and, for the maximum payload and schema,
  p50/p95/p99, cache hit ratio, memory and response size, plus every failure; the bounds of FR-016-024
  hold under it.
- **SC-016-009**: Every error and refusal of the feature exposes `code`, `traceId`, `retryable` and a
  remediation without payload, key, schema fragment or secret.
- **SC-016-010**: Every `getInboxEvent` and dead-letter read by a caller without the reveal grant, or
  without `reveal=pii`, returns zero event values, body bytes, header values and keys in the real
  PostgreSQL/API suites; every successful event reveal has exactly one `payload.pii_revealed` record
  committed before the response; a foreign tenant's event or letter is the opaque `404` and a grant of
  another namespace reveals nothing.

## Assumptions and dependencies

- Depends on features 001 (contracts, identity), 002 (journal, `payloads`, run result), 004 (the
  event inbox and connector dead letters that US08 restricts), 007
  (inspection API and Studio), 008 (telemetry), 009 (roles, permissions, audit, quotas), 011
  (publication and diff), 012 (CLI) and 006 (SDK builders), and on ADR-0014, ADR-0023, ADR-0027,
  ADR-0028.
- **Branch dependencies (this branch is cut from `bugfix/owner-decisions`):** the run result of G21
  (`runs.result_digest`, migration `20261110000000`, PR from `bugfix/run-result`) must be merged before
  `source: "result"` (FR-016-013, T016-031 in `tasks.md`); ADR-0031 and the additive-IR ADR-0032
  live on `feature/workflow-primitives` and are cited by name, not linked, until feature 014 merges.
  This feature needs no 014 code: it adds optional keys to existing nodes and leaves ADR-0031 alone.
- The IR's workflow `inputSchema` and `outputSchema` exist but are not checked beyond `type: object`
  today, and an inline run input is not validated against `inputSchema` at start (only a claim-check
  descriptor is, via `RunInputChecker`). Disclosure therefore validates at read time (G273).
- A value stays stored as it is: `secret` classification limits disclosure, it does not move the value
  to the secret store of feature 009 and does not encrypt it at rest (G279). Credentials belong in
  platform secrets, which never enter the journal.
- There is no REST or CLI way to manage memberships in this branch (`provision-tenant` and `seed-dev`
  only, G290); this feature does not add one. Reveal grants are a separate resource.

## Out of scope

- Retention per customer or regulator, deletion of personal data on request, and data residency
  (ADR-0033, point 6; G283, G284).
- Field-by-field classification of events, connector dead letters and webhook or broker deliveries: an
  event's data is `pii` as a whole until an event type schema registry exists (G282, D302). The
  values of events are restricted by this feature (FR-016-026, FR-016-027); their classification is not.
- Disclosure of external (blob) content, raw download or export, and any field-selection or JSON-path
  API.
- Taint tracking: the classification of a value copied by an expression, a child input or a result
  mapping is whatever the receiving carrier says (G276).
- Declared state and Query/Update results (ADR-0031 owns them); a generic state reader (G285).
- A Studio screen for reveal grants, SDK REST-client methods for disclosure, and annotation helpers in
  the SDK builders (G280, G281).
- Changing the `payloadRef` digest in journal events (G275), validating inline input at run start (G273)
  and encrypting payloads at rest (G279).
