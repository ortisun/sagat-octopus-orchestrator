# Spec Kit consistency analysis

Date: 2026-10-16, revised 2026-10-17 after the owner's answers. Three reviews of this set: a consistency review of the documents against each other,
then a validation of them against ADR-0033, ADR-0028, ADR-0031 and the code of this branch, then a
review of what the owner's answers of 2026-10-17 changed. This is a
design review. No runtime behaviour was built or run, and nothing here is evidence that a requirement
holds in an implementation.

## Result

| Measure | Result |
|---|---|
| User stories | 8 (5 at P1, 2 at P2, 1 at P3) |
| Functional requirements | 29 |
| Non-functional requirements | 6 |
| Success criteria | 10 |
| Requirement/criterion coverage by tasks | 45/45 (every FR, NFR and SC appears in the `Refs` of at least one task) |
| Tasks | 65 (T016-001 to T016-065). At the design review 1 was ticked (T016-002). On 2026-10-25: 62 ticked, 3 open (T016-001 closes when the set is merged; T016-053 and T016-054 are open, see the review below) |
| Tasks without file paths or `Refs` | 0 |
| Decisions / gaps | D261-D290, D302-D306 and D660-D666 (42) / G272-G293 and G660-G664 (27); owner questions O660-O661 with defaults |
| Product questions for the owner | 4 asked (P1-P4), 4 answered on 2026-10-17; none open |
| Unresolved clarification markers | 0 |
| Findings of the validation against ADRs and code | 15, all corrected in the documents or recorded as gaps; 3 of them were put to the owner and are now decided |
| Findings of the review of the owner's answers | 10, all corrected in the documents |

## First review: consistency among the documents

The review compared the audience matrix, the error reasons, the audit actions, the permission names, the
bounds, the task ids and the numbers of requirements across spec, plan, research, data-model, contracts,
quickstart, tasks, checklist, roadmap and traceability. It found and fixed:

| # | Finding | Correction |
|---|---|---|
| 1 | The spec said a granted developer who sends no `reveal` sees `pii` "withheld (`REVEAL_NOT_REQUESTED`)", but a developer lacks `run:payload:inspect`, and withheld nodes are only returned to holders of it. | The scenario now says no value is returned and that an inspector sees the withheld node with the reason (spec US03.3, research D273). |
| 2 | A reveal grant to a `viewer` would have disclosed `pii` while `internal` stayed hidden, which breaks the "a withheld node hides its whole subtree" argument of FR-016-009. | `pii` also needs `payload:read-internal`; a grant is refused for such a role (`ROLE_CANNOT_REVEAL`, the fifth reason); a later demotion makes it inert (D274, FR-016-007, FR-016-011, contracts 2.1 and 2.2). |
| 3 | The contract's `caller.revealPii` had three values, one of which ("not requested") duplicated what `revealed` says. | `revealPii` is `granted` or `not_granted`; `revealed` is a separate boolean (contracts 2.1). |
| 4 | The audit table for grants gave a vague resource detail. | `ResourceType::PayloadRevealGrant` with id `namespace-id/subject`, the form `Worker` already uses (contracts section 3, data-model section 1). |
| 5 | The requirement to report effective levels in `validate` named no field. | `Validation.classification` (bounded to 500 entries) added to the contract and to T016-008, T016-021, T016-022. |
| 6 | FR-016-016 (surfaces that stay unchanged) had no task. | T016-049 asserts the listed surfaces answer byte-identically to their pre-feature golden responses. |
| 7 | Task ids cited in plan, contracts and research did not match the final numbering. | All references were renumbered and checked by script (no id above T016-065, no dangling D or G reference). |
| 8 | The audience-matrix link in the spec pointed at a paragraph, not a heading. | The matrix has a heading (`#### Audience matrix`). |
| 9 | The data-model described projection after the transaction, while the audit has to precede the bytes and describe the answer. | Projection is computed inside the read transaction through a callback; the lifecycle table has separate project, audit and commit steps (data-model section 4). |
| 10 | The quickstart used a `Namespace` header that run operations do not take, a doubled `/v1` prefix and a link-check script that does not exist. | It uses run paths with no namespace header, `$SAGAT_SERVER` (which already ends in `/v1`) and the check recorded below. |

