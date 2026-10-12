# Feature Specification: Visual Studio, Monaco and runtime graph

**Feature Branch:** `007-workflow-studio`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Priority:** M4. **Dependencies:** 001, 002, 003, 004.

## User Scenarios & Testing (mandatory)

### User Story 1 — Create a workflow without programming (P1)

**As** a user of this feature, **I want** to create a workflow without programming, **so that** I can offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Why P1:** independent, verifiable value requirement; supports the M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** an author in the Studio; **When** they drag activity/condition/parallel/saga and configure parameters; **Then** the validator displays errors and saves the IR draft.

### User Story 2 — Edit code in a node (P1)

**As** a user of this feature, **I want** to edit code in a node, **so that** I can offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Why P1:** independent, verifiable value requirement; supports the M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** a selected Script node; **When** the user edits TS/Python/C# and tests; **Then** Monaco shows the result in a sandbox and the version is only published after validation.

### User Story 3 — Analyze a real execution (P1)

**As** a user of this feature, **I want** to analyze a real execution, **so that** I can offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Why P1:** independent, verifiable value requirement; supports the M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** a Run with retries and an untraversed branch; **When** the operator opens the graph; **Then** attempts, duration, states, compensations and timeline reflect the journal.

### User Story 4 — Investigate and act on a failure (P1)

**As** a user of this feature, **I want** to investigate and act on a failure, **so that** I can offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Why P1:** independent, verifiable value requirement; supports the M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** a failed run and an authorized operator; **When** they open the trace, adjust the operational decision and perform a retry; **Then** the action is audited and does not overwrite the history.

## Functional Requirements

- **FR-007-001**: The system MUST implement Workflow Canvas with React Flow and typed node registry (Trigger, Step, Condition, Parallel, Join, Timer, Wait, Saga, Webhook, Child).
- **FR-007-002**: The system MUST implement Monaco editor, schema-aware input/output editors, autocomplete and lint.
- **FR-007-003**: The system MUST compile/verify the graph into Workflow IR with versioned node IDs, with no illegal cycles or broken references.
- **FR-007-004**: The system MUST implement Draft/Published versions, diff, and a warning for contract-breaking changes, with no in-place modification of the published version.
- **FR-007-005**: The system MUST project the journal into an execution graph with attempts, retries, compensations, trace links, redacted payload.
  The redacted-payload implementation is the authorized, audited structural inspection in
  [ADR-0028](../../docs/adr/ADR-0028-authorized-structural-payload-inspection.md): actual persisted
  input or journal-selected output, with every scalar value and object key hidden, bounded
  structure/type/count/size metadata, and explicit unavailable/external-content states. It requires
  `run:payload:inspect` in addition to run read access. No schema annotation authorizes value
  disclosure. T007-048 is implemented and verified; its PostgreSQL/API, application/projector,
  identity, Studio and real-browser evidence is recorded in `tasks.md` and `quickstart.md`.
  Customer classification and raw-value disclosure remain pending under G42/G39.
- **FR-007-006**: The system MUST offer SSE/WebSocket incremental updates and polling fallback, avoiding full DAG reload.
- **FR-007-007**: The system MUST enforce operational actions via RBAC and confirmation explaining the consequences of retry/compensation.
- **FR-007-008**: The system MUST implement a WCAG AA accessibility target and an alternative table/list view.
- **FR-007-009**: The Studio MUST follow the visual direction recorded in [`design/README.md`](./design/README.md) on all of its screens: the
  tokens (colour, type, radius, spacing) as the only source of values for every theme, the application shell (sidebar with the
  namespace and the navigation, a top bar), the Runs screen as a list beside a detail pane with the graph, and the language of the
  graph's cards and edges. Where the direction and FR-007-008 disagree, FR-007-008 wins and the deviation is listed, with its
  contrast ratio, in `design/README.md`. The Studio's labels stay in English. It has a light and a dark theme, both derived from
  the same tokens, and **follows the system's colour scheme by default** (D157; `prefers-color-scheme`, with forced colours as before).
- **FR-007-010**: An element of the design for which the platform has no data or no feature MUST NOT be drawn with sample values or as a
  link to nothing: it is omitted, or shown unavailable with the reason (queue, worker and trigger kind of a run, counts, workers and
  backlog of the platform, pages of other features, the payload tab). The table of such elements is in `design/README.md`.

