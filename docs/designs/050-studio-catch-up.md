# The Studio catch-up: every executable node kind, the approvals inbox, the audit log

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

The engine has moved faster than the Studio. On `develop` at 2026-11-24 the server publishes and runs `switch` and
`approval` nodes, parses loops, collection operators and lake writes behind runtime switches, lists and decides
approvals, keeps an audit trail, records activations, publication requests and compensation plans, and builds Go and
Java scripts. The Studio shows none of these, and it offers two node kinds the server refuses.

From release 0.3.0 the Studio covers what the server executes and exposes:

- **Every node kind the server can run is editable and visible in a run**: `switch`, `approval`, `forEach` with the
  collection operators of design 042, `while`, `repeatUntil`, `return` inside a body, `continueAsNew`, `database` and
  `icebergWrite`, the workflow's declared state with its Queries and Updates, and the limits and chain policy. The
  concurrency and rate-limit fields of feature 022 are built on their own branch and join the same registry. A kind
  the server does not accept is never offered, a combination the server does not publish yet is shown as refused at
  the node while the author edits, by the server's own validation, and a document that holds a refused kind opens
  without losing it
  ([Architecture decision 0143](../adr/ADR-0143-the-studio-draws-node-kinds-from-one-registry-checked-against-the-schema-and-enabled-by-the-servers-capabilities.md)).
- **One inbox** lists what waits for the person signed in: the run approvals they may decide now and the publication
  requests they may approve, ordered by deadline. The server answers "may I decide this" with the same rule that
  decides, and never says who else may
  ([Architecture decision 0144](../adr/ADR-0144-the-approvals-inbox-asks-the-server-for-the-callers-own-eligibility-and-never-returns-a-policy.md)).
- **The audit log** of the tenant is a page, with the filters the server already supports and a bounded download.
- **The operator surfaces a Temporal or Windmill user expects and the server already has**: declared Queries and
  Updates from the run view, loop iterations and the chosen switch case in the run graph, the run's family (parent,
  children, continuation chain), version history with activation, rollback, promotion and the compensation plan,
  namespace settings (publication policy, quotas, worker bindings, reveal grants, secret access lists), event delivery
  health with redrive, a command palette, and Go and Java marked as built.