## Second review: validation against ADR-0033, ADR-0028, ADR-0031 and the code

| # | Finding | Evidence | Correction |
|---|---|---|---|
| 11 | ADR-0033 says every surface keeps ADR-0028's structure-only behaviour until this feature, and that event data is a payload treated as confidential. `getInboxEvent` returns `data` and `listConnectorDeadLetters` returns `body` and `headers` to any `event:read` holder, viewers included. | `specs/001-platform-foundation/contracts/openapi.yaml` (`getInboxEvent`, `listConnectorDeadLetters`); `crates/api-http/src/dto_events.rs` (`EventBody.data`, `DeadLetterBody.body`); `crates/application/src/connectors.rs` (`EventRead`). | Not hidden: recorded as G272 and put to the owner as P2 with three options. The owner decided on 2026-10-17 to restrict the values now (D302; US08, FR-016-026 to FR-016-029, tasks T016-057 to T016-065); see the third review. |
| 12 | The brief and the ADR assume schemas exist for activity outputs, event data and declared state. Only the workflow `inputSchema` and `outputSchema` exist, and the IR checks them only as `type: object`. Activity, script, wait and child nodes have no output schema, events have none, and state and operation schemas belong to feature 014 (not on this branch). | `workflow.schema.json`; `crates/infra-ir/src/convert.rs` header (extra node fields are allowed and ignored). | Carriers are a closed list; node-level `outputSchema` is added as additive IR (D262); event schemas are a gap (G282); state and operation schemas are rejected as carriers (D263, G285). |
| 13 | `Permission::ALL` is the Admin role. Adding the new permission to `ALL` would silently grant `payload:reveal-pii` to every admin, contradicting "granted explicitly". | `crates/domain/src/identity.rs` (`Role::permissions`, `Admin => &Permission::ALL`). | Admin becomes `ALL` minus the reveal permission through an explicit constant with a test against `ALL` (D276, T016-010, T016-012). |
| 14 | Memberships can only be created by `provision-tenant` and `seed-dev` (one administrator). There is no REST resource or CLI command to add a member. A per-membership flag (option B) needs a new management surface exactly like a grant list; the test suites insert members through `tests/support`. | `apps/control-plane/src/main.rs`; `docs/runbooks/control-plane.md` section 8; no membership path in `openapi.yaml`. | P1 prices both options with that cost; G290 records the missing creation path; the quickstart states how members are made. |
| 15 | An inline run input is not validated against `inputSchema` at start (only a claim-check descriptor is, via `RunInputChecker`). The ADR's "values that do not validate are shown as structure only" will therefore apply often. | `crates/application/src/runs.rs`, `blobs.rs`, `crates/infra-schema/src/lib.rs`. | Validity gate at read with a non-echoing check (D269); start-time validation is G273, a separate behaviour change. |
| 16 | `jsonschema` error text repeats instance values (`iter_errors` messages). The existing `infra_schema::validate` avoids it by printing only path and rule. | `crates/infra-schema/src/lib.rs`. | FR-016-008 and T016-014/016 forbid an error iterator on instance data; `is_valid` only. |
| 17 | `JournalEvent.payloadRef` is a plain SHA-256 of the payload, visible to `run:read`: a guessable value, of any level, can be confirmed. | `openapi.yaml` `JournalEvent`. | Not changed (content identity, ADR-0002/0023); recorded as G275 with options. |
| 18 | The ADR says the classification is immutable with the version. The document is immutable, but the interpreter is code that a later release can change. | `workflow_versions_immutable` trigger; no rules version exists. | `workflow_versions.classification_rules` pins the interpretation; legacy is `0` and reads nothing (D270, FR-016-005). |
| 19 | A literal reading of "applies to everything under it" makes an undeclared property of a `public` object public. | ADR-0033 decisions 1 and 2. | The stricter floor of D265, flagged as P3 for confirmation. |
| 20 | Inheritance makes `public` on a root open every unannotated declared child. | ADR-0033 decision 1. | Kept as decided; mitigated by `CLASSIFICATION_INHERITED_OPEN`, the report and the diff findings (D264, G286, US07). |
| 21 | Declared Query and Update results (ADR-0031) are disclosed by their own explicit policy; an annotation inside their schemas would look protective and be ignored. | ADR-0031 decisions 1 and 4. | Rejected at publication (`CLASSIFICATION_NOT_APPLICABLE`); ADR-0031 is not changed (D263, FR-016-016). ADR-0031 and feature 014 live on another branch and are cited by name only. |
| 22 | `ApiOperation::PayloadInspect` charges namespace admission after authorization and run resolution. | `crates/application/src/api_admission.rs`. | The disclosure copies the order (`PayloadDisclose => [RunRead]`, FR-016-023). |
| 23 | SDK builders: Go and Python have `outputSchema`; TypeScript, Java and C# do not; none has a node-level option; no SDK has a client method for inspection. | `sdks/*` builders. | D284 and T016-040..042: setters and one golden, no helper; REST client methods are G281. |
| 24 | Roles: the code has six, the OpenAPI `AuthContext.role` enum lists four, and feature 009 G39 says "five". | `identity.rs`, `openapi.yaml`, 009 research. | The set says six (the code); the drift is recorded as G288 and left to a documentation pass. |
| 25 | `runs.result_digest` is not on this branch (it is on `bugfix/run-result`). | git branch contents. | `source: "result"` is its own task gated on that merge (T016-031, D288). |

