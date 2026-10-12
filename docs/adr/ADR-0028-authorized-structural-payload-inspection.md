# ADR-0028: Authorized structural payload inspection

**Status:** Accepted design; implementation and verification pending.

**Traces:** FR-007-005, FR-007-010, FR-008-006, FR-009-004; T007-048; G144. Customer classification and value-disclosure policy remain pending under G42/G39.

## Context

The journal and runtime graph deliberately omit payloads. This protects values but does not let an operator inspect a run's data: G144 remains a real missing feature. Inline inputs and completed outputs already exist as immutable, tenant-scoped `payloads`; a run references its input and a journal event references the exact output it describes. External payloads have the catalog provenance defined in ADR-0023.

There is no approved customer classification that permits revealing values. A JSON Schema property, annotation, example or default is not such an approval. Credential heuristics are also insufficient: a secret can be an ordinary string, number, array member, or an object key.

## Decision

Implement a separate, authorized and audited inspection API and Studio view under policy `structure-only-v1`. It shows the actual persisted payload's structure, types, container counts, total byte size and provenance. It hides **every scalar value and every object key**, including schema-declared names. This lets an operator distinguish an empty result, a scalar instead of an object, an unexpectedly large array, a nested shape, and an unavailable historical result. It is a structural inspection, not a raw-data viewer.

Structure and sizes are still sensitive metadata. They require their own permission and must not be added to the ordinary graph, SSE, journal listing, logs or telemetry attributes. This policy does not claim that structural metadata reveals nothing about the underlying data.

No raw reveal, field allowlist, schema-public annotation, raw download or payload export is added. Customer/regulatory classification, retention and any later permission to disclose values remain explicit pending decisions; they are not prerequisites for this conservative implementation. A later disclosure policy requires a new decision and tests. Existing payload retention is unchanged.

## Public API and selectors

Add `POST /v1/runs/{run_id}/payload-inspections`. The body is one of the following closed shapes, with unknown fields rejected:

```json
{"source":"input"}
```

```json
{"source":"journal","sequence":42}
```

`sequence` is a positive journal sequence. There is no client-supplied tenant, namespace, digest, blob id, JSON path, redaction mode, reason or policy override. POST keeps the selector out of query strings and avoids a cacheable read URL; it performs an audited read, not an execution-state mutation. An idempotency key is not required. Each HTTP retry consumes the ordinary namespace API admission again; audit repeat collapsing is independent.

Resolve the input only through `(authenticated tenant, run) -> input_digest`; resolve a historical payload only through `(authenticated tenant, run, sequence) -> payload_digest`. All joins include tenant identity. A missing or foreign run, or a missing journal sequence, is the same opaque `404`. An existing event with no payload produces the explicit unavailable result below. Never substitute the current step output for an earlier event, nor imply that a failed attempt's output was retained.

The response uses a discriminated shape, with `policy` fixed to `structure-only-v1`:

```typescript
type Selector = { source: "input" } | { source: "journal"; sequence: number };
type Provenance = {
  runId: string;
  runVersion: number; // version observed with the selected stored record
  selection: Selector;
  nodeId?: string;   // only when present on the selected journal event
  attempt?: number;  // only when present on that event; never inferred from today's step
};
type Structure =
  | { type: "object" | "array"; childCount: number;
      children: Structure[]; omittedChildren: number }
  | { type: "string" | "number" | "boolean" | "null"; redacted: true };
type Inspection = Provenance & { policy: "structure-only-v1" } & (
  | { kind: "inline-json"; sizeBytes: number; truncated: boolean; tree: Structure }
  | { kind: "external"; sizeBytes: number; contentKind: "json" | "text" | "binary" | "other";
      reason: "EXTERNAL_CONTENT_NOT_INSPECTED" }
  | { kind: "unavailable"; reason: "NO_RETAINED_PAYLOAD" | "UNSUPPORTED_FORMAT";
      sizeBytes?: number }
);
```

Object children have only ordinal positions; no key, key hash or raw JSON pointer is returned. Arrays retain their positions. The Studio may label object children “Field 1”, “Field 2”, and so on. Scalar nodes expose neither a value nor a prefix, digest, per-scalar length, truth value or numeric magnitude. `null` is a structural type like the other types. Total `sizeBytes` is the original encoded payload size, not the size of the projection. Unknown/invalid formats return a closed reason without parser diagnostics or excerpts.