All of it is built on the Astryx design system of
[Architecture decision 0101](https://github.com/ortisun/sagat-octopus-orchestrator/blob/feature/007-astryx-design-system/docs/adr/ADR-0101-the-studio-builds-its-interface-on-the-astryx-design-system.md)
(pull request #267), which this design accepts under the owner's delegation (Technical Plan 1). The server gains three
small things: a `capabilities` list on `GET /version`, a namespace-wide approvals list, and a namespace-wide list of
publication requests. Nothing else changes in the engine, the IR, the worker protocol or the SDKs.

### Terms

- **Registry.** The Studio's list of node kinds (`frontend/apps/studio/src/model/registry.ts`): for each kind, its
  label, its fields, its handles (the links to other nodes) and, from this design, the capability it needs.
- **Capability.** A name the server advertises in `GET /version` when the replica that answers would accept the
  matching publication or route (Architecture decision 0143). Advice for a client; the server's validation decides.
- **Body.** A named graph of `bodies` in the workflow document, run once per iteration by `forEach`, `while` and
  `repeatUntil`; it ends at exactly one `return` node.
- **Scoped instance.** One execution of a body node inside one iteration, named in the runtime graph by an
  `instancePath` (the loop node and the iteration index of each enclosing scope) and an id such as `items[2]/charge`.
- **Legacy kind.** `foreach` and `humanApproval`: shapes of the first schema that the server refuses at publication
  for good ("legacy collection placeholder has no executable definition" and "legacy approval placeholder has no
  executable definition", `crates/infra-ir/src/convert.rs`).
- **Publishable subset.** The shapes publication accepts in 0.3.0, narrower than the schema (Technical Plan 4). The
  codec decides them; feature 014's remaining tasks and design 039 widen them during the release.
- **Inbox.** The Studio page "Waiting for me" of this design.

## The Problem

The Studio was built with feature 007 and extended by each later feature for its own screen. What the server does
today and the Studio does not show, read from the code on `develop` at 2026-11-24:

| Area | The server | The Studio |
|---|---|---|
| Node kinds | 19 root kinds and 14 body kinds in `workflow.schema.json`; `approval` and `switch` publish and run in the basic graph; `forEach`, `while`, `repeatUntil`, `return` and `continueAsNew` publish in fixed shapes only (a collection chain or a loop chain ending at `end`, narrow bodies, no declared state with bodies; Technical Plan 4); `icebergWrite` when `lake.enabled`; collection operators when the deployment switch is on | 12 entries in `model/registry.ts`, two of them the refused legacy kinds (`foreach`, `humanApproval`), marked "does not run yet"; no body editing; no declared state |
| Run graph | Scoped instances with `instancePath`, `waitingOn` (`approval`, `loop`, `collection`), `switchSelection`, `childRunId`, chain links | Waiting labels only; iterations are not grouped; the chosen case is not shown; an approval cannot be decided |
| Declared operations | `POST /runs/{runId}/queries/{name}`, `.../updates/{name}` and the chain-addressed forms (feature 014, Architecture decision 0031) | No way to call them |
| Approvals | `GET /runs/{runId}/approvals`, `POST /approvals/{approvalId}/decisions` | Nothing; no list across runs exists on the server either |
| Audit | `GET /audit` with action, resource, actor and time filters (feature 008) | No page |
| Versions | Activation history, activate and roll back with a reason, promotion, publication requests (request, approve, reject, withdraw, publish), compensation plans (feature 011, feature 003) | The publish dialog and `VersionsPanel.tsx` (the version list, a read-only view, a diff); no activation, rollback, promotion, requests or compensation plan |
| Namespace settings | Publication policy, quotas and activity quotas, worker bindings, payload reveal grants, secret access lists, egress policy | Egress policy and secrets only |
| Event plumbing | Event endpoints, webhook and broker subscriptions with deliveries and redrive, connector dead letters with redrive (feature 004) | Connectors only |
| Scripts | `Language::Go.is_built()` and `Language::Java.is_built()` hold (`crates/domain/src/scripts.rs`; Architecture decision 0024) | Go and Java disabled, "not built by this platform yet" (`features/scripts/languages.ts`) |

The assessment of 2026-11-11 found the same gap ("Studio also trails the engine it fronts") and found no specification
that covers it. Schedules and batch operations, which it also listed as missing, have since been built by feature 018.

Measured against the two products the owner wants to surpass:

- **Temporal's Web UI** lists and filters workflows, shows the event history (compact, full, JSON), the pending
  activities, the call stack and the relationships of a run, sends signals, queries and updates, terminates, cancels
  and resets, and runs batch operations and schedules. The Studio already has the history, timeline, reset, signals,
  operations, schedules and batches; it lacks queries and updates, the relationships, and a download of the history.
- **Windmill's interface** edits flows with branches and loops in a graph, has an approval step whose page a person
  opens from a link, an audit log page, folders and groups, a command palette, worker groups, and per-workspace
  settings. The Studio lacks loops and approvals in the editor, the audit page, the palette and the settings.

Without this design a 0.3.0 user can build in an SDK what they cannot see or edit in the Studio, and must call the
REST API to approve a step.

## Requirements

### Functional

- **Requirement 050-001** The Studio MUST hold its node kinds in one registry with one entry per `type` constant of
  the members of `$defs/node` and `$defs/bodyNode` of the workflow schema, each with its scopes (root, body or both),
  its required fields per scope and the capability it needs or none, and a unit test reading the schema from
  `specs/001-platform-foundation/contracts/` MUST fail when the schema and the registry differ (Architecture decision
  0143, decision 1). *Acceptance:* `registry.test.ts` passes on the schema of `develop` and fails on a schema with an
  added kind or a changed required field.
- **Requirement 050-002** `GET /version` MUST return `capabilities`, a list of deployment-wide names (never per
  tenant) of the known list of Technical Plan 3, each present exactly when the answering replica would accept the
  matching publication or route; names that follow an operator's activation in the instance registry MUST appear within
  one heartbeat of it. The field is a list of strings with a pattern; a reader MUST ignore a name it does not know, and
  an absent field means a server older than 0.3.0. *Acceptance:* the contract test
  `capabilities_follow_the_switches_a_server_was_started_with` (each static switch on and off),
  `a_registry_activation_appears_within_one_heartbeat`, and `a_reader_ignores_an_unknown_capability`.
- **Requirement 050-003** The palette MUST offer only the kinds whose capability is present, inside a body only the
  kinds of that body's scope, and never a legacy kind; each entry MUST say in words where it publishes in 0.3.0
  (Technical Plan 4). A combination the codec refuses MUST be shown at its node, with the codec's reason, from the
  editor's live validation, never from a rule written again in the Studio. *Acceptance:* the Studio unit test with a
  capability list without `workflow.lake-writes` shows no `icebergWrite`; the journey that places an `approval` inside a
  collection body shows the server's refusal at that node within the validation delay.
- **Requirement 050-004** A draft that holds a node the palette would not offer (a legacy kind, or a kind whose
  capability is absent) MUST open with that node drawn, read-only, with the reason in words and the server's
  problem; saving the draft MUST keep the node equal under `canonical()` (`model/ir.ts`) and in its place among the
  nodes. *Acceptance:* a draft with a `humanApproval` node opens, saves and reloads equal under `canonical()`, the node
  in its place; its problem list shows the server's message.
- **Requirement 050-005** A legacy node MAY be replaced through a dialog that fills the fields that map exactly
  (Technical Plan 4) and requires the author to fill every other required field before the replacement is made; the
  replacement MUST be one undoable edit. *Acceptance:* the dialog test replaces a `foreach` with a `forEach` whose
  `items`, `body`, `maxItems` and `maxConcurrency` come from the old node and whose `result` the author typed; undo
  restores the `foreach`.
- **Requirement 050-006** The editor MUST edit, with its fields and handles, `switch` (up to 32 ordered cases and a
  default), `approval` (policy subjects and roles, initiator exclusion, timeout and the approve, reject, expire and
  error routes), `forEach` (items, up to eight collection-operator stages, body, result, the item and concurrency
  ceilings), `while` and `repeatUntil` (condition, body, initial, carry, iteration ceiling, delay, timeout),
  `continueAsNew` (input and state), `icebergWrite` (the fields of `$defs/icebergWrite`), the declared state
  (schema, initial value, Queries and Updates with their argument and result schemas), `inputSchema`, `limits` and
  `chainPolicy`. A field the server would refuse MUST show the server's problem at that field after `validate`.
  *Acceptance:* one component test per kind builds the node from the palette, fills it, validates it against the
  test server and, in a shape of the publishable subset (Technical Plan 4), publishes it.
- **Requirement 050-007** A body MUST be edited on a canvas of its own, opened from the loop node that names it, with
  a breadcrumb back to the workflow; the body canvas MUST require exactly one `return` and offer only the kinds of
  `$defs/bodyNode`, each saying where it publishes in 0.3.0 (Technical Plan 4); a body named by two loops MUST say so
  before it is changed. *Acceptance:* the editor test opens the body of a
  `forEach`, adds a step, and cannot save a body without its `return` without the problem shown.
- **Requirement 050-008** The run view MUST group scoped instances under their loop or collection node (counts by
  status, a virtualized list of up to 10,000 iterations, the body graph of one chosen iteration), MUST show the case a
  `switch` chose from `switchSelection`, and MUST show an `UNKNOWN` selection as such. *Acceptance:* the run view test
  with a recorded graph of a `forEach` of 500 items shows the counts, opens iteration 312 and shows its steps.
- **Requirement 050-009** The run view MUST list the run's approval requests with their status and deadline, and let a
  person decide one when `youMayDecide` is true, with a decision key made per dialog and reused on retry, an optional
  reason code, and the receipt shown; an ineligible person MUST see that they cannot decide, without the policy. The
  flag comes from the approvals list filtered by `runId` (Requirement 050-010). *Acceptance:* the journey
  `a_person_approves_a_step_from_the_run_view_and_the_run_moves_on`.
- **Requirement 050-010** The server MUST list the pending approval requests of a namespace
  (`GET /namespaces/{namespaceId}/approvals`) under Architecture decision 0144: ordered by deadline, optionally
  filtered by `workflowId` or `runId` (the latter served by the run's index, outside the deadline scan), each item with
  `youMayDecide` computed by `ApprovalRequest::may_decide` on the caller's current principal, the filter
  `decidable=true`, nothing of the policy, a bounded scan per page. An approval pending on a run that has ended is not
  decidable. Settled approvals stay on the per-run list and in the audit log. *Acceptance:* the property test
  `may_decide_equals_decide` (domain), the contract test that no field names a policy member, the cross-tenant battery
  entry, and the integration test that a page stops at the scan bound and continues from its cursor.
- **Requirement 050-011** The server MUST list the `PENDING` publication requests of a namespace
  (`GET /namespaces/{namespaceId}/publication-requests`), each with `youMayDecide` computed by
  `PublicationRequest::may_decide` with the three clauses of Architecture decision 0018 decision 4 (not the requester,
  not the last editor of the draft, not a person who already decided it) and `publication:approve` held.
  *Acceptance:* integration tests for the requester, the last editor, an approver who already approved a request that
  needs two approvals, another approver and a viewer.
- **Requirement 050-012** The Studio MUST have an inbox ("Waiting for me") that lists, for the chosen namespace, the
  approvals and the publication requests the person may decide, most urgent first, and lets them decide from there;
  the page MUST refresh while open and show when it last did. *Acceptance:* the journey
  `the_inbox_shows_what_waits_for_me_and_nothing_else`.
- **Requirement 050-013** The Studio MUST have an audit log page for holders of `audit:read`: the filters of
  `GET /audit`, an action picker from the build's closed list of audit actions, a record view with a link to the
  resource when its type has a page, and a download of the filtered records as JSON lines, at most 10,000 records,
  saying when it stopped at that bound. The action picker suggests the known actions and accepts any text, because the
  Studio may be served from another build (Architecture decision 0016, `SAGAT_STUDIO_DIR`). *Acceptance:* the
  component tests of the page; the test that the known actions of the OpenAPI `action` parameter equal `AuditAction`
  of `crates/domain/src/audit.rs`.
- **Requirement 050-014** The run view MUST let a person call the version's declared Queries and Updates (run- and
  chain-addressed) with JSON arguments (the server's `422` shown at its pointer; no schema validator in the browser,
  Architecture decision 0016 decision 1), an Update key made per dialog and reused on retry, an optional `expectedStateRevision`, and the result shown as Architecture decision 0031
  discloses it. *Acceptance:* the journey `a_person_queries_then_updates_a_running_order`.
- **Requirement 050-015** The run view MUST show the run's family: its parent, its children with their status (from
  the runtime graph's `childRunId`), and its continuation chain (`GET /namespaces/{namespaceId}/chains/{chainId}/runs`).
  *Acceptance:* the run view test with a parent, two children and a chain of three segments.
- **Requirement 050-016** A workflow MUST have a versions page, grown from today's `VersionsPanel.tsx`: the versions, the activation history, activate and
  roll back with a reason and the version the person saw, promote to another namespace, the publication requests of
  the workflow with request, approve, reject, withdraw and publish, and the compensation plan of a version.
  *Acceptance:* the journey `a_person_rolls_back_a_version_and_the_history_says_who_and_why`.
- **Requirement 050-017** A namespace MUST have a settings page with its publication policy, quotas and activity
  quotas, worker bindings, payload reveal grants and, on each secret, its access list; each control the person cannot
  use is shown unavailable with the permission it needs. *Acceptance:* component tests per section with each role.
- **Requirement 050-018** The Studio MUST show the delivery health of feature 004's event plumbing: event endpoints,
  webhook and broker subscriptions with their deliveries, connector dead letters, and redrive of one delivery, one
  dead letter or a subscription's failures, each redrive confirmed and its receipt shown. *Acceptance:* component
  tests; the journey `a_dead_letter_is_redriven_from_the_studio`.
- **Requirement 050-019** The script editor MUST offer every language `Language::is_built` accepts as built; the
  list of built languages MUST come from the capability list (`scripts.language.<id>`, Technical Plan 3), not from a
  constant. *Acceptance:* the script editor test with Go and Java built; a test that a language the server does not
  advertise is shown disabled.
- **Requirement 050-020** The Studio MUST have a command palette (Ctrl or Command and K) that finds workflows, scripts,
  schedules and connections by name, a run by its identifier or business identifier, and the pages of the Studio,
  using only list routes the person may call. *Acceptance:* component tests; the palette never calls a route the
  person lacks the permission for.
- **Requirement 050-021** The run view MUST offer a download of the run's journal as JSON lines, read through
  `GET /runs/{runId}/events` page by page at most two pages a second, carrying only what that route returns, kept as
  raw page text (never parsed into objects) and bounded at 50,000 events or 32 MiB, saying when it stopped at the
  bound. *Acceptance:* the test that the download equals the concatenated pages; the bound and pace tests.
- **Requirement 050-022** Every new screen MUST be built from the Studio's interface module on Astryx (Architecture
  decision 0101): no raw control outside `src/ui/`, no literal colour, light, dark and forced colours, keyboard
  operation, a table or list alternative to every graph, state by text and shape as well as colour, and no sample
  value where there is no data (Requirement 007-008, Requirement 007-010). *Acceptance:* the ESLint boundary rule, the
  tokens test and axe in the journeys.

### Non-functional

- **Non-functional requirement 050-001 (security)** No new screen shows a value a route withheld, puts a token, an
  argument or a payload in a URL, a log or browser storage, or calls a route the person's permissions exclude without
  showing the refusal. The two new server routes are in the cross-tenant battery and answer another tenant's
  namespace as missing.
- **Non-functional requirement 050-002 (performance)** The inbox and the audit page answer a page in the time the
  benchmark records on a declared environment, with 10,000 pending approvals in one namespace; the run view renders a
  run with 10,000 iterations without blocking input for more than the frame budget the performance journey records.
  No figure is promised.
- **Non-functional requirement 050-003 (compatibility)** The REST additions are additive; an older Studio and older
  clients behave as before; no IR, worker-protocol, AsyncAPI or SDK change.
- **Non-functional requirement 050-004 (observability)** The two new routes have spans and closed-label metrics; a
  decision from the inbox is the existing decision operation with its existing audit.

### Success criteria

- **Success criterion 050-001** A person builds, publishes and runs, without leaving the Studio, three workflows in
  shapes that publish in 0.3.0: a collection workflow (a `forEach` over 500 items with a `filter` stage, a body of an
  activity and a `return`, then `end`), a loop workflow (a `while` over an activity body, then `end`), and a workflow
  whose `switch` routes large amounts to an `approval`; approves the step from the inbox; and follows the 500
  iterations to the end (the journey of Task 050-13).
- **Success criterion 050-002** The registry test, the capability contract test and the inbox property test pass,
  and the schema drift test fails on a schema with an added kind.
- **Success criterion 050-003** axe reports no serious or critical violation on every new page in light, dark and
  forced colours, and every new page reflows at 320 pixels.

### User stories

- **User story 050-01** As an author, I draw every kind of node the platform runs, including loops and approvals,
  and I never lose a node the platform no longer accepts (CUJ 1, CUJ 2).
- **User story 050-02** As an approver, I find what waits for me in one place and decide it (CUJ 3).
- **User story 050-03** As an operator, I follow a run with loops, call its Queries and Updates, and see its family
  (CUJ 4).
- **User story 050-04** As an administrator, I read the audit log and change a namespace's settings (CUJ 5).
- **User story 050-05** As a release manager, I roll back a version, approve a publication and promote it (CUJ 6).

## Goals

- The Studio offers every node kind the server runs, and nothing it refuses.
- One place for what waits for a person, decided by the server's own rule.
- The audit trail readable by the people allowed to read it, without a database client.
- Every operator action the server exposes is reachable from the Studio with the same authorization and audit.
- A registry and a capability list that make the next drift a failing test.

## Non-Goals

- Screens that belong to another 0.3.0 feature: generated forms, the decision page reached from a link, notification
  channels and Workflow Streams (026); search attributes, the list query language and saved views (020); resources,
  variables and the new triggers (021); the step cache, early return, failure handlers, standalone steps and draft
  test mode (022); worker deployments and task-queue insights (019); step logs (041); internal apps (027); Git sync and
  templates (028); roles, groups and folders (029); code runs (044, Task 044-23); service endpoints (030); the codec
  server (031). Each feature builds its screens on this design's registry, capability list and interface rules.
- A namespace column in the audit trail. `audit_records` has no namespace (migration `20261008000000_identity`); the
  audit page is tenant-wide, as `GET /audit` is (Gap 050-002).
- Editing a workflow written as code (design 044 draws a code run's observed graph; it is not editable).
- Converting a legacy node without the author's action, or publishing one.
- A mobile layout beyond reflow at 320 pixels.

## Open Questions

None open. The questions of this design were decided by the agent the coordinating session briefed, under Owner
decision 2026-11-24-4, each weighed over its options and recorded with its reasoning in the register below as
"delegate (OD-2026-11-24-4)".

### Decisions

- **Decision 050-001 — Accept Architecture decision 0101 (Astryx).** Options: accept as proposed; accept with
  conditions; reject and keep the hand-written layer. Chosen: accept with two conditions. The owner gave the direction
  (2026-11-21); the record holds a pre-1.0 library behind one directory (`src/ui`), with an exact pin, a theme that
  carries the approved visual direction and a contrast test; its continuous integration was green on pull request #267.
  Every screen of this design and of features 026 to 029 would otherwise be written twice. The conditions answer the two
  costs the record states and has not measured: (1) the performance journey runs on the merged branch before release
  0.3.0 and its result is recorded in `specs/007-workflow-studio/quickstart.md`; (2) a bundle budget test is added (the
  entry script and the compressed total of the production build may not grow past the figures measured on the merged
  branch plus 10% without a recorded reason), so the next growth is a decision and not an accident (Task 050-1).
  Recorded in `docs/owner-decisions.md` as OD-2026-11-24-700.
- **Decision 050-002 — One capability list for every feature.** Options: a field per feature (029's `accessControl`,
  027's `apps`, 028's `virtual-clock`); one list of names. Chosen: one list (Architecture decision 0143), because a
  client learns one shape and a feature adds a name, not a field. Features 027, 028 and 029 are corrected to add names.
- **Decision 050-003 — A refused kind is kept and shown, never dropped.** Options: drop it on load; convert it on load;
  keep it read-only and offer a guided replacement. Chosen: keep and offer. Dropping loses work; an automatic
  conversion guesses `result` for a `forEach` and the policy for an `approval`, which have no exact source.
- **Decision 050-004 — The inbox decides eligibility on the server.** See Architecture decision 0144.
- **Decision 050-005 — Publication requests join the inbox.** Options: approvals only; approvals and publication
  requests. Chosen: both, because both are "a person must decide before the work goes on", and the second needs only
  a list route over rows that exist.
- **Decision 050-006 — The known audit actions are listed in the OpenAPI document.** Options: a route that lists the
  actions; free text; a separate contract file bundled with the Studio; `x-known-values` on the `action` parameter of
  `listAuditRecords`. Chosen: the OpenAPI list, held to `AuditAction::ALL` by a test, so `npm run check:api` keeps the
  Studio current, with free text still accepted because the Studio may be served from another build (Architecture
  decision 0016, `SAGAT_STUDIO_DIR`); settles Nit 15 of the architecture review of 2026-11-25.
- **Decision 050-007 — The built languages come from the server.** Options: change the constant; a field on the
  scripts routes; a capability name per built language. Chosen: the capability names (`scripts.language.go` and the
  rest, from `Language::is_built`), so feature 023's new languages appear without a Studio change and without a second
  advertisement.
- **Decision 050-008 — The journal download is client-side and bounded.** Options: a new export route; the existing
  `GET /runs/{runId}/events` read page by page. Chosen: the existing route, because it already applies the
  disclosure rules; design 044's `GET /runs/{runId}/history` (for replay) is a different document and is that
  design's. The download is bounded and paced (Requirement 050-021).
- **Decision 050-009 — The publishable subset is the codec's, shown by live validation.** Options: a capability per
  primitive (the first draft; no such switch exists, Blocker 1 of the architecture review of 2026-11-25); placement
  rules written again in the Studio; a placement table served by the server; the schema's scopes in the palette, a note
  per kind, and the editor's existing live validation marking a refused combination at its node. Chosen: the last. The
  subset is shape rules in `crates/infra-ir/src/convert.rs` (a chain ending at `end`, one to eight body steps, no bodies
  with declared state), which a kind-by-scope table cannot express and a TypeScript copy would let drift (Architecture
  decision 0016 rejected a second rule set in the browser); it widens during the release (feature 014, design 039), and
  only the codec and its documents then change.
- **Decision 050-010 — The namespace approvals list holds pending approvals only.** Options: every status ordered by
  deadline (an unbounded sort of settled rows); a second index by `settled_at`; pending only. Chosen: pending only, the
  inbox's purpose; settled approvals stay on the per-run list and in the audit log (Major 6 of the same review).

### Gaps

- **Gap 050-001** The capability list is advice: during a rolling upgrade two replicas may answer differently. The
  server's validation stays the authority (Architecture decision 0143, decision 5).
- **Gap 050-002** The audit trail has no namespace column, so the audit page cannot filter by namespace.
- **Gap 050-003** The approval form, summary and notification fields of feature 026 are drawn by 026's Studio tasks on
  the inspector this design builds; until then the approval inspector shows only the fields of feature 014.
- **Gap 050-004** The subject picker of the approval policy is free text until feature 013's member list exists; then
  it lists the tenant's members (design 013, Requirement 013-020).

## Owner decisions needed

None. The scope is Owner decision 2026-11-24-1 ("the Studio catch-up"); the questions of this design, the acceptance
of Architecture decisions 0101, 0143 and 0144 and the settlement of review findings are delegated by Owner decision
2026-11-24-4. Decision 050-001 is recorded as OD-2026-11-24-700.

## Critical User Journeys (CUJs)

### CUJ 1: An author builds a collection and an approval (User story 050-01)

*   **Given**: a namespace on a server whose `GET /version` lists `workflow.collection-operators`, and an author with
    `definition:write` and `definition:publish`.
*   **When**: she builds `refund-lines`: a `forEach` over `input.lines` with a `filter` stage, whose body has a step
    and a `return`, then `end`; then `refund-approval`: a `switch` with two cases and a default, an `approval` on the
    large-amount case for the role `operator`; she tries to drop an `approval` into the collection body.
*   **Then**: each node is edited through its own fields; the body canvas refuses to save without its `return`; the
    `approval` in the body is marked at the node with the server's reason ("approvals inside collection and loop
    bodies wait for scoped compensation") within the validation delay; once she removes it, both workflows validate
    and publish.

### CUJ 2: An old draft with a legacy node (User story 050-01)

*   **Given**: a draft written with the first Studio that holds a `humanApproval` node.
*   **When**: the author opens it.
*   **Then**: the node is drawn, read-only, with "The platform no longer runs this kind of node: replace it with an
    Approval"; the draft saves unchanged; the replacement dialog fills the timeout and the routes and asks for the
    policy; after the replacement the draft validates.

### CUJ 3: An approver works her inbox (User story 050-02)

*   **Given**: three pending approvals in the namespace: two whose policies name the role `operator`, one that names
    only another subject; she is an operator and started one of the first two, whose policy excludes the initiator.
*   **When**: she opens "Waiting for me".
*   **Then**: one approval is listed as hers to decide, with its workflow, run, step, deadline and time left; the
    others are not listed under "decidable", and nothing on the page says who may decide them; she approves, the
    receipt shows, the item leaves the list, and the run moves to its approve route.

### CUJ 4: An operator follows a large collection (User story 050-03)

*   **Given**: a run of a collection workflow over 2,000 items, 12 of which failed, and a run of an `order` workflow
    (no bodies) whose declared state has the Query `progress`.
*   **When**: the operator opens the first run, then the second.
*   **Then**: the collection node shows 1,988 succeeded and 12 failed; the iteration list filters to the failed ones;
    one opens with its body graph and its failure; on the second run the Queries panel calls `progress` and shows its
    declared result.

### CUJ 5: An administrator investigates a change (User story 050-04)

*   **Given**: a tenant administrator and an unexpected change of a namespace's publication policy.
*   **When**: she opens the audit log, picks the action `publication.policy_changed` and the last day.
*   **Then**: the record shows who changed it, when, the reason and the trace; the link opens the namespace's settings,
    where the policy shows its current value and source.

### CUJ 6: A release manager rolls back and promotes (User story 050-05)

*   **Given**: version 7 of `orders` active in `staging`, version 6 before it, a publication request for version 8.
*   **When**: he opens the versions page, rolls back to 6 with a reason, approves the request for 8 written by another
    person, and promotes 6 to `production`.
*   **Then**: the activation history lists both moves with who and why; the promotion is refused, with the names, if a
    dependency is missing in `production`.

## Technical Plan (Architecture)

### 1. The base: Astryx, and the order of the Studio branches

Design 050 builds on pull request #267 (feature 007, Astryx), which replaces the hand-written controls with Astryx
components behind `frontend/apps/studio/src/ui/`. Architecture decision 0101 is accepted with the two conditions of
Decision 050-001. Every new screen imports only `src/ui`; a control Astryx lacks (the body breadcrumb, the iteration
strip) is a new file in `src/ui/` with its own test, never an edit of an existing primitive in the same pull request as
a screen, so that parallel branches add files instead of changing the same ones.

Order of the open Studio work, so that no two branches rewrite the same folder at once:

1. Pull request #267 (Astryx) merges first.
2. `feature/022-studio-flow-control` rebases on it and merges (concurrency and rate-limit fields, `FlowControlFields`
   in `features/workflows`, run gates in `features/runs`). Its registry entries use `runs: boolean` today; they move to
   `capability` and `scope` in design 050's editor slice, by whichever branch merges second.
3. Feature 041's Studio task (the Logs tab in `RunDetailPage.tsx` and `NodePanel.tsx`) merges before design 050's run
   slice, which rebases on it.
4. Design 050's editor slice (Technical Plan 4: `model/registry.ts`, `features/workflows/`), then its run slice
   (`features/runs/`, `model/runGraph.ts`), then its operations slices.
5. Feature 026's approval fields go on 050's approval inspector after the editor slice. Design 044's Task 044-23 (the
   run view of code runs and the open runs per digest) follows 050's run and versions slices and adds its column to
   050's versions page, which 050 owns.
6. Feature 028's extraction of the graph model into `frontend/packages/workflow-graph` starts after the editor slice
   has merged; feature 029's folder tree touches the list pages, which this design does not change.
7. Feature 029's `x-access` with `discloses` and its `visibleFolders` filter on the two new list routes are added by
   whichever of 050's server slice and 029's slice 0 merges second.

### 2. Ownership: what this design builds and what it leaves

| Surface | Built by | Note |
|---|---|---|
| Node kinds of the schema, bodies, declared state, limits | 050 | the registry is the extension point for 022, 026, 044 |
| Concurrency and rate-limit fields, gate badges in a run | 022 (its branch) | joins the registry with the capabilities `workflow.flow-control.concurrency` and `workflow.flow-control.rate` |
| Approval form, summary, notify; decision page from a link | 026 | on 050's approval inspector |
| Inbox, audit page, versions page, settings page, event health, palette | 050 | |
| Run list query, saved views | 020 | the run list keeps its filters until then |
| Step logs tab | 041 | |
| Code runs; the open runs per code digest on the versions page | 044 (Task 044-23) | drawn with the same node pictures, marked `Code-orchestrated`; a column of 050's versions page |

### 3. Capabilities

`PlatformVersion` gains `capabilities`, a list of strings matching `^[a-z]+(\.[a-z0-9-]+)+$`, at most 128. The names
are deployment-wide: `GET /version` answers the same list to every tenant. A reader MUST ignore a name it does not
know; the OpenAPI document lists the known names under `x-known-values` of the field, never as an `enum`, so a new name
is not a breaking change (`docs/engineering/architecture-principles.md`: readers tolerate what they do not know). A
response without the field comes from a server older than 0.3.0; the Studio then treats every name as absent.

The known names in 0.3.0, each with the source that turns it on:

| Name | Present when | Source | Owner |
|---|---|---|---|
| `workflow.collection-operators` | the deployment's collection-operators switch is on (design 042) | configuration | 042 |
| `workflow.lake-writes` | `lake.enabled` (feature 035) | configuration | 035 |
| `workflow.step-destinations` | step-output destinations publish (feature 035) | configuration | 035 |
| `workflow.flow-control.concurrency`, `workflow.flow-control.rate` | the operator activated `flow-control-concurrency-v1` or `flow-control-rate-v1` | instance registry | 022 |
| `workflow.reset-reapply` | the `resetReapply` capability of feature 018 | configuration | 018 |
| `apps.runtime`, `apps.embedding`, `apps.custom-components`, `apps.operation-bindings` | feature 027's switches and origins | configuration | 027 |
| `testing.virtual-clock`, `testing.history-replay` | only on the test server of feature 028 | composition root | 028 |
| `access.custom-roles`, `access.groups`, `access.external-groups` | feature 029's gates (external groups with design 013's sources) | configuration | 029, 013 |
| `access.folders.workflow`, `access.folders.script`, `access.folders.connection`, `access.folders.app` | feature 029's open-folder gate for that item kind | configuration | 029 |
| `access.restricted.workflow`, `access.restricted.run`, `access.restricted.script`, `access.restricted.connection`, `access.operation-lists` | feature 029's gate for that kind and the operator's activation of `access-narrowing-v1` | instance registry | 029 |
| `identity.federation`, `identity.scim` | design 013's switches | configuration | 013 |
| `scripts.language.typescript`, `scripts.language.python`, `scripts.language.csharp`, `scripts.language.go`, `scripts.language.java` (feature 023 adds `bash`, `powershell`, `php`, `rust`) | `Language::is_built` holds for the language in this build | build | 005, 023 |

The loops and `continueAsNew` need no name: publication accepts them in the shapes of Technical Plan 4 on every 0.3.0
server.

**Where it lives.** `crates/application/src/capabilities.rs` holds a typed `PlatformCapability` enumeration with
`as_str`, the closed source of the names; a feature adds a variant and its table row. Names from configuration, the
build and the composition root are computed once at start-up by `apps/control-plane`. Names from the instance registry
are derived from the set of activated requirements the process already re-reads on each heartbeat (Architecture
decision 0082 decision 4), through one table that maps each registry `Capability` (`*-v1`) to its name, held by a test
that every registry capability that gates a client-visible behaviour has a row. So an operator's activation shows in
`GET /version` within one heartbeat, without a restart. `crates/api-http/src/routes/identity.rs` serves the union.

### 4. The registry and the editor

`model/registry.ts` keeps one entry per `type` constant, with `capability`, its scopes (`root`, `body` or both), its
fields and handles per scope (a root `activity` and a body `activity` are different schema members with different
required fields), and a one-line note of where it publishes in 0.3.0. `registry.test.ts` reads
`specs/001-platform-foundation/contracts/workflow.schema.json` (imported by path in the test, not copied) and asserts:
every `type` constant of the members of `$defs/node` and `$defs/bodyNode` has one entry with exactly those scopes;
every required property of the member of each scope is a field or a handle of that scope; no entry names a kind
outside the schema.

New entries (fields follow the schema; a duration is the existing duration field; an expression the existing
expression field):

| Kind | Fields | Handles | Capability |
|---|---|---|---|
| `switch` | ordered cases (`id`, `when`), up to 32 | one per case, `default` | none |
| `approval` | `policy.subjects`, `policy.roles` (admin, developer, operator), `policy.excludeInitiator` (default true), `timeoutMs` | `onApprove`, `onReject`, `onExpire`, `onError` | none |
| `forEach` | `items`, `operators` (stage list: filter, map, flatMap, skip, take, distinct, window), `body` (a body name), `result`, `maxItems` (1 to 10,000), `maxConcurrency` (1 to 256), `timeoutMs` | `next`, `onError` | none; the stage list needs `workflow.collection-operators` |
| `while`, `repeatUntil` | `condition`, `body`, `initial`, `carry`, `maxIterations` (1 to 10,000), `delayMs`, `timeoutMs` | `next`, `onError` | none |
| `return` (body only) | `result` | none | none |
| `continueAsNew` | `input`, `state` | none | none |
| `icebergWrite` | the fields of `$defs/icebergWrite`, with the connection picker of the database node | `next`, `onError` | `workflow.lake-writes` |
| `database` | exists (`DatabaseFields`) | | none |

**The publishable subset of 0.3.0.** The schema allows more than publication accepts. On `develop` at 2026-11-24
(`crates/infra-ir/src/lib.rs`, `published_definition`; `crates/infra-ir/src/convert.rs`;
`docs/runbooks/workflow-primitives.md`):

| Shape | Publishes | Refused |
|---|---|---|
| Basic graph | activity, database, script, condition, parallel, timer, waitEvent, child, saga, switch, approval, end, and `icebergWrite` with lake writes on | an approval or a loop combined with a collection |
| Collection workflow | source activities, then a chain of `forEach` nodes ending at `end`; a body of one to eight activities, conditions, switches, timers and event waits with local routes and one `return` | triggers, `chainPolicy`, an approval in a body, `while`, `repeatUntil`, `continueAsNew` beside a collection |
| Loop workflow | a chain of `while` or `repeatUntil` nodes ending at `end`, recovery activities; a body that is an activity chain | an activity after a loop's success edge, triggers, an approval in a body |
| Declared state | `state` with Queries and Updates; `continueAsNew` with `chainPolicy` in the continuation shape | `bodies` with `state` |

The Studio writes this table nowhere as a rule: it shows each kind's note in the palette, and the editor's live
validation (the existing `useValidation`, which asks `validate` after a quiet delay) marks a refused combination at its
node with the codec's reason. As feature 014's remaining tasks and design 039 widen the subset during the release, the
codec changes and the Studio needs no change; the notes and this table are corrected in the same delivery as the codec
(documentation first).

The workflow settings panel (`WorkflowSettings.tsx`) gains `inputSchema` (a JSON editor with the subset of design 043
explained, the server's problems by pointer), `state` (schema, initial value, a table of Queries and Updates, each with
its argument schema, result schema, expression, and for an Update its `validate`, `state`, `result` and
`reapplyOnReset`), `limits` and `chainPolicy`.

**Bodies.** The document's `bodies` map is listed in a side panel. A loop's `body` field picks a body by name or
creates one. Opening a body replaces the canvas with the body's graph and shows a breadcrumb (`orders` › `lines`
body); the palette switches to the kinds of `$defs/bodyNode`, each with its note. The body canvas shows a problem until
it has exactly one `return`. When two
loops name the same body, the panel says so and editing it asks for confirmation once per session.