## Third review: the owner's answers of 2026-10-17

P1 (per-namespace list), P3 (restrictive readings) and P4 (audited self-grant) confirmed what the set
already implemented, so they only moved from "to confirm" to decided, with D-ids and a date. P2 changed
the scope: the event values are restricted now. The review followed that change through the code and the
documents.

| # | Finding | Evidence | Correction |
|---|---|---|---|
| 26 | Three operations return event values: `getInboxEvent`, `getConnectorDeadLetter` and `listConnectorDeadLetters`. Redrive and discard return an `AcceptedBody` and `204`, no value. | `crates/api-http/src/routes/events.rs` (`get_event`), `routes/connectors.rs`, `dto_events.rs` (`EventBody`, `DeadLetterBody`, `AcceptedBody`). | US08, FR-016-026 to FR-016-029 and contracts section 2.4 cover exactly those three. |
| 27 | `InboxEvent.data` and `DeadLetter.headers`/`body` are in `required`, and the 004 REST conformance suite validates responses against the OpenAPI schema, so the schema change and the 004 scenarios have to change together. | `openapi.yaml` (`InboxEvent`, `DeadLetter`); `tests/contract/events-webhooks-messaging/rest/conformance.rs` (inbox and dead-letter scenarios). | T016-058 updates the scenarios, T016-060 the schema; the fields leave `required` and three disclosure fields join it. |
| 28 | A dead-letter list page can hold 200 letters of 64 KiB, so a reveal on the list would be a 12.8 MiB response with one audit record for 200 letters. | `crates/application/src/connectors.rs` (`dead_letters` clamps to 200), `MAX_DEAD_BODY`. | The list never reveals; the single resource reveals one letter with one audit record (D304). Accepted by the coordinator on 2026-10-17 as a technical security choice consistent with the owner's intent; decided. |
| 29 | The application services are called directly by `tests/integration/events-webhooks-messaging/connectors.rs` (`dead_letters(&w.admin, ...)` at several places), so the signature change breaks them. | that file. | T016-064 updates the callers. |
| 30 | The break reaches API clients only: no Studio screen reads event data or dead-letter bodies, the CLI has no event command, and the SDK clients only publish events. | greps over `frontend/apps/studio/src`, `apps/cli/src`, `sdks/*`. | G292 and T016-064 record and re-check it; the compatibility impact is documented (FR-016-028). |
| 31 | `viewer` holds `event:read` but not `payload:read-internal`, so a viewer cannot reveal event data even with a grant, which is consistent with D274 and with "the same reveal grant", but is a visible consequence for existing viewer integrations. | `crates/domain/src/identity.rs`. | Stated in D305, US08 scenario 3 and the audience table of contracts 2.4. |
| 32 | The events runbook says `GET events` gives "last error (never the payload)" and that a `DEAD` event "keeps its payload for inspection", and the API reference describes the old shape. | `docs/runbooks/events.md`, `docs/reference/api.md`. | T016-055 and T016-065 update them. |
| 33 | The pending wording ("pending owner decision", "recommended", "awaits the owner") survived in spec, plan, contracts, checklist and quickstart. | grep over the set. | Replaced by the decisions and dates; a script check finds none left. |
| 34 | T016-002 (record the owner's answers) is now done, so "no task is ticked" was no longer true in tasks, traceability and this file. | `tasks.md`. | T016-002 is ticked with its evidence; every count that said 0 ticked now says 1. |
| 35 | Feature 014 took D291 to D301 on `feature/workflow-primitives`, so this feature's D291 to D295 collided. | `git` branch contents of `feature/workflow-primitives`. | Renumbered to D302 to D306 everywhere (set, ADR-0033 addendum, owner decisions, roadmap, traceability); gaps keep G272 to G293; next free D307 and G294. The coordinator also accepted D304 (dead letters revealed one at a time) as decided. |

## Consistency with the decision records

- **ADR-0033**: levels, order, unmarked is `pii`, no loosening, the audience table, pinning, audited
  `pii` reveal, structure-only for non-validating values, no values in telemetry and retention out of
  scope are all carried. The three readings the set adds (undeclared floor, `pii` also needing
  `payload:read-internal`, an explicit `reveal` request) are decisions D265, D274 and D273; the owner
  confirmed the restrictive readings on 2026-10-17 (P3). Its statement that every surface is structure-only
  until this feature was inexact for events; an addendum to the ADR records the correction and the interim
  restriction (D302).
- **ADR-0028**: the structural endpoint, its permission, bounds and audit are unchanged; the new
  operation reuses its selector shape, transaction pattern, external-content rule and non-substitution
  rule, and returns its structure only to holders of `run:payload:inspect`.
- **ADR-0031**: unchanged; no declared operation result is filtered or reclassified.
- **ADR-0027**: admission charged once; the two new text columns are classified in
  `storage-classification.json`; the grant table is bounded by a per-namespace cap.
- **Constitution I-VIII**: checked in `plan.md`; principle VII now records that the event reads change and
  the event pipeline does not.

## Evidence and limits

Checks run on this set (not on any implementation), from the repository root of the worktree:

| Check | Command | Result |
|---|---|---|
| Spec Kit prerequisites | `SPECIFY_FEATURE_DIRECTORY=specs/016-payload-classification SPECIFY_FEATURE_NO_PERSIST=1 .specify/scripts/bash/check-prerequisites.sh --json --require-spec --require-tasks --include-tasks` | listed research, data-model, contracts, quickstart and tasks |
| Whitespace | `git add -A && git diff --cached --check` | no whitespace errors reported (exit 0), run on the staged set |
| Relative links | a script that resolves every relative Markdown link target of the new and edited files, outside code fences, and checks the heading anchors they name | 41 relative links and anchors checked (the new set, the roadmap, traceability, owner decisions and ADR-0033), 0 broken |
| Identifier and coverage scripts | extraction of D, G, T, FR, NFR and SC ids and their references | no dangling id; coverage 45/45 |

None of these proves that a design decision is right. The decisions that could be wrong in ways only
building will show are the walker's soundness (D266-D268), the cost of validating per response (D280,
D281) and the usability of the inheritance rule (D264). Each has a task that tests or measures it.


