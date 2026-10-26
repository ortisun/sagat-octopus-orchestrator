# Research: per-field payload classification

**Date**: 2026-10-16. The direction is decided by ADR-0033 (owner decision of 2026-10-16). This file
records what the repository was found to contain, the design decisions D261 onward that fill the points
ADR-0033 leaves to the specification, the gaps G272 onward, and the owner's answers of 2026-10-17 to the
four product questions P1 to P4 (D275, D302 to D306, D265). Nothing here is implemented. No new ADR is
proposed (see D290).

**Numbering.** Feature 014 took D291 to D301 on `feature/workflow-primitives`, so the decisions of the
owner's answers of 2026-10-17 are D302 to D306 (they were first written as D291 to D295). The decisions of
this feature are D261 to D290 and D302 to D306; the gaps are G272 to G293. Next free: D307 and G294.

## Baseline verified in this repository

| Evidence | Consequence for 016 |
|---|---|
| `specs/001-platform-foundation/contracts/workflow.schema.json`: root `inputSchema` and `outputSchema` are `{"type": "object"}`; node schemas (`activity`, `script`, `waitEvent`, `child`, ...) have **no** `outputSchema`; `base` has `additionalProperties: true` | The only schemas that exist for values are the two workflow ones. Step outputs have no carrier, so node-level `outputSchema` is new IR (D262). Unknown node keys are tolerated and ignored by an older engine. |
| `crates/infra-ir/src/{schema.rs,convert.rs,lib.rs}`: the codec validates the document against the embedded schema, converts to the domain, compiles expressions, canonicalises and hashes; `canonical_json` keeps the whole document | An annotation survives into the stored `document` and its digest without domain changes; the codec is where publication checks go. |
| `crates/infra-schema/src/lib.rs`: `compile` (no network, no file, linear-time `pattern`), `validate` returns `"{path} breaks {rule}"` only, `Schemas: RunInputChecker` | The schema machinery and its non-echoing discipline exist; the walker belongs here, and `iter_errors` text (which repeats instance values) must not be used on instance data. |
| `crates/application/src/runs.rs`, `blobs.rs`: `RunInputChecker` runs only for a claim-check descriptor; an inline run input is not validated against `inputSchema` at start | Inputs are not guaranteed to match their schema; the validity gate must run at disclosure (D269, G273). |
| `crates/application/src/payloads.rs`, `crates/infra-postgres/src/payload_inspection.rs`, `crates/api-http/src/routes/payload_inspection.rs`, `crates/infra-ir/src/payload_projection.rs` | The model for the new operation: closed selector, `prepare`/`inspect` split, one transaction for read plus audit, bytes only after commit, external payloads by catalog provenance only, bounded projection. |
| `crates/infra-postgres/src/execution.rs` (event insert): `payload_digest` is set only for `StepCompleted`, `WaitSatisfied`, `ReconciliationConcluded` | These are the journal events a `journal` selector can disclose; step *inputs* are not retained as payloads. |
| `crates/domain/src/identity.rs`: `Permission::ALL` has 36 entries and **is** the Admin role; `Role` has six variants; `RunPayloadInspect` is held by Operator and Admin; `Principal` has one role; `memberships` has one row per `(tenant, subject)` | Adding a permission to `ALL` would silently grant it to Admin (D276). A pii grant cannot be a second role. |
| `crates/application/src/api_admission.rs`: `ApiOperation::PayloadInspect => [RunRead, RunPayloadInspect]` | The admission class for the new operation (`PayloadDisclose => [RunRead]`). |
| `crates/domain/src/audit.rs`: `PayloadInspected`/`PayloadInspectionDenied`, `collapses_repeats`, `repeat_window` 300 s | The audit pattern to reuse; pii reveals deliberately do not collapse (D277). |
| `docs/runbooks/control-plane.md` section 8 and `apps/control-plane`: memberships are created by `provision-tenant` and `seed-dev` only; there is no membership REST or CLI | A per-membership flag needs a new management surface exactly as a grant list does (P1). |
| `openapi.yaml` `getInboxEvent` returns `data`; `listConnectorDeadLetters` returns `body` and `headers`; both need only `event:read` (viewer holds it); `dto_events.rs` `EventBody`, `DeadLetterBody` | **Existing value disclosure** that contradicts the premise of ADR-0033 (G272); restricted by D302. |
| `openapi.yaml` `JournalEvent.payloadRef`: "SHA-256 of the output payload" visible to `run:read` | A plain digest of a guessable payload confirms a guess, whatever its level (G275). |
| `openapi.yaml` `Error.code` is a closed set of ten; refinements are `details.reason`; `DefinitionProblem.code` is the refinement for `validate`; `crates/api-http/src/error.rs` `definition_details` | The reasons and codes of D272/D273 follow these mechanisms. |
| `sdks/*`: all five builders take `inputSchema` as a free-form map; Go and Python have `outputSchema`, TypeScript, Java and C# do not; none has a node-level `outputSchema`; no SDK client method for `payload-inspections` | SDK work is the missing setters plus one shared golden; helpers are unnecessary (D284). |
| `apps/cli/src/workflow.rs` `inspect_run`: prints status, journal and attempts, "No payload is shown" | The CLI extension point (D286). |
| `frontend/apps/studio/src/features/runs/{PayloadInspection.tsx,NodePanel.tsx,RunDetailPage.tsx}`; `frontend/e2e/tests/payload-inspection.spec.ts`; `tests/e2e/workflow-studio/payload_inspection.rs` | The Studio extension point and the canary test models. |
| `crates/telemetry/src/redact.rs` | A scrubber exists; it is defense in depth, not the policy (ADR-0028). |
| Branch state: this worktree is cut from `bugfix/owner-decisions`. `runs.result_digest` (migration `20261110000000`) is on `bugfix/run-result`; ADR-0030/0031/0032 and feature 014 are on `feature/workflow-primitives` | Dependencies are recorded in the spec; links to files that are not on this branch are avoided. |

## Decisions

### D261 — The annotation is the JSON Schema keyword `x-sagat-classification`