**Legacy and disabled kinds.** A node of a kind without a registry entry, of a legacy kind, or of a kind whose
capability is absent is drawn with the generic node card, read-only, with a reason: "The platform no longer runs this
kind of node" (legacy) or "This server does not enable this kind of node" (capability). The draft keeps it equal under
`canonical()` and in its place among the nodes (`documentIO.ts` changes from appending unreadable entries after the
known nodes to keeping each at its index). The replacement dialog maps what maps exactly and asks for the rest:

| From | To | Mapped | Asked |
|---|---|---|---|
| `foreach` | `forEach` | `collectionExpr` → `items`; `maxItems` and `maxConcurrency` when within 1 to 10,000 and 1 to 256 (`maxConcurrency` 1 when absent); `next`; `onError` | `result`; `body` (the legacy node named a node, the new one names a body); a `maxItems` or `maxConcurrency` out of range, shown with the old value |
| `humanApproval` | `approval` | `timeoutMs`; `onTimeout` → `onExpire`; `next` → `onApprove` | `policy` (`approverPolicy` is free text with no exact meaning); `onReject` |

`itemKeyExpr` and `formSchema` have no executable counterpart and are listed as dropped in the dialog before the author
confirms.

### 5. The run view

- **Iterations.** `model/runGraph.ts` (it exists today) gains the grouping of `RuntimeNode`s with an `instancePath`
  under their outermost scope node. The collection or loop card shows counts by status. Choosing it opens the iteration
  strip: a virtualized list (index, status, failure code) filterable by status; choosing an iteration draws its body
  graph with that iteration's statuses. Nested scopes (design 039) nest the same way.