An external result is recognized only from committed catalog provenance, not merely from JSON containing `$blob`. Run input uses the stored `input_blob_id`, including legitimate rerun/child inheritance; a journal output must match the committed step-output owner for that run/event. Its size comes from the catalog. `contentKind` is a closed category derived from the stored content type; do not echo an arbitrary content-type string. Return neither backend keys nor signed URLs. This operation does not manufacture a worker lease, call the worker's blob-read capability, or fetch arbitrarily large external data. The UI must say that external content itself was not inspected. A forged descriptor without catalog provenance remains ordinary redacted JSON.

Successful results are `200` and carry `Cache-Control: private, no-store`. Errors use the existing typed envelope: `401` unauthenticated, `403` permission denied, opaque `404`, existing request/validation errors for invalid selectors, quota/rate refusal under ADR-0027, and `503` for store/audit unavailability. Do not reflect selector content in error messages.

Reuse the existing error mappings rather than inventing payload-specific top-level codes:
`UNAUTHENTICATED`, `FORBIDDEN`, `NOT_FOUND`, `BAD_REQUEST` for malformed JSON/body limits,
`VALIDATION_FAILED` for a well-formed body outside the selector schema, `QUOTA_EXCEEDED` for
disabled admission, `RATE_LIMITED` with bounded `Retry-After` for temporary depletion, and
`UNAVAILABLE` for a dependency/audit failure. Preserve each existing mapping's retryability.
The three uppercase inspection reasons in the response union are result states, not error codes.

## Permission, admission and auditing

Add `RunPayloadInspect` / `run:payload:inspect` to the permission catalog and authentication-context contract. Require both this permission and `RunRead`. Initially grant inspection to `Admin` and `Operator`; do not grant it implicitly to `Developer`, `Viewer`, `Worker` or `Runner`. No `DefinitionRead` dependency is needed because even schema property names remain hidden.

Authorize and resolve the run before charging its canonical namespace API bucket. Charge once for the public request, not per visited node or internal read. An unknown/foreign run or forbidden role does not debit another namespace. Keep the existing tenant-scoped denial behavior and audit protection against cross-tenant existence oracles.

Add `PayloadInspected` / `payload.inspected` on `ResourceType::Run`. Use the actor's tenant and subject, the run id, trace id and a server-generated bounded reason: `structure-only-v1:input` or `structure-only-v1:journal:<sequence>`. No payload, key, raw error, blob key, digest, schema, user-written reason or secret goes in the audit. Collapse identical actor/run/selector reads in a 300-second window using the existing audit mechanism. A failed audit must not be treated as a successful read.

The store reads the selected immutable payload/provenance and inserts the audit in one short transaction. Return bytes to the projector only after that transaction commits; if the audit or commit fails, return no inspection. A repeat whose audit already exists may return the same permitted projection. The input, event and payload are immutable, so this read does not need a run `FOR UPDATE` lock or changes to execution scheduling, timers or outbox. Resolve the run and its observed version consistently with the selected event (one statement or an equivalent consistent read). A selector race with a newly committed event may return `404` until that event exists, never substitute another result.

Add `PayloadInspectionDenied` / `payload.inspection_denied` for an authenticated permission
refusal, with `ResourceType::Run`, actor-owned tenant, the parsed run id and fixed reason
`permission`. Collapse actor/run/reason repeats for 300 seconds. This denial record is best effort
and cannot authorize an inspection on failure. Cross-tenant denials reuse
`CrossTenantAccessDenied` and remain withheld from tenant-visible audit. Adding denial telemetry
must not turn a missing/foreign distinction into an API or tenant-visible audit oracle. An
unauthenticated request keeps the existing authentication refusal path.

## Resource bounds and implementation boundary

Inline stored payloads already have a 256 KiB database limit. Preserve it and recheck at the adapter boundary. Projection limits are fixed for this policy:

| Bound | Value |
|---|---|
| Request body | 1 KiB |
| Input bytes parsed | 256 KiB |
| Maximum tree depth | 16, with the root at depth zero |
| Maximum projected nodes, including the root | 256 |
| Maximum children of one projected container | 32 |
| Maximum serialized response | 64 KiB |