## Implementation checkpoint — 2026-10-17

The design reviews above are historical. Implementation is now in progress, with exact focused
results and outstanding gates in `tasks/project-completion/integration-notes.md`. Current combined
classification proof: 15 contract, 21 real PostgreSQL/HTTP, 2 deferred-audit failure tests and one
actual Chromium disclosure journey. Full canary, process-crash, upgrade, runtime conformance and
workspace release acceptance remain open. Canonical task checkboxes name only complete evidence.


### Executed classification quickstart and guide reconciliation

Added complete `specs/016-payload-classification/examples/` workflows (original/relaxed/invalid),
synthetic input and a canned mock worker response. Actual standalone CLI validation passed both
sound documents and refused the invalid one at the expected CLASSIFICATION_LOOSENED pointer.
The new `quickstart::the_documented_classified_orders_complete_and_keep_their_original_pin`
passed 1/1 in 21.00 s (`/tmp/sagat-completion-classification-quickstart-3.log`). It runs the actual
`sagat` and `curl` processes against HTTP/gRPC/PostgreSQL and the production engine entry point:
publish, worker-dev binding/execution, terminal result, actual step sequence, public/internal/PII
reads, grant/revoke and refusal, republish/activate and unchanged old run pin. It also publishes an
actual event, executes the four documented event/letter reads and checks the audit API for data
absence. The retained paused-connector letter is explicitly an isolated fixture; no Kafka delivery
is claimed by this walkthrough. The first compile needed named arrays for async CLI arguments;
this was a harness lifetime correction before execution.