- **Switch.** The card of a `switch` shows the chosen case id or "default" from `switchSelection`; `UNKNOWN` shows
  "an unknown case was recorded" and the id.
- **Approvals.** A run's approvals come from `GET /runs/{runId}/approvals`, and `youMayDecide` from
  `GET /namespaces/{namespaceId}/approvals?runId=` (Technical Plan 6). The decide dialog makes a decision key (a random
  128-bit value printed as hex) per open dialog and reuses it on retry; it shows the receipt (`replayed` when a retry
  found the first answer).
- **Queries and Updates.** The pinned version's `state.queries` and `state.updates` (from `GET /workflows/{id}/versions/{n}`)
  list the operations. A JSON argument editor sends the arguments; the server's `422` shows at its pointer. No schema
  validator runs in the browser (Architecture decision 0016 decision 1). An Update makes its `updateKey` per dialog,
  like a decision key. Results are shown as returned; a person who lacks `run:query` or `run:update` sees the panel
  unavailable with the reason.
- **Family.** Parent (from the run), children (from `childRunId` of the graph's `child` nodes and their run status),
  chain (from the chain route when the run has a chain).
- **Download.** "Download events" reads `GET /runs/{runId}/events` at most two pages a second, keeps each page's raw
  text as a part of the file being built (never parsed into objects), stops at 50,000 events or 32 MiB and says so, and
  saves JSON lines.

### 6. The inbox

**Server, approvals.** `GET /namespaces/{namespaceId}/approvals?decidable=true|false&workflowId=&runId=&cursor=&limit=`
lists pending approvals only and needs `run:read` and `approval:read`. The rule stays where it is:
`crates/domain/src/execution/approval.rs` gains `ApprovalRequest::may_decide(&Principal) -> bool`, extracted from
`authorize_decision` (tenant, `approval:decide`, the policy snapshot's subjects and roles, the initiator exclusion), and
`decide` calls it; an approval whose run has ended is not decidable, as the decision path refuses it. The adapter
(`crates/infra-postgres/src/approval_inbox.rs`) returns a bounded page of pending rows in key order, with their policy
snapshots and the run's status, joined to `workflows` for the name and to the run for its business identifier; the use
case (`crates/application/src/approval_inbox.rs`) builds the caller's principal from the current membership (as the
decision does) and computes `youMayDecide` with `may_decide`, then drops the undecidable rows when `decidable=true`. No
SQL predicate repeats the rule; the snapshots never leave the use case. A page reads at most 2,000 rows; when
`decidable=true` drops most of them it returns what it found with a cursor (Architecture decision 0144, decision 5). A
`runId` filter reads the run's own index (`approval_requests_run_idx`) and is outside the deadline scan. A migration adds
`approval_requests_pending_deadline_idx ON approval_requests (tenant_id, namespace_id, deadline, request_id) WHERE
status = 'PENDING'`; its `down` drops it; the table has one row per approval ever opened, and the migration test
records the build time on the test data set. Items: `approvalId`, `runId`, `workflowId`, `workflowName`, `businessId`,
`nodeId`, `instancePath`, `status`, `deadline`, `openedAt`, `youMayDecide`. No policy member, initiator, decider or
reason. Feature 026 adds its summary flag additively when it builds the summary.

**Server, publication requests.** `GET /namespaces/{namespaceId}/publication-requests?decidable=&workflowId=&cursor=&limit=`
lists `PENDING` requests only and needs `definition:read`. `crates/domain/src/versioning.rs` gains
`PublicationRequest::may_decide(&Subject) -> bool` with the three clauses of Architecture decision 0018 decision 4 (not
the requester, not the last editor, not a person who already decided it), which `decide` calls; the use case adds
`publication:approve`.

**Metrics.** `approval_inbox_requests_total{outcome}` and `publication_inbox_requests_total{outcome}` (`outcome` in
`ok`, `denied`, `invalid`, `unavailable`), `approval_inbox_rows_scanned` (a histogram); no tenant, namespace, workflow
or subject label.

**Studio.** `features/inbox/`: two tabs, approvals and publication requests, the decidable ones first, deadline
ascending, "time left" in words and as a number, refreshed every 30 seconds while the tab is visible, the time of the
last refresh shown. Deciding uses the existing operations; an item that settled meanwhile shows its new status instead
of an error.

### 7. The audit page

`features/audit/` at the tenant level (beside `namespaces`), shown in the navigation only with `audit:read`. Filters:
action (a picker that suggests the known actions and accepts any text), resource type and id, actor, from and to. The table shows time, actor, action,
resource and reason; a record view adds the trace link (the existing `TraceLink`). A resource link is made for the
types that have a page (run, workflow, schedule, script, connection, namespace). The download reads pages until 10,000
records or the end and saves JSON lines, saying which. The known actions are listed under `x-known-values` of the
`action` parameter of `listAuditRecords` in `openapi.yaml`, so `npm run check:api` keeps the Studio's types current;
`crates/domain/src/audit.rs` gains `AuditAction::ALL`, an exhaustive `match` that fails to compile when a variant is
added without being listed, and a test that `ALL` equals the OpenAPI list. The server already records each reading
(`audit.read`, collapsing within five minutes).

### 8. Versions, settings, events, scripts, palette

- **Versions** (grown from `features/workflows/VersionsPanel.tsx`, which already lists, shows and compares versions,
  into a page at route `workflows/:workflowId/versions`; design 044's Task 044-23 adds its column of open runs per code
  digest): versions list, activation history
  (`GET /workflows/{id}/activations`), activate or roll back (`PUT /workflows/{id}/active-version` with reason and the
  version seen), promote (`POST .../versions/{n}/promote`, destination picker, missing names shown), publication
  requests of the workflow, compensation plan (`GET .../versions/{n}/compensation-plans`) drawn as the reverse-order
  list with `after` dependencies.
- **Settings** (`features/settings/`, route `settings`): publication policy (get, set with a reason), quotas and
  activity quotas (read; set where the person holds `quota:manage`), worker bindings (list, bind, unbind), payload
  reveal grants (list, grant, revoke), and on the secret page the access list.
- **Events** (`features/events/`): event endpoints, webhook and broker subscriptions with their deliveries, connector
  dead letters, redrive of one item or of a subscription's failures, each confirmed and its receipt shown. Feature 021
  later adds resources and triggers next to them.
- **Scripts**: `features/scripts/languages.ts` reads `built` from the capability list; the constant keeps the labels
  and editor modes only.
- **Palette** (`src/app/CommandPalette.tsx`): opened by Ctrl or Command and K; searches the lists the person may read
  (workflows, scripts, schedules, connections by name prefix; a run by identifier or business identifier through
  `GET /namespaces/{namespaceId}/business-ids/{businessId}`), and the Studio's pages; never a route outside the
  person's permissions (`GET /auth/context`).

### 9. Contracts and documents

- `specs/001-platform-foundation/contracts/openapi.yaml`: `PlatformVersion.capabilities` (strings with the pattern
  and `x-known-values`); the two list operations with their schemas, errors and `x-implemented: '050'`; the known
  values of the audit `action` parameter; `frontend/apps/studio/src/api/schema.d.ts` regenerated; the OpenAPI
  compatibility baseline of `tests/contract/` updated after reading its diff.
- `docs/engineering/architecture-principles.md`: four rows with their checks (the registry against the schema; the
  capability names against `PlatformCapability` and the registry mapping; `may_decide` against `decide` for approvals
  and publication requests; `AuditAction::ALL` against the OpenAPI list).
- `docs/runbooks/studio.md`: the inbox, the audit page, the capability list and how an operator reads it.
- `specs/007-workflow-studio/design/README.md`: the new screens.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The inbox reveals who may decide | Architecture decision 0144: only the caller's own eligibility; a contract test refuses a policy field |
| The inbox scan grows with pending approvals | A partial index in deadline order and a bounded scan per page with a cursor |
| A legacy node is lost on save | The canonical form keeps unknown members; a test saves and reloads one |
| A capability shown by one replica is refused by another | The server's validation decides; the problem appears at validation (Gap 050-001) |
| Parallel Studio branches rewrite the same files | The order of Technical Plan 1; new primitives in new files |
| The bundle grows with every screen | The bundle budget test of Decision 050-001; pages load on demand as today |
| A run with 10,000 iterations freezes the page | A virtualized iteration list; the graph draws one iteration at a time |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No new state transition: decisions, updates, activations and redrives use the existing operations and their transactions; the new routes only read |
| II. Modular domain, ports and adapters | Pass | The eligibility rules stay in the domain (`ApprovalRequest::may_decide`, `PublicationRequest::may_decide`) and their decision paths call them; the use cases in `application` filter with them; the adapter only reads bounded pages; the capability names are a typed enumeration in `application`; no new dependency edge |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect is added |
| IV. Compatibility is versioned and pinned | Pass | Additive REST fields and routes; the Studio draws the IR and keeps nodes it cannot run; no IR change |
| V. Code, tenant boundary and secrets are isolated | Pass | Both routes take the tenant from the principal and answer another tenant's namespace as missing (battery); nothing of a policy, a payload or a secret leaves; no token in a URL or storage |
| VI. Observability and audit are part of the feature | Pass | Spans and closed-label metrics on both routes; decisions and changes keep their existing audit; audit reading is audited |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | No message is added; redrive uses feature 004's routes |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Property test of eligibility, contract tests of the capability list and the inbox schema, journeys with real PostgreSQL; nothing claimed before it runs |
| Delivery Constraints | Pass | One new frontend dependency only through pull request #267 (Astryx, accepted with conditions); no new server technology |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A capability list on `GET /version` | The palette must follow the server it talks to, and three other features need the same advertisement | A field per feature; the Studio's constant list (how it drifted) |
| Two list routes | A person cannot find what waits for them without them | The Studio asks every run (one request per run, no eligibility) |
| A body canvas | Bodies are named graphs of their own in the IR | Editing bodies as JSON |
| A guided replacement for legacy nodes | Drafts of the first Studio exist | Dropping or converting silently |

## Alternatives Considered

- **Build the catch-up on the hand-written interface layer and migrate later.** Rejected: every screen would be
  written twice; Architecture decision 0101 is accepted (Decision 050-001).
- **A generic form generated from the JSON Schema for every node.** Rejected for the reasons of Architecture decision
  0143 (handles, bodies and expressions need their own controls).
- **An inbox in feature 026 only (links in messages).** Rejected as the only path (Architecture decision 0144,
  option 4).
- **A server route for the journal download.** Rejected (Decision 050-008).

## Non-Functional Requirements

Non-functional requirements 050-001 to 050-004 above. Their tests: the cross-tenant battery and the canary checks of
the journeys (050-001); the performance journey with 10,000 pending approvals and a run of 10,000 iterations, recorded
with its environment (050-002); the existing contract baselines and the Studio suites unchanged (050-003); the metric
label test of the two routes (050-004).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Documents first (constitution, Documentation first): Task 050-2 lands before the code that serves the new fields. Tests
are seen failing first. Two builders can work in parallel after Task 050-2: the server builder (Tasks 050-3 to 050-5;
`crates/` only) and the editor builder (Tasks 050-6 to 050-8; `frontend/apps/studio/src/model/` and
`features/workflows/` only); the operations tasks (050-9 to 050-12; `features/runs/`, `features/inbox/`,
`features/audit/`, `features/versions/`, `features/settings/`, `features/events/`, `features/scripts/`, `src/app/`)
follow on either builder once Task 050-3 has merged; the domain part of Task 050-11 (`AuditAction::ALL` and its test)
goes with the server builder. A new primitive goes in a new file of `src/ui/`.

- [ ] T050-1 Accept Architecture decision 0101 with its two conditions: set its status in pull request #267 (or a
  follow-up on `develop` after it merges) to accepted under OD-2026-11-24-700; add the bundle budget test to
  `frontend/apps/studio` with the figures measured on the merged branch; run the performance journey and record it in
  `specs/007-workflow-studio/quickstart.md`.
  Verification: the budget test fails on an entry script 11% larger; the quickstart names the journey's result and
  environment.
  Refs: Requirement 050-022
