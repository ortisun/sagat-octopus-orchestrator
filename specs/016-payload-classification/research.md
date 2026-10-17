# Research: per-field payload classification

**Date**: 2026-10-16. The direction is decided by ADR-0033 (owner decision of 2026-10-16). This file
records what the repository was found to contain, the design decisions D261 onward that fill the points
ADR-0033 leaves to the specification, the gaps G272 onward, and the owner's answers of 2026-10-17 to the
four product questions P1 to P4 (D275, D291 to D295, D265). Nothing here is implemented. No new ADR is
proposed (see D290).

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
| `openapi.yaml` `getInboxEvent` returns `data`; `listConnectorDeadLetters` returns `body` and `headers`; both need only `event:read` (viewer holds it); `dto_events.rs` `EventBody`, `DeadLetterBody` | **Existing value disclosure** that contradicts the premise of ADR-0033 (G272); restricted by D291. |
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
withheld for everyone and never a value. **Rationale**: ADR-0028 treats structure as sensitive metadata
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

### D291 — Event values are restricted now, before event schemas exist (owner decision P2, 2026-10-17)

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

### D292 — Shape of the event answer: conditional `data`, structure for everyone else, explicit `reveal=pii`

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

### D293 — Dead letters: the single resource reveals, the list never does

**Decision**: `getConnectorDeadLetter` with `reveal=pii` (and the grant) returns `body` and `headers`; the
single resource without it, and every item of `listConnectorDeadLetters`, return `bodyDisclosure:
"structure-only"`, `bodySizeBytes`, `bodyStructure` (the structure of a bounded JSON body, otherwise
`{"kind": "opaque"}`) and `headerCount` (no header name, no value). The list ignores `reveal`.
**Rationale**: a page of 200 letters of up to 64 KiB would be a 12.8 MiB audited response, and one audit
record per page would not say which letters were read; the single resource gives one record per letter
(`dead-letter:<id>`). A dead letter exists because the message was not an event, so its body is often not
JSON, which is why `opaque` exists. **Alternative**: reveal on the list with a smaller page (more code,
less precise audit). This choice goes beyond the owner's words and is reported for confirmation.

### D294 — The event restriction reuses the grant, the permissions, the reason and the audit action

**Decision**: no new table, permission, `Error.code` or `details.reason`: the check is `event:read`
plus `payload:read-internal` plus the grant of the event's own namespace (the connector's for a dead
letter), a refusal is `403 PII_REVEAL_NOT_GRANTED`, the audit action is `payload.pii_revealed` with
`ResourceType::Event` (id `source/event id`) or `ResourceType::Connector`, never collapsed, written in the
transaction that reads the payload. The structure uses the existing `PayloadProjector` port and
`JsonPayloadProjector`. A `viewer` therefore cannot reveal event data even with a grant (the grant is
refused for that role, D274). **Rationale**: one reveal concept across runs and events; the grants, the
CLI and the audit trail do not learn a second one.

### D295 — An administrator may grant the reveal permission to themselves, audited (owner decision P4, 2026-10-17)

**Decision**: the grant operation does not exclude the caller as the subject; the record is the same
`payload.reveal_granted` with the actor and the subject visible. **Rationale**: a tenant with one
administrator would otherwise be unable to reveal anything (members other than the bootstrap
administrator cannot even be created, G290), and G49 already treats a tenant administrator as trusted.
**Alternative** rejected: four-eyes (a second person must grant), with a lockout risk for small tenants.

## Gaps

| # | Gap | Treatment | Status |
|---|---|---|---|
| G272 | **The event inbox and dead letters disclosed values before this feature.** `getInboxEvent` returned `data` and `listConnectorDeadLetters` returned `body` and `headers` to any `event:read` holder, viewers included, against ADR-0033's premise. | Restricted in this increment by D291 to D294 (US08): values need the reveal grant, everyone else gets structure. Field classification of events remains open (G282). | Resolved by restriction (owner decision P2, 2026-10-17); classification open
| G273 | An inline run input is not validated against `inputSchema` at start (only a claim-check descriptor is). | Disclosure validates at read (D269); non-conforming inputs show as structure only. Start-time validation is a separate behaviour change. | Open |
| G274 | Until an author adds an `outputSchema` to a node, its output cannot be disclosed. | Adoption: `validate` reports activities without one as informational in the Studio panel; no default schema. | Accepted |
| G275 | `JournalEvent.payloadRef` is a plain SHA-256 of the payload, readable by any `run:read` holder; a guessable payload (even a `secret` one) can be confirmed by hashing a guess. | Not changed: the digest is the payload's content identity (ADR-0002, ADR-0023). Options if the owner wants it closed: expose an HMAC under a per-tenant key (breaks equality across tenants, which is fine, and any client comparing digests), or hide the field for viewers. | Open |
| G276 | No taint tracking across expressions, child inputs and results (D283). | Documented; mitigated by report and review. | Accepted |
| G277 | External (blob) payload values are not disclosed. | `external/EXTERNAL_CONTENT_NOT_INSPECTED`, as ADR-0028. Reading a blob for disclosure needs a size policy and a streaming validator. | Open |
| G278 | Script execution logs (`ExecutionBody.log`, readable with `script:read`) and script test-run input/result are author-controlled text and test data, outside classification. | Documented boundary; log quotas are ADR-0027's. | Accepted |
| G279 | A `secret` field is stored in `payloads` like any value; no encryption at rest of payload content. | Documented (D287); disk and backup encryption are deployment concerns of feature 010. | Accepted |
| G280 | No Studio screen for reveal grants. | REST and CLI only (D285). | Open |
| G281 | No SDK REST-client method for the disclosure or the grants. | Out of scope; the five builders get the schema setters only (D284). | Open |
| G282 | No event type schema registry, so no carrier for inbox data (the next step after G272, which the restriction of D291 stands in for). | Candidate design: a namespace-level `event-types` resource with a schema, pinned by digest on each event. Not designed here. | Open |
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
| P2 | What happens to the event inbox `data` and the connector dead-letter `body` that disclosed values to every `event:read` holder? | **Restrict them now**: they need the same reveal grant; everyone else gets structure only; field classification of events waits for event type schemas. Rejected: leave them and record the exception; restrict by a plain permission with no classification; classify through a consuming `waitEvent` or trigger. This changed the scope (US08, FR-016-026 to FR-016-029, SC-016-010, T016-057 to T016-065). | D291 to D294 |
| P3 | Two readings of ADR-0033: (a) an undeclared field under a `public` parent; (b) a schema with no annotation, and a payload with no schema. | The **restrictive** reading: (a) it is at least `pii`; (b) a schema with no annotation is `pii` throughout (a reveal holder sees it), a payload with no schema at all is structure only for everyone. | D265, D269 |
| P4 | May an administrator grant `payload:reveal-pii` to themselves? | **Yes**, audited. Rejected: four-eyes. | D295 |

One detail goes beyond the owner's words and is reported for confirmation: a dead letter is revealed on
the single resource only, never on the list (D293).

## What is not decided

Not decided and not designed: field classification of events and the event type schema registry that
would carry it (G282), whether a dead letter should also be revealable on the list (D293, reported for
confirmation), erasure and retention per regulator (G283, G284), taint tracking (G276), a Studio grants
screen (G280), SDK client methods (G281), whether `payloadRef` should stop being a plain digest (G275), a
membership management surface (G290) and the production values of the bounds (G287). The design values of
D280 are not capacity claims.