The guide now points at real files, uses valid Cargo filter invocations, includes the worker/run
commands and version activation and explains receiving-carrier secret declarations. Its spec
prerequisite command passed (`/tmp/sagat-completion-classification-spec-prerequisites.log`); the
PayloadDisclosure/WorkflowSettings tests passed 8/8 in 1.96 s
(`/tmp/sagat-completion-classification-guide-ui.log`). Developer-experience references/links/CLI
syntax/secret scan passed 8/8 (`/tmp/sagat-completion-classification-guide-references.log`); strict
Clippy of the three classification targets passed with warnings denied in 27.81 s
(`/tmp/sagat-completion-classification-final-focused-clippy.log`).

The benchmark, transaction/process faults, actual old release/old Studio/five-client compatibility
and Chromium commands in the guide have their separate executed evidence recorded above; those
are reused where the tested production path is unchanged, not invented reruns. The requirements
checklist now distinguishes executed local evidence from the still-open full workspace/release
gates. T016-056 closes for the delivered guide and traceability; T016-054 remains open.


## Review of the integrated branch — 2026-10-25

Two independent read-only reviews (security and correctness) of the integrated branch found defects.
The surface half is fixed on `feature/016-fix-surface` (publication, diff, events, grants, response
types, specification and evidence); the engine half (walker, validator cache, disclosure store,
decimals, budgets, audit of structure reads, keys admitted by `additionalProperties`) is recorded under
D640+ by the branch that fixes it. The decisions are D660 to D666 and the gaps G660 to G664 in
[research.md](./research.md#surface-fixes-after-the-two-independent-reviews-2026-10-25).

What the review changed in this set:

| Finding | Correction |
|---|---|
| T016-054 was ticked with no evidence while this file said it was open | Unticked; the gates run on the merged tree (G662) |
| T016-053 was ticked while its own report says the target and the cache tests must pass first, and the cache holds about 26 times the weight it counts | Unticked (G662) |
| T016-047 cited a test of the format of one `prepare` line for the diff findings | The evidence names `classification_diff.rs` and the other tests that prove it |
| T016-050 cited a test that drives the grant helper, not a reveal | A test pauses a production reveal and revokes and deletes the membership while it runs |
| T016-056 cited a test that hard-coded its commands | The test reads `quickstart.md` and compares |
| 56 evidence lines cite `tasks/project-completion/`, which is not in git | Each says "(outside git)"; a tick stands on a test in the repository (G663) |
| FR-016-009 said a `secret` node is withheld for everyone, the matrix and the code omit it for a caller without `run:payload:inspect` | The text says what the matrix says (D662) |
| US01.6 did not say what republishing an identical legacy document does | It does, and `classificationRules` is returned (D661) |

Nothing in this review is evidence that a requirement holds beyond the tests named in `tasks.md`.