- [ ] T050-2 Contracts and documents: `PlatformVersion.capabilities` (pattern and `x-known-values`), the two list
  operations, their schemas and errors, the known audit actions in `openapi.yaml`; the compatibility baseline; the four
  rows of `docs/engineering/architecture-principles.md`; `schema.d.ts` regenerated; `docs/runbooks/studio.md`, with the
  publishable subset of Technical Plan 4; the screen list in `specs/007-workflow-studio/design/README.md`.
  Verification: `contract_platform_foundation` (OpenAPI lint), `npm run check:api`; the docs checks.
  Refs: Requirement 050-002, Requirement 050-010, Requirement 050-011, Requirement 050-013
- [ ] T050-3 The capability list: `PlatformCapability` in `crates/application/src/capabilities.rs`, the static names
  in `apps/control-plane`, the registry-derived names and their mapping table, the route in
  `crates/api-http/src/routes/identity.rs`.
  Red tests: `capabilities_follow_the_switches_a_server_was_started_with`,
  `a_registry_activation_appears_within_one_heartbeat`, `every_gating_registry_capability_has_a_name`,
  `a_reader_ignores_an_unknown_capability`.
  Refs: Requirement 050-002
- [ ] T050-4 The approvals list: `ApprovalRequest::may_decide`, called by `decide` (`crates/domain/src/execution/approval.rs`),
  the use case that filters with it (`crates/application/src/approval_inbox.rs`), the bounded page query, the `runId`
  filter and the migration with the partial index and its `down` (`crates/infra-postgres`), the route and its metrics
  (`crates/api-http`).
  Red tests: `may_decide_equals_decide` (property, domain), `an_approval_on_an_ended_run_is_not_decidable`,
  `the_inbox_never_names_a_policy_member`, `the_run_filter_reads_outside_the_deadline_scan`,
  `a_page_stops_at_the_scan_bound_and_continues`, `another_tenants_namespace_is_not_found` (battery),
  `the_runtime_role_can_read_the_inbox` (`SAGAT_TEST_AS_RUNTIME_ROLE=1`).
  Refs: Requirement 050-010
