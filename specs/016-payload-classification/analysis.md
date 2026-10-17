# Spec Kit consistency analysis

Date: 2026-10-16. Two reviews of this set: a consistency review of the documents against each other,
then a validation of them against ADR-0033, ADR-0028, ADR-0031 and the code of this branch. This is a
design review. No runtime behaviour was built or run, and nothing here is evidence that a requirement
holds in an implementation.

## Result

| Measure | Result |
|---|---|
| User stories | 7 (4 at P1, 2 at P2, 1 at P3) |
| Functional requirements | 25 |
| Non-functional requirements | 6 |
| Success criteria | 9 |
| Requirement/criterion coverage by tasks | 40/40 (every FR, NFR and SC appears in the `Refs` of at least one task) |
| Tasks | 56 (T016-001 to T016-056), 0 ticked |
| Tasks without file paths or `Refs` | 0 |
| Decisions / gaps | D261-D290 (30) / G272-G290 (19) |
| Product questions for the owner | 4 (P1-P4) |
| Unresolved clarification markers | 0 |
| Findings of the validation against ADRs and code | 15, all corrected in the documents or recorded as gaps; 3 leave explicit owner decisions |

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
| 7 | Task ids cited in plan, contracts and research did not match the final numbering. | All references were renumbered and checked by script (no id above T016-056, no dangling D or G reference). |
| 8 | The audience-matrix link in the spec pointed at a paragraph, not a heading. | The matrix has a heading (`#### Audience matrix`). |
| 9 | The data-model described projection after the transaction, while the audit has to precede the bytes and describe the answer. | Projection is computed inside the read transaction through a callback; the lifecycle table has separate project, audit and commit steps (data-model section 4). |
| 10 | The quickstart used a `Namespace` header that run operations do not take, a doubled `/v1` prefix and a link-check script that does not exist. | It uses run paths with no namespace header, `$SAGAT_SERVER` (which already ends in `/v1`) and the check recorded below. |

## Second review: validation against ADR-0033, ADR-0028, ADR-0031 and the code

| # | Finding | Evidence | Correction |
|---|---|---|---|
| 11 | ADR-0033 says every surface keeps ADR-0028's structure-only behaviour until this feature, and that event data is a payload treated as confidential. `getInboxEvent` returns `data` and `listConnectorDeadLetters` returns `body` and `headers` to any `event:read` holder, viewers included. | `specs/001-platform-foundation/contracts/openapi.yaml` (`getInboxEvent`, `listConnectorDeadLetters`); `crates/api-http/src/dto_events.rs` (`EventBody.data`, `DeadLetterBody.body`); `crates/application/src/connectors.rs` (`EventRead`). | Not hidden and not changed: recorded as G272 and put to the owner as P2 with three options; FR-016-016 and the spec context say so; traceability lists it as a known exception. |
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

## Consistency with the decision records

- **ADR-0033**: levels, order, unmarked is `pii`, no loosening, the audience table, pinning, audited
  `pii` reveal, structure-only for non-validating values, no values in telemetry and retention out of
  scope are all carried. The three readings the set adds (undeclared floor, `pii` also needing
  `payload:read-internal`, an explicit `reveal` request) are decisions D265, D274 and D273; the ones that
  refine the text are listed for confirmation (P3).
- **ADR-0028**: the structural endpoint, its permission, bounds and audit are unchanged; the new
  operation reuses its selector shape, transaction pattern, external-content rule and non-substitution
  rule, and returns its structure only to holders of `run:payload:inspect`.
- **ADR-0031**: unchanged; no declared operation result is filtered or reclassified.
- **ADR-0027**: admission charged once; the two new text columns are classified in
  `storage-classification.json`; the grant table is bounded by a per-namespace cap.
- **Constitution I-VIII**: checked in `plan.md`; the exception for the event inbox is stated under VII
  instead of being left out.

## Evidence and limits

Checks run on this set (not on any implementation), from the repository root of the worktree:

| Check | Command | Result |
|---|---|---|
| Spec Kit prerequisites | `SPECIFY_FEATURE_DIRECTORY=specs/016-payload-classification SPECIFY_FEATURE_NO_PERSIST=1 .specify/scripts/bash/check-prerequisites.sh --json --require-spec --require-tasks --include-tasks` | listed research, data-model, contracts, quickstart and tasks |
| Whitespace | `git add -A && git diff --cached --check` | no whitespace errors reported (exit 0), run on the staged set |
| Relative links | a script that resolves every relative Markdown link target of the new and edited files, outside code fences, and checks the heading anchors they name | 39 relative links and anchors checked, 0 broken |
| Identifier and coverage scripts | extraction of D, G, T, FR, NFR and SC ids and their references | no dangling id; coverage 40/40 |

None of these proves that a design decision is right. The decisions that could be wrong in ways only
building will show are the walker's soundness (D266-D268), the cost of validating per response (D280,
D281) and the usability of the inheritance rule (D264). Each has a task that tests or measures it.