**Decision**: one string-valued keyword, `x-sagat-classification`, with the four levels. **Rationale**:
JSON Schema 2020-12 treats unknown keywords as annotations; an `x-` prefix cannot collide with a
future standard keyword; `sagat` avoids collisions with other tools that read `x-classification`.
**Alternatives**: a bare `classification` (could collide with a future vocabulary); a custom
`$vocabulary` meta-schema (heavy, and the `jsonschema` crate would need the vocabulary to be
declared); a side map `{pointer: level}` next to the schema (drifts from the schema it describes,
needs pointer maintenance on edit, and is invisible to a schema author's tools). **Risk**: that
`jsonschema` 0.58 rejects or reacts to an unknown keyword; T016-013 proves it does not, and the
fallback is to compile a copy with the annotation removed.

### D262 — Carriers: the two workflow schemas plus an optional node `outputSchema`

**Decision**: read the annotation in `inputSchema`, `outputSchema`, and a new optional `outputSchema`
on `activity`, `script`, `waitEvent` and `child` nodes. **Rationale**: ADR-0033 says "the JSON Schema of
a value in the workflow IR", but the IR has schemas only for the run input and the (now stored) run
result. Without a node schema the only step output a reader could disclose would be none, so the
step-output story (US05) needs the new key. It is additive and ignored by an older engine.
**Alternatives**: workflow-level carriers only (smaller; US05 would be cut and operators could still
read input and result; this is the cheapest cut if the owner wants one); a separate `classification`
document per workflow (a second pinned artifact, more machinery); infer a schema from the value (the
`infer` function in `infra-schema` exists for scripts; inference cannot decide a level). Node `inputSchema`
is not added: a step input is not retained as a payload, so there is nothing to disclose.

### D263 — Annotations outside the carriers are rejected

**Decision**: an annotation in declared state, a Query or Update schema, `humanApproval.formSchema` or
anywhere else is a publication error (`CLASSIFICATION_NOT_APPLICABLE`). **Rationale**: ADR-0031 discloses
a declared operation result by its own explicit policy and this feature leaves it unchanged; silently
accepting an annotation there would let an author believe a `pii` or `secret` mark protects a result that
is disclosed anyway. **Alternatives**: accept and ignore (false assurance); interpret it for operation
results (changes ADR-0031, which this feature must not do).

### D264 — Levels, order and inheritance exactly as ADR-0033, with the root default made explicit

**Decision**: order `public < internal < pii < secret`; an annotated node sets the level of what it
governs; an unannotated declared node inherits its parent; the root of a carrier with no annotation is
`pii`. **Consequence made visible**: because an unannotated child inherits, putting `public` on a root
makes every unannotated declared child `public`. That is what ADR-0033 point 1 says; the mitigation is
the `validate` report and the `CLASSIFICATION_INHERITED_OPEN` warning (US07), not a different rule.
**Alternative**: annotation applies to the node only and every unannotated node is `pii` (safer by
default, but contradicts point 1 and forces an annotation on every leaf).

### D265 — A value no schema declares is at least `pii` (decided by the owner, 2026-10-17, P3)

**Decision**: an undeclared property, an item no `prefixItems`/`items` governs, and anything under an
unsupported keyword has effective level `max(inherited, pii)`. **Rationale**: "nothing is disclosed by
omission" (ADR-0033 point 2). A producer that adds `ssn` to a `public` object whose schema allows
additional properties must not publish it. **Deviation**: a literal reading of point 1 ("applies to
everything under it") would make the extra property `public`. The stricter rule changes no declared
case and is invisible to a closed schema (`additionalProperties: false`), which the examples use. An
author who wants an open map public annotates `additionalProperties` with `public`. **Alternative**:
literal inheritance (simpler to explain, leaks undeclared data). The owner confirmed the restrictive
reading on 2026-10-17, together with its companion: a carrier with no annotation is `pii` throughout
(a reveal holder sees it) and a payload with no carrier at all is structure only for everyone (D269).

### D266 — The strictest annotation among all governing subschemas wins; no branch is chosen by the data

**Decision**: for `allOf`, `anyOf`, `oneOf` and `$ref`, the effective level at a location is the
maximum over every subschema that could govern it. **Rationale**: it is deterministic, it needs no
evaluation of which branch matched (which would make the level depend on the value), and it can only
err toward hiding. **Alternative**: evaluate the matching branch (value-dependent levels; cost; a
mismatch between validator and walker becomes a leak).

### D267 — A closed set of supported keywords; an annotation under any other applicator is refused

**Decision**: the walker interprets `type`, `properties`, `additionalProperties`, `items`,
`prefixItems`, `$ref` (inside the carrier), `$defs`, `allOf`, `anyOf`, `oneOf`. An annotation under
`patternProperties`, `if`/`then`/`else`, `dependentSchemas`, `contains`, `propertyNames`, `not`,
`unevaluated*` or `$dynamicRef` is `CLASSIFICATION_UNSUPPORTED_KEYWORD`. **Rationale**: a keyword the
walker does not understand cannot be trusted to place the annotation on the right instance location;
refusing is the only way not to guess. Values that only such a keyword governs are undeclared (D265),
so they are `pii`. **Alternative**: support `patternProperties` now (needs regex evaluation per key;
the linear-time engine makes it possible; deferred, not forbidden, and recorded as a candidate for
classification rules version 2).

### D268 — A declared descendant that is looser than its parent is a publication error, inline or through `$ref`

**Decision**: `CLASSIFICATION_LOOSENED` is an error everywhere, including for a `$defs` entry annotated
`public` that is used under a `pii` parent; the evaluator also clamps with `max`. **Rationale**: one
simple rule an author can predict; a shared definition annotated at its definition cannot be used in a
stricter context, so authors annotate at the use site and leave shared definitions unmarked (they then
inherit). **Alternative**: error for inline only and clamp silently through `$ref` (hides a surprising
result; rejected). An unmarked root counts as `pii` for this rule, so a `public` child under an
unmarked root is refused with a message that names the root.

### D269 — Disclosure is gated by the validity of the whole payload against its pinned carrier

**Decision**: compile the carrier, call `is_valid` on the parsed instance; only then walk and project.
Otherwise `structure-only` for a caller with `run:payload:inspect`, `unavailable/NOT_DISCLOSABLE` for
the rest, with the reasons of the contract. **Rationale**: ADR-0033 point 5; a payload that does not
match its schema cannot be classified field by field, and an inline run input is not validated at start
(G273), so mismatches will happen. `is_valid` produces no message, so nothing about the instance is
echoed. The `NOT_DISCLOSABLE` answer is the same for invalid, schema-less and legacy, so it is not an
oracle for a caller who may not see structure. **Alternative**: disclose the valid parts (the walk cannot
trust positions in an invalid value); fail with an error (an error body is a channel).

### D270 — The interpretation is pinned with the version: `workflow_versions.classification_rules`

**Decision**: store a small integer with each published version; `0` for every earlier version
(annotations are not read), `1` for the rules of this document. **Rationale**: ADR-0033 point 4 says
the classification is immutable with the version. The *document* is immutable, but the *code that
interprets it* is not; a later release that supports `patternProperties` would change what version 1's
annotations mean. Pinning the rules version makes "immutable" true of the meaning. It also makes the
legacy guarantee exact: version 0 reads nothing, even for a document that happens to contain the key.
**Alternatives**: nothing (silent reinterpretation); store a computed effective-level index per version
at publish (fast, but a second artifact to keep consistent with the document, and the runtime still needs
an interpreter for the instance walk); put the rules in `apiVersion` (ADR-0032 forbids a new public
revision for additive changes).

### D271 — A new operation, `payload-disclosures`, not a change to `payload-inspections`

**Decision**: add `POST /runs/{runId}/payload-disclosures` with policy `classified-v1`; leave
`payload-inspections` and its `structure-only-v1` response as they are. **Rationale**: ADR-0028 says a
later disclosure policy requires a new decision (ADR-0033) and tests, and the existing endpoint's
audience is Admin and Operator by `run:payload:inspect`; the new one has a different audience (viewer for
`public`) and a different response. Changing the old response would break the Studio and any client.
**Alternative**: a `policy` field on the old endpoint (two shapes behind one operation id; a client
that does not send it must keep today's answer, so the new behaviour is opt-in either way).

### D272 — Result states are not errors; there are five error reasons

**Decision**: `kind` and `reason` values in a `200` answer describe the payload; errors are
`PII_REVEAL_NOT_GRANTED` (403), `CLASSIFICATION` (422 on publish) and, for the grants,
`UNKNOWN_SUBJECT`, `ROLE_CANNOT_REVEAL` (D274) and `GRANT_LIMIT`. All in `details.reason` over the
closed `Error.code`.
**Rationale**: the `Error.code` set is closed; ADR-0028 draws the same line ("result states, not error
codes").

### D273 — A reveal is a deliberate act: it needs `reveal: ["pii"]`, not only a grant

**Decision**: a holder of the grant who does not send `reveal` gets no `pii` value (an inspector sees
the node withheld with `REVEAL_NOT_REQUESTED`) and no reveal is audited. **Rationale**: the audit then records an intentional
act, not every view of a run by someone who happens to hold the right; the Studio asks for confirmation
once per view. **Alternative**: reveal whenever permitted (simpler call, audit noise and accidental
exposure, and `pii` values in every view of the holder).

### D274 — Permissions: `payload:read-internal` for three roles, `payload:reveal-pii` for none; a reveal also needs read-internal

**Decision**: `payload:read-internal` goes to `developer`, `operator`, `admin` (the audience ADR-0033
names, expressed in the existing role-to-permission table, not by testing role names). `pii` needs a
reveal grant **and** `payload:read-internal`, so that a principal never sees a stricter level than a
looser one; granting to a subject whose role lacks it is refused (`ROLE_CANNOT_REVEAL`) and a later
demotion makes the grant inert. **Alternative**: let a grant alone suffice (a `viewer` could then see
`pii` and not `internal`, which breaks the nesting argument of FR-016-009).

### D275 — The grant is a per-namespace list (decided by the owner, 2026-10-17, P1)

**Decision**: `payload_reveal_grants(tenant, namespace, subject)`, managed through
`membership:manage`, REST and CLI. **Rationale**: it follows G39's own answer to "no custom roles"
(resource-level control where the resource is: worker bindings, a secret's access list, the pools of
owner decision 9), limits the reach to a namespace (and so to an environment: dev, staging, prod),
leaves `Principal` and the six roles untouched, is revoked by deleting the membership
(cascade), and each change is audited. The cost is one table, three endpoints and a CLI command.
**Alternatives** rejected by the owner: a tenant-wide flag on the membership, adding the permission to
`admin`, a seventh role (see P1 below).

### D276 — Admin does not hold `payload:reveal-pii`; `Permission::ALL` stops defining Admin

**Decision**: `Role::Admin` becomes `ALL` minus `PayloadRevealPii`, through an explicit constant with a
test that compares it to `ALL`. **Rationale**: ADR-0033 says "granted explicitly"; the current
`Admin => &Permission::ALL` would grant it by default. G49 says an administrator is trusted with secrets,
but trust in an administrator is not the owner's wording for personal data.

### D277 — Audit model

**Decision**: every `pii` reveal is audited, never collapsed, in the read's transaction, before bytes
return; a disclosure that returns an `internal` value is audited with a 300 s collapse (as ADR-0028);
a `public`-only disclosure is counted by metric, not audited per request; a refused reveal is a
best-effort denial record; grants and revocations are audited with their change. **Rationale**: ADR-0033
point 3 requires the `pii` audit; `internal` follows ADR-0028/0031 practice for sensitive reads; `public`
reads by every viewer would dominate the trail. The unbounded growth of reveal records is bounded by the
namespace API bucket of ADR-0027 and by the number of holders. **Alternative**: audit every disclosure
(trail growth); audit only `pii` (no trace of who read internal values).

### D278 — What a caller learns about what they may not see

**Decision**: callers without `run:payload:inspect` get nodes omitted and a `childCount` of what they
received; holders get typed `withheld` nodes (type and level, no value) and the true count. A `secret` is
omitted for a caller without that permission, withheld (reason `SECRET`) for one who holds it, and its
value is shown to nobody (D662). **Rationale**: ADR-0028 treats structure as sensitive metadata
behind its own permission; the new operation must not hand it to a viewer by another route.

### D279 — Names: a declared property name is schema text; an undeclared key is data

**Decision**: a child declared in `properties` carries its name whenever the child is returned (a value,
a container, or a withheld node shown to an inspector); an undeclared key carries its name only when its
value is disclosed. **Rationale**: declared names are in the published document that `definition:read`
holders (viewers included) already see; undeclared keys are data (an email address as a map key) and
follow their value's level.

### D280 — Bounds of a disclosure

**Decision**: payload 256 KiB, schema 64 KiB, schema nesting 32, `$ref` expansions 1,000, response depth
16, 1,024 nodes, 128 children, strings cut at 4 KiB, 128 KiB of disclosed string bytes, response 256 KiB,
request 1 KiB. **Rationale**: the 256 KiB and 1 KiB limits are ADR-0028's; the other values are design
choices larger than the structural projection's (256 nodes, 32 children) because values are what the
operator came for, still bounded so that the response is a fixed worst case. They are measured by
T016-053 and may be lowered; they are not promises.

### D281 — Compiled carriers are cached per process, keyed by the immutable version

**Decision**: LRU of 256 entries or 16 MiB of schema bytes, key `(workflow_version_id, carrier)`.
**Rationale**: compiling a 64 KiB schema per request is the dominant cost; the version is immutable and
content-addressed (`document` digest), so a cached validator can never be stale and a restart only costs
a recompile. **Alternative**: compile every request (simple, predictable, expensive under a loop of
requests); a shared cache across processes (not needed, and a cross-process invalidation problem that the
immutability removes).

### D282 — Telemetry shape

**Decision**: the metrics, span and logs of `contracts/README.md` section 4; labels from closed sets
(`outcome`, `level`, `op`); tenant, run, node, subject, key and value never in a label, a log field or an
audit record. **Rationale**: constitution VI and ADR-0028.

### D283 — No taint tracking

**Decision**: the classification of a value copied by an expression, into a child input or into a result
is whatever the receiving carrier says. **Rationale**: tracking flow through the bounded expression
evaluator, child inputs and results is a research problem; ADR-0033 puts the decision with the author.
**Mitigation**: the `validate` report, the diff attention findings and review (G276).

G21 integration clarification: a parent receives a child's retained result in `child.finished`.
That journal selector uses the exact parent `child` node's output schema and persisted scope;
the child's own input/result selectors use its own pinned workflow version. Event names do not
broaden the carrier set: `child.finished` on an activity, wait or database node is ineligible.

### D284 — SDK scope: setters and one golden, no helper

**Decision**: add `outputSchema` to the TypeScript, Java and C# builders and a node-level
`outputSchema` option to all five; add one shared conformance scenario that builds a classified workflow
in each language and compares canonical bytes; add no `classify()` helper or level constant.
**Rationale**: the annotation is a key in a free-form map every builder already accepts; a helper would
be five implementations and five test sets (AGENTS.md rule 8) for a string constant. **Alternative**:
typed constants per language (sugar; can follow in a minor release without a contract change).

### D285 — Studio scope

**Decision**: values in the run view, an explicit reveal action, the classification report on the
workflow settings panel; no grants screen. **Rationale**: the grants resource is an administrator task
that REST and the CLI cover; the Studio screen can follow as a separate delivery (G280).

### D286 — CLI scope

**Decision**: `sagat workflow inspect <RUN> --values [--reveal-pii] [--sequence N]`,
`sagat workflow validate` prints the report, `sagat namespace reveal-grants list|grant|revoke`.
Control and bidirectional characters in values are neutralised (as `one_line` does for existing output).

### D287 — `secret` limits disclosure; it is not the secret store

**Decision**: a `secret`-classified field is stored like any payload value and never disclosed.
**Rationale**: the workflow needs the value to run. Credentials must still use platform secrets (feature
009), which never enter the journal; the `secret_in_output` check that fails a step whose output repeats
a secret it was given is unchanged. Encryption of payload content at rest is out of scope (G279).

### D288 — The run result selector waits for G21

**Decision**: `source: "result"` is implemented after the `bugfix/run-result` change merges; until
then the other selectors ship. **Rationale**: `runs.result_digest` does not exist on this branch.

### D289 — Compatibility by construction

**Decision**: rules version `0` for every existing version; additive IR and OpenAPI; the old endpoint,
Studio and clients unchanged; the migration is additive and reversible (`down` drops the column and the
table). A document published with annotations to an engine that lacks this feature discloses nothing
(safe direction).

### D290 — No ADR-0034

**Decision**: ADR-0033 is the decision and this set fills the points it assigns to the specification
(annotation name, grant mechanism, schema carriers). The owner's answers of 2026-10-17 are recorded as an
addendum to ADR-0033's Consequences (the grant mechanism and the interim restriction of event values).
A new ADR would be needed only if the owner chose to change the audience table of ADR-0033 or to bring
events under field classification by a mechanism other than a schema carrier.

### D302 — Event values are restricted now, before event schemas exist (owner decision P2, 2026-10-17)

**Decision**: in this increment `getInboxEvent` `data` and connector dead-letter `body` and `headers` stop
being returned to plain `event:read` holders. They require the reveal grant (`payload:reveal-pii` in the
namespace, with `payload:read-internal` and `event:read`); everyone else receives size and structure.
Field-by-field classification of events stays for when event type schemas exist (G282); until then the
data of an event is `pii` as a whole. **Rationale**: the inbox was the one surface that returned values to
every viewer, which is the situation ADR-0033 set out to end; waiting for a schema registry would leave
that open for as long as the registry takes. **Alternatives** rejected by the owner: leave it and record
the exception; restrict it to a plain permission with no classification; classify through a consuming
`waitEvent` or trigger. **Consequence**: a breaking change for every reader of those fields (FR-016-028,
G292).

### D303 — Shape of the event answer: conditional `data`, structure for everyone else, explicit `reveal=pii`

**Decision**: `getInboxEvent` keeps its metadata and gains `dataDisclosure` (`revealed` or
`structure-only`), `dataSizeBytes` and, when structure-only, `dataStructure` (the ADR-0028
`PayloadStructure`, same bounds, no key and no scalar). `data` is present only when revealed, so it leaves
`required`. The reveal is a deliberate act, as for runs (D273): the query flag `reveal=pii` on the GET,
audited, `Cache-Control: private, no-store`. The structure goes to every `event:read` holder, because the
owner asked that everyone else gets structure only and because viewers saw the whole value before. This
differs from run payloads, where structure needs `run:payload:inspect` (D278); the difference is recorded
as G293. **Alternatives**: `data: null` with a flag (keeps the field in `required` but a client that
reads `data.amount` fails either way); a new sub-resource for the data (leaves the old GET with a smaller
shape and an extra call; kept as a later option); structure behind a new permission (a second break for
no gain).

### D304 — Dead letters: the single resource reveals, the list never does (decided 2026-10-17)

**Decision**: `getConnectorDeadLetter` with `reveal=pii` (and the grant) returns `body` and `headers`; the
single resource without it, and every item of `listConnectorDeadLetters`, return `bodyDisclosure:
"structure-only"`, `bodySizeBytes`, `bodyStructure` (the structure of a bounded JSON body, otherwise
`{"kind": "opaque"}`) and `headerCount` (no header name, no value). The list ignores `reveal`.
**Rationale**: a page of 200 letters of up to 64 KiB would be a 12.8 MiB audited response, and one audit
record per page would not say which letters were read; the single resource gives one record per letter
(`dead-letter:<id>`). A dead letter exists because the message was not an event, so its body is often not
JSON, which is why `opaque` exists. **Alternative**: reveal on the list with a smaller page (more code,
less precise audit. The coordinator accepted this on 2026-10-17 as a technical security choice consistent
with the owner's intent (each reveal has its own audit record), so it is decided.

### D305 — The event restriction reuses the grant, the permissions, the reason and the audit action

**Decision**: no new table, permission, `Error.code` or `details.reason`: the check is `event:read`
plus `payload:read-internal` plus the grant of the event's own namespace (the connector's for a dead
letter), a refusal is `403 PII_REVEAL_NOT_GRANTED`, the audit action is `payload.pii_revealed` with
`ResourceType::Event` (id `source/event id`) or `ResourceType::Connector`, never collapsed, written in the
transaction that reads the payload. The structure uses the existing `PayloadProjector` port and
`JsonPayloadProjector`. A `viewer` therefore cannot reveal event data even with a grant (the grant is
refused for that role, D274). **Rationale**: one reveal concept across runs and events; the grants, the
CLI and the audit trail do not learn a second one.

### D306 — An administrator may grant the reveal permission to themselves, audited (owner decision P4, 2026-10-17)

**Decision**: the grant operation does not exclude the caller as the subject; the record is the same
`payload.reveal_granted` with the actor and the subject visible. **Rationale**: a tenant with one
administrator would otherwise be unable to reveal anything (members other than the bootstrap
administrator cannot even be created, G290), and G49 already treats a tenant administrator as trusted.
**Alternative** rejected: four-eyes (a second person must grant), with a lockout risk for small tenants.

## Gaps

| # | Gap | Treatment | Status |
|---|---|---|---|
| G272 | **The event inbox and dead letters disclosed values before this feature.** `getInboxEvent` returned `data` and `listConnectorDeadLetters` returned `body` and `headers` to any `event:read` holder, viewers included, against ADR-0033's premise. | Restricted in this increment by D302 to D305 (US08): values need the reveal grant, everyone else gets structure. Field classification of events remains open (G282). | Resolved by restriction (owner decision P2, 2026-10-17); classification open
| G273 | An inline run input is not validated against `inputSchema` at start (only a claim-check descriptor is). | Disclosure validates at read (D269); non-conforming inputs show as structure only. Start-time validation is a separate behaviour change. | Open |
| G274 | Until an author adds an `outputSchema` to a node, its output cannot be disclosed. | Adoption: `validate` reports activities without one as informational in the Studio panel; no default schema. | Accepted |
| G275 | `JournalEvent.payloadRef` is a plain SHA-256 of the payload, readable by any `run:read` holder; a guessable payload (even a `secret` one) can be confirmed by hashing a guess. | Not changed: the digest is the payload's content identity (ADR-0002, ADR-0023). Options if the owner wants it closed: expose an HMAC under a per-tenant key (breaks equality across tenants, which is fine, and any client comparing digests), or hide the field for viewers. | Open |
| G276 | No taint tracking across expressions, child inputs and results (D283). | Documented; mitigated by report and review. | Accepted |
| G277 | External (blob) payload values are not disclosed. | `external/EXTERNAL_CONTENT_NOT_INSPECTED`, as ADR-0028. Reading a blob for disclosure needs a size policy and a streaming validator. | Open |
| G278 | Script execution logs (`ExecutionBody.log`, readable with `script:read`) and script test-run input/result are author-controlled text and test data, outside classification. | Documented boundary; log quotas are ADR-0027's. | Accepted |
| G279 | A `secret` field is stored in `payloads` like any value; no encryption at rest of payload content. | Documented (D287); disk and backup encryption are deployment concerns of feature 010. | Accepted |
| G280 | No Studio screen for reveal grants. | REST and CLI only (D285). | Open |
| G281 | No SDK REST-client method for the disclosure or the grants. | Out of scope; the five builders get the schema setters only (D284). | Open |
| G282 | No event type schema registry, so no carrier for inbox data (the next step after G272, which the restriction of D302 stands in for). | Candidate design: a namespace-level `event-types` resource with a schema, pinned by digest on each event. Not designed here. | Open |
| G283 | Retention and deletion per customer or regulator. | Out of scope (ADR-0033 point 6); the default tier's retention applies. | Out of scope |
| G284 | Erasure of personal data on request (a deletion path that keeps the journal's integrity). | Not designed; the append-only journal and content addressing make it a feature of its own. | Open |
| G285 | A generic reader of declared state (feature 014) would need its carrier classified; the 014 state schema rejects annotations now (D263). | Revisit if 014 adds a generic reader. | Open |
| G286 | An unannotated declared child under an annotated `public` root is `public` (D264). | `CLASSIFICATION_INHERITED_OPEN` warning and the report. | Accepted, mitigated |
| G287 | The bounds of D280 and the cache sizes are design values with no measurement. | T016-053 measures and may lower them. | Open |
| G288 | `AuthContext.role` in OpenAPI lists four roles while the code has six (`worker`, `runner`); feature 009 research G39 says "five". Pre-existing drift. | Not corrected here beyond the permissions text; noted so a later documentation pass fixes it. | Open |
| G289 | `Run.correlationId`, `startedBy` and `Cause.code` are metadata fields a caller or worker can fill; they are not payload values and are not classified. | Unchanged; callers are told not to put personal data there (runbook). | Accepted |
| G290 | There is no supported way to add a member other than the bootstrap administrator: `provision-tenant` and `seed-dev` create one, the REST API has no membership resource and the control-plane CLI has no add-member command (feature 013 says "an administrator adds members" without a mechanism). | Not built here. Grants (P1 option A) and the role matrix presuppose members; tests insert them through `tests/support`. A membership management surface is a separate decision (013 or a 009 follow-up). | Open |
| G291 | Event data has no classification carrier, so a reveal of an event returns all of it, including content that is `secret` in the sender's eyes; a dead-letter body is revealed as text. | Accepted until event type schemas exist (G282); the runbook says so. | Accepted |
| G292 | The restriction of event values breaks every client that read `data`, `body` or `headers` as a plain `event:read` holder (Studio does not: no screen reads them; SDKs have no event-read method; the e2e and contract suites that read them are updated in T016-064). There is no transitional switch. | Release notes, API reference, runbook and OpenAPI descriptions (FR-016-028); clients take the grant and send `reveal=pii`. | Accepted by the owner (P2) |
| G293 | The structure of event data goes to every `event:read` holder, unlike run payloads, where structure needs `run:payload:inspect` (D278). | Owner wording ("everyone else gets structure only"); a viewer saw the full value before, so the structure is a reduction. Revisit if events get a structure permission. | Accepted |

## Product decisions answered 2026-10-17

The owner answered the four questions on 2026-10-17 (recorded in `docs/owner-decisions.md`, section
2026-10-17). The options that were weighed are kept so that the reasons stay readable.

| # | Question | Answer | D-id |
|---|---|---|---|
| P1 | How is `payload:reveal-pii` granted while the built-in roles stay six? | A **per-namespace grant list** managed by administrators (worker-binding and secret-access-list precedent); grants and revocations are audited. Rejected: a tenant-wide flag on the membership (reach is the whole tenant, and it needs a membership management surface that does not exist), adding it to `admin` (not explicit), a seventh role (breaks one role per membership). | D275 |
| P2 | What happens to the event inbox `data` and the connector dead-letter `body` that disclosed values to every `event:read` holder? | **Restrict them now**: they need the same reveal grant; everyone else gets structure only; field classification of events waits for event type schemas. Rejected: leave them and record the exception; restrict by a plain permission with no classification; classify through a consuming `waitEvent` or trigger. This changed the scope (US08, FR-016-026 to FR-016-029, SC-016-010, T016-057 to T016-065). | D302 to D305 |
| P3 | Two readings of ADR-0033: (a) an undeclared field under a `public` parent; (b) a schema with no annotation, and a payload with no schema. | The **restrictive** reading: (a) it is at least `pii`; (b) a schema with no annotation is `pii` throughout (a reveal holder sees it), a payload with no schema at all is structure only for everyone. | D265, D269 |
| P4 | May an administrator grant `payload:reveal-pii` to themselves? | **Yes**, audited. Rejected: four-eyes. | D306 |

One detail was added by the specification and accepted by the coordinator on 2026-10-17 as a technical
security choice consistent with the owner's intent: a dead letter is revealed one at a time on the single
resource, never on the list, so that each reveal has its own audit record (D304, decided).

## What is not decided

Not decided and not designed: field classification of events and the event type schema registry that
would carry it (G282), erasure and retention per regulator (G283, G284), taint tracking (G276), a Studio grants
screen (G280), SDK client methods (G281), whether `payloadRef` should stop being a plain digest (G275), a
membership management surface (G290) and the production values of the bounds (G287). The design values of
D280 are not capacity claims.

## Review fixes: engine and disclosure (2026-10-25)

Two independent reviews of the integrated feature (security and correctness, 2026-10-24) found defects in
the walker, the validator cache and the disclosure answer. This section records the decisions, gaps and
owner questions of the engine and disclosure half (D640 to D645, G640 to G647, O640 and O641). Another
branch fixes the authoring and surface half and records its own numbers from D660. Findings are cited by
the reviews' own numbering (S for the security review, C for the correctness review); each decision names
the test that proves it.

### D640 — A number is validated on the spelling that was sent, and "unsafe" is a precise rule (C1)

**Decision**: a payload number is unsafe to validate only when the validator would compare a different
value than the author sent. An integer spelling fits `i64`/`u64` (compared exactly) and, when the carrier
has a decimal bound, constant or divisor, stays within 2^53. Any other spelling is the shortest decimal
spelling of the `f64` it parses to (`9.99`, `12.5`, `1.5e2`; not a 30-digit fraction, not `1e400`) and
is below 2^53. A carrier with an integer type refuses only a fraction that rounds to a whole number.
Decimal schema bounds, constants and divisors are accepted below 2^53. A payload that fails the rule is
`UnsupportedPayload`: callers without `run:payload:inspect` get `NOT_DISCLOSABLE`; inspectors get
`UNSUPPORTED_FORMAT`. **Rationale**: the previous rule refused every fractional number as soon as the
carrier had any numeric keyword or any integer type, so `{"price":9.99}` under `minimum: 0` and
`{"amount":12.5,"qty":2}` under `type: integer` never disclosed, and the refusal told a viewer
something about the payload. **Alternatives**: keep refusing all fractions (the defect); arbitrary
precision arithmetic in the validator (a dependency feature we do not enable, and a different
validator); refusing only above 2^53 for integers (would regress the tested exact large-integer modulo).
**Proof**: `crates/infra-schema/tests/classification_numbers.rs` (6 tests: decimals under `minimum`,
`maximum`, `exclusiveMinimum`, `const`, `enum`, `multipleOf: 0.25` and `0.01`, `uniqueItems`; mismatch
stays a mismatch; unsafe spellings stay refused) and the integration test on real PostgreSQL.

### D641 — One step budget bounds the whole walk (S1, C2, S7)

**Decision**: a projection spends at most 4,000,000 steps (`MAX_WALK_STEPS`) and five seconds
(`MAX_PROJECTION_DURATION`), whichever comes first, and ends as `SchemaUnusable` (an inspector sees
`structure-only` with `SCHEMA_UNUSABLE`, everyone else `NOT_DISCLOSABLE`). Every expanded subschema is
charged once for each instance node it applies to, which is also what the validator spends there, and
the charge is made in a precheck before `is_valid` runs. A keyword outside the walked set that descends
(`not`, `if`, `contains`, `propertyNames`, `dependentSchemas`, `unevaluated*`, a `$ref` inside them) is
charged per descendant. The schemas that govern every remaining array item, and every key no schema
names, are computed once per container. Number spellings live in a tree shaped like the payload, so the
bookkeeping is linear in the payload instead of one path string per number. The 1,000 `$ref` expansion
limit now applies to one schema location, as publication already counted it, so an array of more than
1,000 referenced items discloses; a cycle still ends as an unusable carrier. **Rationale**: the walk
runs inside the read transaction; its cost grew with payload times carrier. **Alternatives**: a payload
node limit alone (does not bound the carrier side); cancelling the validator by timeout (it cannot be
interrupted); memoising expansion per schema pointer (does not bound the product).
**Reproduced and fixed**: 120 nested keys of 1 KiB over 60,000 integers retained about 7 GB (scaled to
1/20 it peaked at 413 MB on the old code); it now runs in 0.02 s with 20 MB. 20,000 `allOf` branches
under `items` with 131,000 elements did not finish in 60 s on the old code; it now stops at the budget.
**Proof**: `crates/infra-schema/tests/classification_bounds.rs` (8 tests) and four walk unit tests.

### D642 — The carrier compiles before the transaction, once per version (C5, S5)

**Decision**: the store reads the pinned version and the selector's carrier without a transaction and
asks the evaluator to `prepare` it, which compiles and caches the validator; only then does it open the
transaction, where it checks the version is the one prepared and repeats every authority check. The
compile is single-flight per key (other callers wait for its result), different keys compile together up
to four permits, a cached hit never waits, a panicking compiler releases its flight, and a carrier that
cannot be compiled is remembered in a separate table of 1,024 entries so it costs one attempt and does not
evict a usable entry. Preparation and projection run through `block_in_place` on a multi-thread runtime.
**Rationale**: the old cache compiled under a process-wide `std::sync::Mutex`, inside the transaction
that holds the run, namespace, membership and grant locks, on an async worker, and never remembered a
failure. The version is immutable (the `document` is covered by the immutability trigger), so what is
read before the transaction cannot differ from what the transaction reads. **Proof**:
`tests/fault/payload-classification/races.rs` (`a_cold_carrier_compiles_before_the_transaction_...`
asserts no `idle in transaction` backend while the compile runs and that `project` only hit the cache;
`a_projection_in_progress_does_not_occupy_the_only_runtime_worker` runs on one worker) and seven cache
unit tests.

### D643 — The cache budget is estimated compiled bytes, with a pattern cap (C4)

**Decision**: an entry weighs 4 KiB plus 64 bytes per serialized schema byte plus 128 KiB per regular
expression; the budget is 64 MiB of that estimate (it was 16 MiB of serialized bytes) with the 256-entry
limit kept; a carrier may hold at most 32 `pattern` keywords and `patternProperties` keys; each regular
expression is limited to 64 KiB of program and 64 KiB of lazy DFA (it was 1 MiB each). **Rationale**: the
committed benchmark retained 414 MiB live at the old "16 MiB" limit, and a 64 KiB carrier of patterns
could compile to hundreds of megabytes (1,400 simple patterns measured 17.8 MB; one pattern at the old
limit could be 1 MiB). Measured ratios on this machine: 21 to 54 compiled bytes per serialized byte
(wide object, 3,000-branch `allOf`, large `enum`, minimal schema) and up to 79 KB per pattern at the new
limit. The weight rounds those up so it is an upper bound; the benchmark asserts it for five shapes.
**Supersedes** the weight of D281 (the key, the LRU and the process scope stand). The numbers are design
values; O641 asks the owner to confirm them. **Proof**: `classification_cache.rs` (5 tests) and the
benchmark report `tests/performance/reports/payload-classification.md`.

### D644 — A caller who may not inspect never learns why or how big (S2)

**Decision**: `sizeBytes` (the size of the whole stored payload, personal and secret fields included) and
`UNSUPPORTED_FORMAT` go only to a caller with `run:payload:inspect`. Every other caller gets
`NOT_DISCLOSABLE` for every failure except `NO_RETAINED_PAYLOAD`, and a `disclosed` answer carries no
`sizeBytes`. `sizeBytes` is therefore optional on the `disclosed` variant in the OpenAPI schema, the
contract README and the generated Studio types. **Rationale**: D269 and D278 promised no data-dependent
answer for these callers; the old code gave `UNSUPPORTED_FORMAT` for unsupported or too-large payloads
(a one-bit oracle, and data-dependent through any fraction in a secret field) and `sizeBytes` to all.
**Proof**: `the_stored_size_goes_only_to_a_caller_who_may_inspect` and the closed-failures tests.

### D645 — A structure answer is audited as an inspection (C3)

**Decision**: a `structure-only` answer (legacy version, no carrier, mismatch, unusable carrier) writes
`payload.inspected` in the read transaction, reason `classified-v1:<selector>:structure`, with the 300 s
collapse of ADR-0028. **Rationale**: ADR-0028 audits the same structure read and has no exemption; every
run started before the feature is `LEGACY_VERSION`, so every such read was unaudited. No new ADR: this
follows ADR-0028. **Proof**: the use-case test over every failure and role, and the PostgreSQL test of a
legacy run (one row for two reads, none for a developer or a viewer).

### Gaps and owner questions of this section

| # | Gap | Treatment | Status |
|---|---|---|---|
| G640 | A key of a map governed only by `additionalProperties` is named at the level of its value (D279), so an email used as a key is shown to a viewer when the values are public; authors cannot classify keys (S4). | Behaviour kept and pinned by `pending_decision_keys_of_an_additional_properties_map_...`. The disclosure-side fix (name the key only for `audience.pii`) contradicts D279 and needs the owner (O640). The publication-side mitigation, `CLASSIFICATION_INHERITED_OPEN` for `items` and `additionalProperties` children, is in `publication.rs`, which the surface branch owns. | Open (O640) |
| G641 | An inspector's `disclosed` answer shows the shape of withheld nodes and the real counts but is audited only when it returns an `internal` or `pii` value. | Same rule as D269 (a public-only read is a metric); inspectors are few, so auditing every inspector read as `payload.inspected` is a small change if the owner wants it. | Open |
| G642 | Publication does not warn that a carrier has more than 32 patterns or that a read would exceed the walk budget; the runtime refuses it as an unusable carrier. | `publication.rs` belongs to the surface branch; the counts are exported constants (`MAX_CARRIER_PATTERNS`, `MAX_WALK_STEPS`). | Open |
| G643 | A payload whose numbers cannot be validated shows an inspector `UNSUPPORTED_FORMAT` and no structure, because the structure-only wire reasons do not name it. | A new reason (for example `UNSUPPORTED_NUMBER`) is a contract change in OpenAPI, the SDKs and Studio; not made here. | Open |
| G644 | Numbers in the schema (bounds, constants, divisors) reach the validator as `f64` after parsing, so their source spelling is not checked; a constant with more digits than an `f64` keeps compares as its `f64`. | Documented in the runbook; the payload side is exact. | Accepted |
| G645 | The per-descendant charge for descending keywords outside the walked set can refuse a legitimate schema that puts a large conditional (`if`/`then`) on every element of a very long array. | Fails closed as an unusable carrier, which an inspector sees as `SCHEMA_UNUSABLE`; raise `MAX_WALK_STEPS` after measurement if it bites. | Accepted |
| G646 | The compiled-validator cache is shared by all tenants, so a tenant that publishes and reads many large versions can evict other tenants' entries (a cost, not a leak: keys are global version ids). | The weight budget bounds memory; a per-tenant partition or quota is not built. | Open |
| G647 | The weight bounds allocator-requested heap of a compiled entry, not RSS: regular-expression matchers also keep per-thread caches while serving requests. | The benchmark states `heap` semantics; the pattern cap and the 64 KiB limits bound each matcher. | Accepted |

| # | Question for the owner | Options | Recommendation |
|---|---|---|---|
| O640 | Should the key of a map governed only by `additionalProperties` follow its value's level (D279 today) or be at least `pii`, shown only to a caller revealing PII? | (a) keep D279 and warn authors at publication; (b) name such keys only when the audience includes `pii`, so a viewer sees values without keys; (c) let authors classify keys (a new annotation under `propertyNames`). | (b): maps keyed by an identifier are common, and (b) needs no new schema syntax; it changes what a viewer sees for every such map, hence the question. |
| O641 | Confirm the cache budget (64 MiB of estimated compiled bytes, 256 entries, 32 patterns per carrier, 64 KiB per regular expression) or choose other numbers. | The estimate is an upper bound measured on one machine (D643). | Keep; revisit with production telemetry (`classification_cache_total`). |

## Surface fixes after the two independent reviews (2026-10-25)

Two independent reviews of the integrated branch (a security review and a correctness review, both
read-only) found defects in the publication, diff, event, grant and evidence surface. This section
records what was decided while fixing them. The numbers D660+, G660+ and O660+ are this branch's;
D640+, G640+ and O640+ belong to the engine-side fixes made at the same time (walker, validator cache,
disclosure store, decimals, budgets). Findings are cited as S for the security review and C for the
correctness review.

### D660 — An annotation under any key the walker does not read is refused (S6, C7)

**Decision**: publication refuses `x-sagat-classification` under every key of a schema except
`properties`, `items`, `prefixItems`, `additionalProperties`, `allOf`, `anyOf`, `oneOf`, `$defs` and a
local `$ref` target (`CLASSIFICATION_UNSUPPORTED_KEYWORD`, at the path of the key). Property names and
`$defs` names are not keywords. An array under `items` is refused too. `CLASSIFICATION_INHERITED_OPEN` is
raised for the children declared by `items` and `additionalProperties` as it was for `properties` and
`prefixItems`. A 422 lists errors only in `details.issues`. **Rationale**: the list of 11 refused
applicators let `contentSchema`, `additionalItems`, `dependencies` or a vendor key hold an annotation
that nothing reads, which left the field `pii` with no feedback (US01.4). **Alternative** rejected: grow the
list (every new keyword of a JSON Schema draft would reopen the hole).

### D661 — Every version says which rules it was published under (C9)

**Decision**: `VersionSummary` (publish, version read and list, the active version of a workflow) returns
`classificationRules`. Publishing the document of an existing version still returns that version found by
workflow and digest (`created: false`), so a legacy document stays at rules `0`; the answer now says so.
**Alternative** rejected: re-pin the old version (versions are immutable) or publish a new version for an
identical document (it would break the digest identity that rollback and promotion rely on).

### D662 — A `secret` node: omitted, withheld, never shown (spec alignment)

**Decision**: FR-016-009 and D278 now say what the contract matrix and the code do: a `secret` node is
omitted for a caller without `run:payload:inspect`, withheld with the reason `SECRET` for one who holds it,
and its value is shown to nobody. The earlier text ("withheld for everyone") disagreed with the matrix.

### D663 — A change of role removes the subject's reveal grants (S10)

**Decision**: a trigger on `memberships` deletes the subject's `payload_reveal_grants` in every namespace
when `role` changes (migration `20261220000000`). A grant belongs to the role held when it was granted; an
administrator grants again after the change. **Rationale**: D274 made a demoted grant inert but kept the
row, so promoting the member again revived a right nobody renewed. Storing the role and comparing for
equality would still revive a developer who was demoted and promoted back. Any change of role (also a
promotion) removes the grants: simpler than ranking roles, and no role table is copied into SQL.
**Alternative** rejected: store the role at grant time and require it to be at least the same.

### D664 — The version diff compares what is disclosed, walks each side once (C8)

**Decision**: a side with no schema for a location (a carrier added or dropped), or published under rules
`0`, counts as `undisclosed`, as strict as `secret`; adding an `outputSchema` that makes a field `public`
is `CLASSIFICATION_RELAXED`, dropping one is `CLASSIFICATION_TIGHTENED`, and a legacy version compared
with the same document under the current rules relaxes every location its annotations make readable. The
first draft of a workflow (nothing to compare with) is not compared. The application passes the rules of
each side (`IrCodec::diff_with_rules`). Each document is walked once into an index
(`ClassificationLocationIndex`) and each location is looked up in the other one's; before, every location
started three walks of the whole carrier (130 s in a debug build for a 52 KB carrier of 2,000 properties,
inside the async handler). The cost is asserted in walk steps (`ClassificationWalkStatus.steps`, five per
location and side), not in time. The attention codes `CLASSIFICATION_TOO_COMPLEX`, `_UNRESOLVED_REF`,
`_UNSUPPORTED_KEYWORD` and `MORE_FINDINGS` are described in the OpenAPI `DiffFinding`, the runbook and the
Studio (the settings panel shows a notice for carriers that were not compared).

### D665 — Event data the projector cannot shape is `opaque`, not a 403 (C, lower)

**Decision**: `getInboxEvent` returns `dataStructure: {"kind": "opaque"}` when the stored data is not
bounded JSON the projector accepts, as a dead letter already did. The earlier code answered
`EventReadNotGranted` (a 403 for a server-side limit, which told the caller they lacked `event:read`).

### D666 — Event and dead-letter reads audit cross-tenant probes; the unredacted readers are gone (S3, S8, S9, S11)

**Decision**: the prepare step of the three event reads calls the shared `require_namespace`, so a probe of
a namespace of another tenant is the opaque `404` and leaves a `CrossTenantAccessDenied` record against
the prober (the old `EventService::event` did; the value-safe service had lost it). Dead letters were not
audited before and are now. `EventService::event`, `ConnectorService::dead_letters` and `::dead_letter`
(unredacted readers no route calls since D302) and the store read of a whole inbox event are deleted; the
store keeps what redrive needs. The response types that hold revealed values (`EventBody`,
`DeadLetterBody`) print state and sizes only in `Debug`. `Audience::new` keeps `pii` only when `internal`
is set, so an audience that reads `pii` and not `internal` cannot be built.

### Gaps

| # | Gap | Treatment | Status |
|---|---|---|---|
| G660 | The removal of grants by a change of role (D663) and by the cascade of a deleted membership leaves no `payload.reveal_revoked` record: there is no membership management surface to audit (G290), and the trigger runs inside whatever statement changed the role. | The change of the membership itself is the audited act once that surface exists; until then the database owner who changes a role knows the effect. Revisit with G290. | Open |
| G661 | The first version of a legacy workflow under the current rules lists a `CLASSIFICATION_RELAXED` finding per readable location (100, then `MORE_FINDINGS`): accurate, and noisy for a large carrier. | O660. | Open |
| G662 | `T016-053` (benchmark) and `T016-054` (workspace gates) are not closed: the benchmark report states that the registered target and the cache tests must pass first and that the cache does not bound compiled-validator memory (414 MiB at the 16 MiB limit); the workspace gates were never run as one set on the integrated tree. | Unticked in `tasks.md`; the cache weight is an engine-side fix (D640+). Close both with the commands of the task on the merged tree. | Open |
| G663 | `tasks/project-completion/` (about 1.7 million lines of command output) is not in git; 56 evidence lines and about 288 lines of specs and docs cite it. | The evidence lines say so; a tick stands only on a test that is in the repository and can be rerun. The output is kept outside git (local ref `handoff/016-project-completion-20261024`). | Accepted |
| G664 | `npm audit --audit-level=moderate` in `frontend/` fails on this branch (2026-10-25): vitest and `@vitest/mocker` <= 4.1.11 (moderate), `dompurify` <= 3.4.16 (critical; the root `package.json` overrides it to 3.4.16) and `undici` 8.0.0 to 8.12.1 (high). No package or lockfile changed here. | Not fixed in this change (a dependency bump is its own change); it belongs to the advisory part of T016-054 and to whoever owns the Studio dependencies. | Open |

### Owner questions

| # | Question | Default taken |
|---|---|---|
| O660 | Should the first version of a legacy workflow under the current rules list a relaxation per location (D664), or one finding per carrier that says how many locations became readable? | Per location, bounded at 100 with the exact remainder; it follows US01.7 ("an attention finding for each"). |
| O661 | Should a promotion (for example `operator` to `admin`) keep the member's reveal grants, and only a demotion remove them (D663)? | Any change of role removes them; the administrator grants again. Simpler, and it needs no ranking of roles. |