- [ ] T050-5 The publication requests list: `PublicationRequest::may_decide` with three clauses, called by `decide`
  (`crates/domain/src/versioning.rs`); the use case with `publication:approve`; pending requests only.
  Red tests: `the_requester_and_the_last_editor_may_not_decide`, `a_person_who_decided_may_not_decide_again`,
  `an_approver_may_decide`, battery entry.
  Refs: Requirement 050-011
- [ ] T050-6 The registry: capability and scope per entry, the new kinds of Technical Plan 4, the schema drift test,
  the palette filtered by capability, legacy and disabled kinds read-only.
  Red tests: `registry_matches_the_schema`, `a_schema_with_a_new_kind_fails_the_registry_test`,
  `a_missing_capability_hides_the_lake_write`, `a_legacy_node_is_kept_in_place_under_canonical`.
  Refs: Requirement 050-001, Requirement 050-003, Requirement 050-004
- [ ] T050-7 The inspectors of `switch`, `approval`, `forEach` (with the stage list), `while`, `repeatUntil`,
  `return`, `continueAsNew`, `icebergWrite`; the workflow settings for `inputSchema`, `state`, `limits` and
  `chainPolicy`; server problems at their fields; the replacement dialog for legacy nodes.
  Red tests: one component test per kind and per setting; `the_replacement_of_a_foreach_asks_for_its_result`;
  `an_out_of_range_maxitems_is_asked_with_the_old_value`; `undo_restores_the_legacy_node`;
  `a_refused_combination_shows_the_codecs_reason_at_its_node`.
  Refs: Requirement 050-005, Requirement 050-006