At a depth/node/child boundary, retain the container's original `childCount`, emit only the bounded children, set `omittedChildren = childCount - children.length`, and set top-level `truncated: true`. Traverse deterministically. Do not construct an unbounded projection and truncate its serialized bytes: it must remain a valid response. Parsing must also have a bounded recursion/allocation strategy and no raw diagnostic output. An unsupported encoding or JSON depth beyond the safe parser capability returns `UNSUPPORTED_FORMAT`. Ordinary deep JSON within the parser's safe bound can still produce a truncated structural tree.

Put selector/provenance/projection semantics and the inspection use case in `crates/application/src/payloads.rs`; keep raw byte carriers' `Debug` redacted. A small projector port lets `crates/infra-ir` parse JSON without importing `serde_json` into the Rust domain or application business model. The PostgreSQL adapter owns the selected-payload-plus-audit transaction in a new `payload_inspection.rs`, using existing payload and journal tables. The HTTP adapter owns the wire union and route; a structural projection is not a new content-storage model.

Emit a span and duration plus `payload_inspections_total` with closed outcomes (`inline_json`, `external`, `unavailable`, `denied`, `error`) and a truncation counter. No tenant/run/node/key/value becomes a metric label. Spans and logs may carry the existing correlation identifiers but never raw payloads, projections or parser errors. The telemetry text scrubber is defense in depth; it is not this redaction policy.

## Studio behavior

Add a lazy “Inspect redacted payload” action to the run input and to journal events that actually retain a payload. A selected step can link to a specific such event, with its sequence and attempt, rather than pretending all attempts have data. Keep the graph and SSE payload-free. Render only the server's projection; the browser never receives raw values or keys to redact itself.

Show the policy notice, source/provenance, byte size, structural tree, omitted-child markers, and real no-payload/unsupported/external/loading/error states. Provide a keyboard-accessible list/tree with the existing design tokens and accessible error handling. Do not show sample data or raw reveal/download/export controls. Do not copy the projection to telemetry, local storage or persistent query caches. Abort pending inspection requests and clear the view when the run, selected event, tenant or session changes; a late response cannot populate a different selection.

## Verification and delivery contract

T007-048 remains pending until all of these have real evidence:

1. Canary values **and keys**, ordinary secrets without credential shapes, numeric/boolean values, nested arrays, Unicode, and schema `public`/`default`/`examples` annotations reveal no values or names in response, DOM, errors, logs or audit. The projection contains actual container/type/count data.
2. Admin/operator success; developer/viewer/worker/runner refusal; missing/foreign run and sequence behavior; no global digest enumeration; cross-tenant requests leave the other namespace's rate bucket unchanged.
3. Audit insert/commit failure returns no projection; repeated reads have bounded audit records; no payload content enters the trail. Concurrent execution does not make one event resolve to another attempt's output.
4. Exact input and historical journal selection after later events/retries, events without retained output, committed external input inheritance/output provenance and forged `$blob` JSON.
5. Deep/wide/maximum-byte/multibyte payloads obey every bound and serialize within 64 KiB; invalid formats fail closed without excerpts. No external backend GET is made for a metadata inspection.
6. REST responses validate against OpenAPI; permission catalog and generated Studio types agree. UI tests cover loading, denial, unavailable/external data, truncation and stale-response cancellation. A real browser journey verifies the useful structural view and keyboard/accessibility behavior with canaries absent.

The implementer owns the new application/projector/PostgreSQL/HTTP modules and focused tests. Coordinate shared `identity.rs`, `audit.rs`, module registrations, router/state wiring, OpenAPI and Studio files with their owners before edits. Update `data-model.md`, `research.md` G144, design's unavailable-element table, traceability and quickstart with the delivered behavior and actual test results in the implementation delivery. This design-only change does not claim those surfaces are implemented.

## Alternatives and consequences

Continuing to omit all payload inspection does not satisfy the operational use case. Returning raw JSON then hiding it in the browser exposes it in the network and client state. Heuristic redaction misses unrecognizable secrets; schema annotations cannot authorize customer-data disclosure. Uniform server-side value/key removal is deliberately less descriptive than named-field inspection, but supplies a useful bounded view without making that missing policy decision.

The endpoint and permission are additive. Existing readers keep their old responses and rights. External content remains opaque; no retention rule changes and no historical payload is deleted. The structural inspection can be completed and verified independently of later value-disclosure policy, which must stay explicitly pending.