Visual defect acceptance (2026-10-12, FR-007-008/009): initial workflow and run graphs must show
readable cards rather than shrinking the entire graph to microscopic text. Explicit fit/zoom and
the alternative list/table remain available. At desktop widths the run graph controls must fit in
the visible canvas; workflow palettes and inspectors must not squeeze the canvas or overflow their
panels. Run identity/actions remain legible, with secondary metadata available through an accessible
disclosure. Verify rendered geometry at 320, 768, 1024, 1280 and 1470px and selected controls in light,
dark and forced-colour modes; presence-only and page-width checks are insufficient.

## Edge Cases

- graph with 50k nodes.
- run started on an unpublished version.
- signal between snapshots.
- worker without OTel trace.
- two simultaneous draft edits.
- circular validation error.
- redacted payload in history.

## Mandatory cross-cutting requirements

- **NFR-007-SEC:** tenant isolation, RBAC, least privilege, secrets never in logs or in the journal.
- **NFR-007-DUR:** after commit, loss of a worker/broker must not lose a transition; use lease/fencing where applicable.
- **NFR-007-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-007-COMP:** versioned protocol and schemas; do not break in-flight runs.
- **NFR-007-PERF:** 1k-node DAG usable in the reference browser; virtualized lists for 10k+; FPS targets per UX benchmark.

## Success Criteria (measurable)

- **SC-007-001:** E2E create→validate→publish→trigger→fail→compensate→inspect journal/traces.
- **SC-007-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-007-003:** no violation of the Constitution's gates and no silent loss of persisted state.
- **SC-007-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.
- **SC-007-005:** (FR-007-009, FR-007-010) no stylesheet of the Studio holds a literal colour outside the token definitions; every text,
  control-border and focus pair of every theme (light, dark, forced colours) meets its WCAG threshold, checked by a test; axe-core finds
  no serious or critical violation on any page in any of the three; the US01..US04 journeys pass unchanged in what they assert; and
  the Runs screen was compared with `design/runs-prototype.jpg` by a person (a review, reported as one: no tool judges intent).

## Key Entities

Check `docs/architecture/overview.md`, `docs/architecture/execution-semantics.md` and `specs/001-platform-foundation/data-model.md` for the persisted entity; feature abstractions are described in the local `data-model.md`.

## Assumptions

- Workflow IRs and versions are immutable after publication.
- External integrations are at-least-once, subject to idempotency or reconciliation.
- V1 single-region HA; multi-region active-active requirements are out of scope.
- The design is a snapshot of a project its owner may keep editing: the owner's export of 2026-10-10, whose Runs prototype is kept in `design/source/`. It is exact for the dark theme of the Runs screen; `design/README.md` says what it does not cover (G160).

## Out of Scope

- Reimplementing Temporal/Restate; implementing the entire connector market; automatic migration of arbitrary code; exactly-once physical execution of external side effects.
- The pages the design's navigation lists that belong to other features (schedules, workers, task queues, secrets, settings) and a Portuguese interface (an internationalisation feature).

## Open Design Questions

- Define capacity/quota values per tier, after benchmark and security testing.
- Confirm the payload classification/retention scheme per customer and regulator.
  This remains open for value disclosure and retention policy; ADR-0028's `structure-only-v1`
  inspection makes neither decision and reveals no scalar value or object key.
- Adjust SDK ergonomics based on interviews and proof-of-concept.
- The design's owner: the light theme and the screens beyond Runs (editor, scripts, sign-in, states), the logo, where Scripts sits in the navigation, and whether *Send signal* and the per-step *Timeline* bars are part of this feature (G160–G163, G167, G168).

## Traceability

Requirements `FR-007-###` and criteria `SC-007-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation.

Implementation status (2026-10-11): FR-007-009/010 visual changes and technical checks are implemented: 1 033 Vitest tests, 18 executed browser journeys (light/dark/forced colours, 320px reflow and system-font fallback), and both three-round benchmarks passed. Review captures are linked in `design/README.md`; detailed evidence is in `quickstart.md`. The owner approved the proposal on 2026-10-11; T007-044 is complete after the real-sandbox CI journey; US02 was excluded from this local browser run because Docker's sandbox runtime was unavailable. The Studio CI job passed 1 034 unit tests and all 19 browser journeys, including US02; see `quickstart.md` for the exact job.

Owner review recorded on 2026-10-11: Marcelo explicitly approved the visual proposal ("Aprovo a proposta visual")
after opening the local Studio. This approval covers Runs, the light theme, editor, Scripts and sign-in;
automated CI evidence for this branch is recorded separately.