- [ ] T050-8 The body canvas: bodies panel, breadcrumb, body palette, exactly one `return`, shared-body confirmation.
  Red tests: `a_body_without_its_return_shows_the_problem`, `a_shared_body_asks_once_before_a_change`.
  Refs: Requirement 050-007
- [ ] T050-9 The run view: iterations grouped and virtualized, the switch selection, the approvals panel and decision,
  the Queries and Updates panel, the family, the journal download.
  Red tests: `iteration_312_of_500_shows_its_steps`, `an_unknown_switch_selection_says_so`,
  `a_retried_decision_reuses_its_key`, `an_update_retry_reuses_its_key`, `the_download_equals_the_pages`,
  `the_download_stops_at_its_bound_and_says_so`.
  Refs: Requirement 050-008, Requirement 050-009, Requirement 050-014, Requirement 050-015, Requirement 050-021
- [ ] T050-10 The inbox page (`features/inbox/`).
  Red tests: `only_decidable_items_are_under_waiting_for_me`, `a_settled_item_shows_its_status_not_an_error`,
  `the_inbox_refreshes_only_while_visible`.
  Refs: Requirement 050-012
- [ ] T050-11 The audit page (`features/audit/`), `AuditAction::ALL` and the action list test in
  `crates/domain/src/audit.rs`.
  Red tests: `the_openapi_actions_equal_the_domain_actions`, `the_download_stops_at_ten_thousand_and_says_so`,
  `the_page_is_absent_without_audit_read`.
  Refs: Requirement 050-013
- [ ] T050-12 The versions page, the settings page, the events page, the built languages from the server, the command
  palette.
  Red tests: one component test per section with each role; `go_and_java_are_built_when_the_server_says_so`;
  `the_palette_never_calls_a_route_the_person_cannot_call`.
  Refs: Requirement 050-016, Requirement 050-017, Requirement 050-018, Requirement 050-019, Requirement 050-020
- [ ] T050-13 Journeys in `frontend/e2e/tests/` driven by `e2e_workflow_studio`: the journey of Success criterion
  050-001 and the journeys named by the requirements; axe in three themes and reflow at 320 pixels on every new page.
  Verification: `cargo test -p sagat-tests --test e2e_workflow_studio --locked -- --ignored journeys`, with its result
  recorded.
  Refs: Requirement 050-009, Requirement 050-012, Requirement 050-014, Requirement 050-016, Requirement 050-018, Requirement 050-022
- [ ] T050-14 The performance journey with 10,000 pending approvals and a run of 10,000 iterations, recorded with its
  environment and failures.
  Refs: Non-functional requirement 050-002

#### Follow-up tasks

None yet.

### Verification Plan

- Unit: the registry drift test; the inspectors; the run graph grouping; the inbox, audit, versions, settings, events
  and palette components (`npm test`).
- Property: `may_decide_equals_decide` for approvals and publication requests in `crates/domain`.
- Integration (real PostgreSQL): the two list routes, their scan bound, their runtime-role privileges
  (`SAGAT_TEST_AS_RUNTIME_ROLE=1`), the registry-derived capabilities, in a new target `integration_studio_catch_up`.
- Contract: the OpenAPI lint, the capability list against the switches, the inbox schema without policy fields, the
  cross-tenant battery, the audit action list.
- Journeys: `e2e_workflow_studio` with the new journeys; axe in light, dark and forced colours.
- Performance: the journey of Task 050-14, reported with its environment, never as a promise.
- Gates: `npm run lint`, `typecheck`, `test`, `build`, `check:api`; `python3 scripts/check-architecture.py`; the
  design-doc and docs checks.
