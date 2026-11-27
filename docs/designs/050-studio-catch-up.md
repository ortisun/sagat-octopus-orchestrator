# The Studio catch-up: every executable node kind, the approvals inbox, the audit log

*   **Author:** Claude Code, for the owner
*   **Status:** Approved
*   **Date:** 2026-11-26
*   **Reviewed-commit:** 34a282d5e36e93ae99d939852ddb1eb51617fd7d
*   **EGM-hash:** 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
*   **Claude-Code:** 2.1.332
*   **Reports:** docs/designs/050-studio-catch-up.reviews/2026-11-26/approval/reread-4-egm-review.md sha256:b5bdcec26c6cb7e39daaf180ace0186a3ba137fd6001d22a57e6b0ad13801a65; docs/designs/050-studio-catch-up.reviews/2026-11-26/approval/reread-6-architecture.md sha256:ef984e643bf1a83d275c8423ea7f8fd671bee09661dc6115d1d651cb5c968478; docs/designs/050-studio-catch-up.reviews/2026-11-26/approval/reread-18-security.md sha256:c642db2b442463ea1c55c7f5f7986b3c7c74dccc7bfa98116f356bca798cf65e; docs/designs/050-studio-catch-up.reviews/2026-11-26/approval/reread-4-correctness.md sha256:dca4da63e2bab9438b265a2b2ad0886d700cf9b3d2b62afab6b56e9ddf1eff3e
*   **Open findings:** Blocker 0, Major 0
*   **Accepted:** 2; OD-2026-11-26-900, OD-2026-11-26-901

--------------------------------------------------------------------------------

## Overview

The engine has moved faster than the Studio. On `develop` at 2026-11-24 the server publishes and runs `switch` and
`approval` nodes, publishes `forEach`, `while`, `repeatUntil` and `continueAsNew` in fixed shapes on every server,
collection operators behind design 042's deployment switch and lake writes when `lake.enabled`, lists and decides
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
(pull request #267), which this design accepts under the owner's delegation (Technical Plan 1). The server gains five things: a `capabilities` list on `GET /version`, a namespace-wide approvals list, a namespace-wide list of
publication requests, a paged list of the iterations of one loop or collection, and `youMayDecide` (a flag saying
whether the caller may decide the item now) on the two existing per-run and per-workflow lists. It makes two additive
corrections the screens need: a validation problem's pointer names the body of a body node, and the runtime
graph reports on a loop or collection node the counts of its iterations by outcome, taken from the engine's own
iteration events. Nothing changes in the IR (the
workflow document's schema, `workflow.schema.json`), the worker protocol or the SDK interfaces.

The owner is the product owner of the platform; the decisions of this design are taken under the owner's delegation
(Owner decision 2026-11-24-4 in [`docs/owner-decisions.md`](../owner-decisions.md)), and the reviews it settles are in
[`050-studio-catch-up.reviews/`](050-studio-catch-up.reviews/) (the resolutions of 2026-11-25 and 2026-11-26).

### Terms

- **Intent.** One body the person sent (a decision or an Update), frozen until the person changes it; a key made for
  an intent is reused only to send that same body again.
- **Owner node.** The loop or collection node whose body a scoped instance runs in; it carries the iteration counts.
- **Registry.** The Studio's list of node kinds (`frontend/apps/studio/src/model/registry.ts`): for each kind, its
  label, its fields, its handles (the links to other nodes) and, from this design, what it requires: `requires: { capability: string } | { driver: 'iceberg' } | null`.
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
- **Codec.** The server's conversion of a workflow document into its executable definition (`crates/infra-ir`), run by
  `validate` and by publication; its refusals are the validation problems the editor shows. "The server's validation"
  in this document means the same thing.
- **Chain.** The runs linked by `continueAsNew` (feature 014): each run is a segment; a chain-addressed Query or Update
  names the chain and reaches its current run; `chainPolicy` bounds the segments.
- **Declared state.** The `state` of a workflow document (feature 014, Architecture decision 0031): a schema, an initial
  value, and the Queries that read it and the Updates that change it; `expectedStateRevision` makes an Update refuse
  when the state moved; `reapplyOnReset` keeps an Update's arguments so a reset can apply it again.
- **Admitted iteration.** An iteration the engine has started (its scoped instances exist); a large collection admits
  iterations as concurrency allows.
- **Compensation.** A step, drawn with the `~c` suffix, that undoes a step in a saga (feature 003).
- **Reconciliation.** A step, drawn with the `~r` suffix, that finds out whether a step with an unknown outcome
  happened (feature 003).
- **The Studio served from another build.** An operator may point the server at a Studio bundle of another build
  (`SAGAT_STUDIO_DIR`, Architecture decision 0016), so the Studio and the server can differ in version; the Studio
  therefore treats what it bundles as suggestions and the server's answers as the authority.
- **Cross-tenant battery.** The shared contract suite that calls every route with another tenant's identifiers and
  expects the answer of a missing resource.
- **Runtime role.** The database role the deployed server runs as (`SAGAT_TEST_AS_RUNTIME_ROLE=1` runs a suite as
  it).
- **Referenced features.** 003 sagas; 004 events and webhooks; 007 the Studio; 011 versions and publication; 013
  federated login; 014 workflow primitives (loops, approvals, declared state, chains); 018 schedules and batches; 020
  run search; 022 flow control; 026 forms and notifications; 027 internal apps; 028 developer tooling; 029 roles and
  folders (its `x-access`, `discloses` and `visibleFolders` are the access declaration of a route, the item kinds an
  answer can reveal, and the folders a caller may see); 035 lake writes and step-output destinations; 039 nested
  collections; 041 step logs; 042 collection and stream operators; 043 run input validation; 044 workflow as code.

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

Measured against the two products this design is compared with:

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
  tenant) in the form of Architecture decision 0143; design 050 ships the field with the two names it uses
  (`workflow.collection-operators`, `workflow.reset-reapply`), each present when the answering replica would accept the
  matching publication. A reader MUST ignore a name it does not know, and an absent field means a server older than
  0.3.0. Other features add their names when they ship (Technical Plan 3). *Acceptance:* the contract test
  `capabilities_follow_the_switches_a_server_was_started_with` and `a_reader_ignores_an_unknown_capability`.
- **Requirement 050-003** The palette MUST offer only the kinds whose requirement is met: an advertised capability, or,
  for `icebergWrite`, Iceberg in the database drivers list; inside a body only the kinds of that body's scope, and never a legacy
  kind; each entry MUST say in words where it publishes in 0.3.0 (Technical Plan 4). A combination the codec refuses
  MUST be shown at its node, with the codec's reason, from the editor's live validation, never from a rule written
  again in the Studio. *Acceptance:* the Studio unit test with a drivers list without Iceberg shows no `icebergWrite`;
  the journey that places an `approval` inside a collection body shows the server's refusal at that node within the
  editor's validation delay (`useValidation`'s quiet delay), also when a root node has the same id (a validation problem's pointer names the body, Technical
  Plan 4).
- **Requirement 050-004** A draft that holds a node the palette would not offer (a legacy kind, or a kind whose
  capability is absent) MUST open with that node drawn, read-only, with the reason in words and the server's
  problem; saving the draft MUST keep the node equal under `canonical()` (`model/ir.ts`) and in its place among the
  nodes, in the root graph and in every body. *Acceptance:* a draft with a `humanApproval` node opens, saves and reloads
  equal under `canonical()`, the node in its place, with its problem shown; a draft with an entry without an `id`
  between two nodes and an entry with a duplicate id, in the root and in a body, keeps both at their index (this test
  fails today: `documentIO.ts` appends them at the end).
- **Requirement 050-005** The inspector of a legacy node MUST show its values read-only beside an action that adds the
  executable kind in its place ("Add a For each here", "Add an Approval here"): the new node takes every link that
  pointed at the legacy node, the author fills it and deletes the legacy node; nothing is converted for the author.
  *Acceptance:* the inspector test of each legacy kind; the action moves every incoming link (two predecessors, and an
  entrypoint) to the new node.
- **Requirement 050-006** The editor MUST edit, with its fields and handles, `switch` (up to 32 ordered cases and a
  default), `approval` (policy subjects and roles, initiator exclusion, timeout and the approve, reject, expire and
  error routes), `forEach` (items, up to eight collection-operator stages, body, result, the item and concurrency
  ceilings), `while` and `repeatUntil` (condition, body, initial, carry, iteration ceiling, delay, timeout),
  `continueAsNew` (input and state), `icebergWrite` (the fields of `$defs/icebergWrite`), the declared state
  (schema, initial value, Queries and Updates with their argument and result schemas), `inputSchema`, `limits` and
  `chainPolicy`. A field the server would refuse MUST show the server's problem at that field after `validate`.
  *Acceptance:* one component test per kind builds the node from the palette, fills it and shows a mocked
  `validate` problem at its field; a journey case of Task 050-13 publishes each kind in a shape of the publishable
  subset (Technical Plan 4).
- **Requirement 050-007** A body MUST be edited on a canvas of its own, opened from the loop node that names it, with
  a breadcrumb back to the workflow; the body canvas MUST show a problem until the body has exactly one `return` (the draft still saves, as every draft
  with a problem does, Requirement 050-004) and offer only the kinds of
  `$defs/bodyNode`, each saying where it publishes in 0.3.0 (Technical Plan 4); a body named by two loops MUST say so
  before it is changed. *Acceptance:* the editor test opens the body of a `forEach`, adds a step, and sees the problem
  shown until the body has its `return`.
- **Requirement 050-008** The run view MUST group scoped instances under their loop or collection node by
  `instancePath` and `nodeId`, show the counts of iterations by outcome that the runtime graph reports on the owner
  node (`SUCCEEDED`, `FAILED`, `DRAINED`, `RUNNING`, and for a collection the items not admitted), list the iterations
  from the server's paged iteration list (`GET /runs/{runId}/graph/iterations`, filterable by outcome, virtualized up to
  10,000), draw the body graph of one chosen iteration with compensations and reconciliations beside their forward
  step, show the case a `switch` chose from `switchSelection`, and show an `UNKNOWN` selection as such (Technical Plan
  5: the engine's own iteration events, never a rule written again in the Studio). The counts MUST stay current under
  `GET /runs/{runId}/graph/changes` without a full reload. *Acceptance:* the run view test with a recorded graph of a
  `forEach` of 500 items, a two-step body, one compensation, one retry and one step routed by `onError` shows the
  counts the graph reports, opens iteration 312 and shows its steps; the projection test that the counts and the listed
  outcomes equal the iteration events; the test that `graph/changes` polled across a retry, a return, a failure and a
  drain gives the same counts as a fresh `GET /runs/{runId}/graph`.
- **Requirement 050-009** The run view MUST list the run's approval requests with their status and deadline, and let a
  person decide one when `youMayDecide` (on the items of `GET /runs/{runId}/approvals`) is true, with a decision key
  per intent, an optional reason code, and the receipt shown; an ineligible person MUST see that they cannot decide,
  without the policy. The flag is a point-in-time hint the decision re-checks. A resend after a lost response, a `429`
  or a `5xx` MUST replay the first decision, and a changed body MUST use a new key, so the server never refuses it as a reused key; after a `403` or
  a `409` the status is re-read and shown. *Acceptance:* the journey
  `a_person_approves_a_step_from_the_run_view_and_the_run_moves_on`; `a_resend_replays_and_a_changed_decision_gets_a_new_key`.
- **Requirement 050-010** The server MUST list the pending approval requests of a namespace
  (`GET /namespaces/{namespaceId}/approvals`) under Architecture decision 0144, with a sealed cursor (Technical Plan 6): ordered by deadline, each item with
  `youMayDecide` computed by `ApprovalRequest::may_decide` on the caller's current principal, the
  filter `decidable=true` (`false` or absent: every pending item, each flagged), nothing of the policy, never an
  unbounded scan and always a cursor while rows remain. The items of `GET /runs/{runId}/approvals` gain the same
  `youMayDecide`. An approval whose run has ended is left out of the namespace list. *Acceptance:* the property test `may_decide_equals_decide`
  (domain), the contract test that no field names a policy member, the cross-tenant battery entry, the integration
  test that 2,001 undecidable rows followed by one decidable row are found in two requests, and
  `an_ended_run_is_left_out_of_the_inbox`.
- **Requirement 050-011** The server MUST list the open publication requests of a namespace that still need an approval
  under the current effective policy (`PENDING`, and `APPROVED` with fewer approvals than the policy now requires,
  because publication recounts the approvals against the policy in force when it publishes; a request is superseded
  when its draft is edited),
  oldest first, with the same paging guarantee (`GET /namespaces/{namespaceId}/publication-requests`), each with
  `youMayDecide` computed by `PublicationRequest::may_decide` with the open-state clause and the three clauses of
  Architecture decision 0018 decision 4 (not the requester, not the last editor of the draft, not a person who already
  decided it) and `publication:approve` held; each item carries the fields of Technical Plan 6, never who decided it or who
  last edited the draft; the items of `GET /workflows/{id}/publication-requests` gain the same flag. *Acceptance:*
  the contract test that the inbox item has exactly its fields; integration tests for the requester, the last editor, an approver who already approved a request
  that needs two approvals, another approver and a viewer; a policy raised after an approval puts the request back for
  another approver and not for the first; paging across a page boundary while a request is superseded.
- **Requirement 050-012** The Studio MUST have an inbox ("Waiting for me") that lists, for the chosen namespace (an
  inbox per namespace, Decision 050-011), the approvals (soonest deadline first) and the publication requests (oldest
  first) the person may decide, following cursors (at most five pages per list per refresh), and lets them decide from there; each refresh starts
  from the first page; the page MUST refresh only the inbox tab in view, and only while the browser tab is visible, back off while nothing changes
  (Technical Plan 6), and show when it last refreshed and whether more may wait. *Acceptance:* the journey
  `the_inbox_shows_what_waits_for_me_and_nothing_else`; `only_the_tab_in_view_is_refreshed`,
  `the_inbox_refreshes_only_while_visible`, and the test that an unchanged list is not polled at the base interval.
- **Requirement 050-013** The Studio MUST have an audit log page for holders of `audit:read`: the filters of
  `GET /audit`, an action picker that suggests the known actions and accepts any text (the Studio may be served from
  another build, Architecture decision 0016, `SAGAT_STUDIO_DIR`), a record view with a link to the resource when its
  type has a page, and a bounded download of the filtered records as JSON lines that says when it stopped at its bound.
  *Acceptance:* the component tests of the page; the test that the known actions of `audit-actions.json`
  equal `AuditAction` of `crates/domain/src/audit.rs`.
- **Requirement 050-014** The run view MUST let a person call the version's declared Queries and Updates (run- and
  chain-addressed) with JSON arguments (the server's `422` shown at its pointer; no schema validator in the browser,
  Architecture decision 0016 decision 1), an Update key per intent (a resend replays; a changed body uses a new key, so it is
  never refused as a reused key), `expectedStateRevision` prefilled with the revision last observed, and the result shown as
  Architecture decision 0031 discloses it. *Acceptance:* the journey `a_person_queries_then_updates_a_running_order`;
  `a_corrected_update_after_a_stale_revision_succeeds`; `a_second_tab_gets_a_revision_conflict`.
- **Requirement 050-015** The run view MUST show the run's family: its parent, its children with their status (from
  the runtime graph's `childRunId`), and its continuation chain (`GET /namespaces/{namespaceId}/chains/{chainId}/runs`).
  *Acceptance:* the run view test with a parent, two children and a chain of three segments.
- **Requirement 050-016** A workflow MUST have a versions page, grown from today's `VersionsPanel.tsx` in
  `features/workflows/`: the versions, the activation history, activate and roll back with a reason and the version
  the person saw, promote to another namespace, the publication requests of the workflow with request, approve,
  reject, withdraw and publish, and the compensation plan of a version. *Acceptance:* the journey
  `a_person_rolls_back_a_version_and_the_history_says_who_and_why`.
- **Requirement 050-017** A namespace MUST have a settings page with its publication policy, quotas and activity
  quotas, worker bindings, payload reveal grants and, on each secret, its access list; each control the person cannot
  use is shown unavailable with the permission it needs. *Acceptance:* component tests per section with each role.
- **Requirement 050-018** The Studio MUST show the delivery health of feature 004's event plumbing: event endpoints,
  webhook and broker subscriptions with their deliveries, connector dead letters, and redrive of one delivery, one
  dead letter or a subscription's failures, each redrive confirmed and its receipt shown. *Acceptance:* component
  tests; the journey `a_dead_letter_is_redriven_from_the_studio`.
- **Requirement 050-019** The script editor MUST offer Go and Java as built, as it offers TypeScript, Python and C#
  (`Language::is_built` holds for all five); a Studio served by another build learns otherwise from the create-script route's
  `LANGUAGE_NOT_BUILT` (`details.reason`; `ScriptService::create` refuses a language that is not built), shown at the
  language picker of `CreateScriptDialog.tsx`. *Acceptance:* the script editor test with Go and Java built; the test of
  a `LANGUAGE_NOT_BUILT` answer.
- **Requirement 050-020** The Studio MUST have a command palette (Ctrl or Command and K) that finds workflows, scripts,
  schedules and connections by name, a run by its identifier or business identifier, and the pages of the Studio,
  using only list routes the person may call. *Acceptance:* component tests; the palette never calls a route the
  person lacks the permission for.
- **Requirement 050-021** The run view MUST offer a bounded download of the run's journal as JSON lines, one line per
  event as `GET /runs/{runId}/events` returned it, honouring the server's rate answers, and saying when it stopped at
  its bound and that it is the journal as of the last page read. *Acceptance:* the test that the download's lines equal
  the items of the pages in order; the bound and `429` tests.
- **Requirement 050-022** Every new screen MUST be built from the Studio's interface module on Astryx (Architecture
  decision 0101). It MUST NOT use a raw control outside `src/ui/`, a literal colour, or a sample value where there is no
  data. It MUST support light, dark and forced colours and keyboard operation, give a table or list alternative to
  every graph, and show state by text and shape as well as colour (Requirement 007-008, Requirement 007-010). *Acceptance:* the ESLint boundary rule and the
  tokens test, which arrive with pull request #267, and axe in the journeys.

### Non-functional

- **Non-functional requirement 050-001 (security)** No new screen shows a value a route withheld, puts a token, an
  argument or a payload in a URL, a log or browser storage, or calls a route the person's permissions exclude without
  showing the refusal. The three new server routes are in the cross-tenant battery and answer another tenant's
  namespace or run as missing.
- **Non-functional requirement 050-002 (performance)** The inbox and the audit page answer a page in the time the
  benchmark records on a declared environment, with 10,000 pending approvals in one namespace; the journey also records
  the requests per second that 300 open inboxes send to one tenant, against the tenant's request budget. The run view
  renders a run with 10,000 iterations of an eight-step body without a task that blocks input for more than 50 milliseconds (the browser's long-task
  threshold), and the journey records the size of `GET /runs/{runId}/graph` for that run and the time
  the browser spends parsing it (Gap 050-006). No figure is promised.
- **Non-functional requirement 050-003 (compatibility)** The REST additions are additive: new routes, and new
  optional fields that readers ignore when they do not know them (`sdks/SPEC.md`); `ApprovalListItem` (a closed
  schema today) and `PublicationRequest` gain `youMayDecide` in the OpenAPI document, and the SDK type
  descriptions that say the per-run list carries no eligibility are corrected. One existing answer changes on purpose: an
ineligible caller deciding an approval of an ended run gets the eligibility refusal (`403` or
`INITIATOR_CANNOT_DECIDE`, with its denial audit record) where it got a retryable `503`, because the ended-run guard
moves after authorization (Technical Plan 6). No other answer changes, and no IR,
  worker-protocol or AsyncAPI change.
- **Non-functional requirement 050-004 (observability)** The three new routes have spans and closed-label metrics; a
  decision from the inbox is the existing decision operation with its existing audit.

### Success criteria

- **Success criterion 050-001** A person sets the tenant's collection capacity on the settings page, then builds,
  publishes and runs, without leaving the Studio, three workflows in
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
- **User story 050-02** As an approver, I find what waits for me in a namespace in one place and decide it (CUJ 3).
- **User story 050-03** As an operator, I follow a run with loops, call its Queries and Updates, and see its family
  (CUJ 4).
- **User story 050-04** As an administrator, I read the audit log and change a namespace's settings (CUJ 5).
- **User story 050-05** As a release manager, I roll back a version, approve a publication and promote it (CUJ 6).

## Goals

- The Studio offers every node kind the server runs, and nothing it refuses.
- One place per namespace for what waits for a person, decided by the server's own rule.
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
- A tenant-wide inbox: the inbox is per namespace, because the routes, the indexes and feature 029's permissions are
  per namespace (Decision 050-011).

## Open Questions

None open. All decisions below are delegated under Owner decision 2026-11-24-4, each weighed over its options and
recorded with its reasoning; only Decision 050-001 is also recorded in `docs/owner-decisions.md`, as OD-2026-11-24-700.
A "Stage N review" named in a decision is that stage of the egm-review of this design, whose report and resolution are
in `050-studio-catch-up.reviews/`.

### Decisions

- **Decision 050-001 — Accept Architecture decision 0101 (Astryx).** Options: accept as proposed; accept with
  conditions; reject and keep the hand-written layer. Chosen: accept with two conditions. The owner gave the direction
  (2026-11-21); the record holds a pre-1.0 library behind one directory (`src/ui`), with an exact pin, a theme that
  carries the approved visual direction and a contrast test; its continuous integration was green on pull request #267.
  Every screen of this design and of features 026 to 029 would otherwise be written twice. The conditions answer the two
  costs the record states and has not measured: (1) the performance journey runs on the merged branch before release
  0.3.0 and its result is recorded in `specs/007-workflow-studio/quickstart.md`; (2) a bundle budget test is added (the
  entry script and the compressed total of the production build may not grow past the figures measured on the merged
  branch plus 10% without a recorded reason) (Task 050-1).
  Recorded in `docs/owner-decisions.md` as OD-2026-11-24-700.
- **Decision 050-002 — One capability list for every feature.** Options: a field per feature (029's `accessControl`,
  027's `apps`, 028's `virtual-clock`); one list of names. Chosen: one list (Architecture decision 0143), because a
  client learns one shape and a feature adds a name, not a field. Features 027, 028 and 029 are corrected to add names.
- **Decision 050-003 — A refused kind is kept and shown, never dropped.** Options: drop it on load; convert it on load;
  keep it read-only and offer a guided replacement. Chosen: keep and offer. Dropping loses work; an automatic
  conversion guesses `result` for a `forEach` and the policy for an `approval`, which have no exact source.
- **Decision 050-004 — The inbox decides eligibility on the server.** Options: the Studio asks every run; the server
  returns the policy and the Studio computes; the server answers the caller's own eligibility by the rule that
  decides. Chosen: the last, one rule for listing and deciding with nothing of the policy in the answer (Architecture
  decision 0144).
- **Decision 050-005 — Publication requests join the inbox.** Options: approvals only; approvals and publication
  requests. Chosen: both, because both are "a person must decide before the work goes on", and the second needs only
  a list route over rows that exist.
- **Decision 050-006 — The known audit actions are a JSON file the Studio imports, held to the domain.** Options: a
  route that lists the actions; free text; `x-known-values` on the `action` parameter of `listAuditRecords`; a
  contract file bundled with the Studio. Chosen: `frontend/apps/studio/src/api/audit-actions.json`, held equal to
  `AuditAction::ALL` by one contract test (the type generator does not carry `x-` extensions, the Studio has no YAML
  parser, and no other client reads such a list, so an OpenAPI copy would be a second list without a reader), with
  free text still accepted because the Studio may be served from another build (Architecture decision 0016,
  `SAGAT_STUDIO_DIR`); settles Nit 15 of the architecture review of 2026-11-25 and Major 4 of the Stage 9 review.
- **Decision 050-007 — Go and Java are marked built in the Studio's constant.** Options: a capability name per built
  language (an earlier draft of this design); a field on the scripts routes; the constant. Chosen: the constant, settling Nit 8 of the
  Stage 5 review: `Language::is_built` is a compile-time constant true for all five languages, feature 023's languages
  need Studio changes anyway (a label, an editor mode, a sample), and a Studio served by another build learns of a
  missing language from the create-script route's `LANGUAGE_NOT_BUILT`.
- **Decision 050-008 — The journal download is client-side and bounded.** Options: a new export route; the existing
  `GET /runs/{runId}/events` read page by page. Chosen: the existing route, because it already applies the
  disclosure rules; design 044's `GET /runs/{runId}/history` (for replay) is a different document and is that
  design's. The download is bounded and honours the server's rate answers (Requirement 050-021).
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
- **Decision 050-011 — The inbox is per namespace.** Options: one tenant-wide inbox; an inbox per namespace with a count
  per namespace; an inbox per namespace. Chosen: per namespace, settling Major 5 of the Stage 7 review. Every route,
  index and cursor of this design is per namespace, and so are feature 029's grants; a tenant-wide list would scan each
  namespace the person can read on every refresh and merge their cursors, and a count per namespace costs the same
  scan. An approver who works in several namespaces switches namespace as every other Studio page asks; a tenant-wide
  inbox is a follow-up if the performance journey and use show it is needed.
- **Decision 050-012 — Iteration outcomes come from the iteration events, counted on the owner and listed by a paged
  route.** Options: a status field on every scoped instance (the first draft); a per-index list on the owner node; the
  counts on the owner and a paged list route. Chosen: the last, settling Majors 1 to 3 of the Stage 7 review. Every
  iteration event (`CollectionIterationAdmitted`, `...Returned`, `...Checkpointed`, `CollectionDrainStarted`,
  `CollectionIterationDrained`, `CollectionFinished`, and `LoopIterationAdmitted`, `LoopIterationReturned`,
  `LoopIterationCheckpointed`, `LoopDrainStarted`, `LoopIterationDrained`, `LoopFinished`) names the owner node, so
  counts on the owner stay current under `graph/changes`, whose rule is that a node changes only through events that
  name it; a status on each scoped instance would change without an event naming it; a per-index list on the owner
  would resend up to 10,000 entries with every iteration event.

### Gaps

- **Gap 050-001** The capability list is advice: during a rolling upgrade two replicas may answer differently. The
  server's validation stays the authority (Architecture decision 0143, decision 5). A new Studio that reaches an older
  replica also meets three other answers, and handles each: a new list route answering `404` with `ROUTE_NOT_FOUND`
  shows "this page is not available on this server", never "namespace missing"; an absent `youMayDecide` means
  unknown, so the decision is offered and the server decides.
- **Gap 050-002** The audit trail has no namespace column, so the audit page cannot filter by namespace.
- **Gap 050-003** The approval form, summary and notification fields of feature 026 are drawn by 026's Studio tasks on
  the inspector this design builds; until then the approval inspector shows only the fields of feature 014.
- **Gap 050-004** The subject picker of the approval policy is free text until feature 013's member list exists; then
  it lists the tenant's members (design 013, Requirement 013-020).
- **Gap 050-005** No count exists of drafts that hold legacy nodes; a guided conversion is built only if a count
  (from `control-plane` over stored drafts) shows they are common.
- **Gap 050-006** `GET /runs/{runId}/graph` returns every scoped instance of a run in one response, as it does today
  (feature 014): a collection of 10,000 items over an eight-step body is more than 80,000 nodes and as many edges, sent
  whole on every `reset` of `graph/changes`. This design does not page it; it adds no scoped node and moves the
  per-iteration list to a paged route. The performance journey records the response size and the parse time (Task
  050-14); if they exceed the 50-millisecond long-task threshold, a graph read by scope (the owners only, then the instances of one chosen
  iteration) becomes a follow-up task of its own.

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
*   **Then**: each node is edited through its own fields; the body canvas shows a problem until the body has its
    `return`; the
    `approval` in the body is marked at the node with the server's reason ("approvals inside collection and loop
    bodies wait for scoped compensation") within the validation delay; once she removes it, both workflows validate
    and publish.

### CUJ 2: An old draft with a legacy node (User story 050-01)

*   **Given**: a draft written with the first Studio that holds a `humanApproval` node.
*   **When**: the author opens it.
*   **Then**: the node is drawn, read-only, with "The platform no longer runs this kind of node: replace it with an
    Approval"; the draft saves unchanged; its inspector shows the old timeout, routes and approver text beside "Add an
    Approval here"; she adds the approval, fills it from the values shown, deletes the legacy node, and the draft
    validates.

### CUJ 3: An approver works her inbox (User story 050-02)

*   **Given**: three pending approvals in the namespace: two whose policies name the role `operator`, one that names
    only another subject; she is an operator and started one of the first two, whose policy excludes the initiator.
*   **When**: she opens "Waiting for me".
*   **Then**: one approval is listed as hers to decide, with its workflow, run, step, deadline and time left; the
    others are not listed, and nothing on the page says who may decide them; she approves, the
    receipt shows, the item leaves the list, and the run moves to its approve route.

### CUJ 4: An operator follows a large collection (User story 050-03)

*   **Given**: a failed run of a collection workflow over 2,000 items with a concurrency of 16, whose iteration 1,213
    failed permanently, which stopped the collection: the engine admitted no new iteration and the 15 iterations in
    flight ended without a result; and a run of an `order` workflow (no bodies) whose declared state has the Query
    `progress`.
*   **When**: the operator opens the first run, then the second.
*   **Then**: the collection node shows the counts the server reports, which add up to 2,000, for example 1,200
    succeeded, 1 failed, 15 drained and 784 not admitted; the iteration list filters to the failed one and the drained
    ones; the failed one opens with its body graph and its failure code; on the second run the Queries panel calls
    `progress` and shows its declared result.

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
   `requires` and `scope` in design 050's editor slice, by whichever branch merges second.
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
know; the OpenAPI document lists the names already shipped under `x-known-values` of the field, never as an `enum`, so
a new name is not a breaking change (`docs/engineering/architecture-principles.md`: readers tolerate what they do not
know). A response without the field comes from a server older than 0.3.0; the Studio then treats every name as
absent.

Three things are called a capability in this repository, and they stay distinct: the instance registry's `Capability`
(`crates/application/src/instances.rs`: what a binary can honour, recorded by the operator), the workflow switches
that publication follows (the collection-operators switch, read by `DefinitionService` in
`crates/application/src/definitions.rs` from `apps/control-plane/src/config.rs`, and the workflow capabilities of
`crates/infra-ir/src/capabilities.rs`, Architecture decision 0032 decision 8, of which `SAGAT_RESET_REAPPLY_ENABLED`
enables an Update's `reapplyOnReset`), and these advertised names (what a client
may offer). The advertised names are the variants of `AdvertisedCapability` in `crates/application/src/capabilities.rs`,
computed by one pure function, `advertised_capabilities(&CapabilitySwitches) -> Vec<AdvertisedCapability>` in that
file, which `apps/control-plane` calls at start-up and the test harness (`tests/support/api.rs`, from its `Options`)
calls too, so a test exercises the same mapping; `AppState` keeps a capability source (a field `capabilities` set by
`AppState::with_capabilities`), the start-up switches and, from the first feature that derives a name from the
instance registry, the registry member's kept requirement set, read on each request; it is served by
`platform_version` in `crates/api-http/src/routes/identity.rs`, which
gains the state, on both routers that mount `/version` (`crates/api-http/src/lib.rs`).

Design 050 ships two names, one per switch that changes what a field may hold:

| Name | Present when |
|---|---|
| `workflow.collection-operators` | the deployment's collection-operators switch is on (design 042) |
| `workflow.reset-reapply` | `sagat_infra_ir::reset_reapply_enabled()` is true (the codec's own switch, set from `SAGAT_RESET_REAPPLY_ENABLED`), so the name and publication read the same value; without the name the Studio shows that field read-only |

Step-output destinations (feature 035: the `stepDestinations` field of a step, which also writes the step's output to
a table or a stream) are refused by every server today (`DESTINATIONS_NOT_ENABLED`, `crates/infra-ir/src/convert.rs`)
and no switch turns them on, so design 050 ships no name for them: feature 035 adds `workflow.step-destinations` with
its switch, its variant and its test when it builds them (Decision 050-002), and until then the Studio shows the field
read-only, as for any absent name.

Lake writes are already advertised by the database drivers list (`GET /namespaces/{namespaceId}/database-drivers`
lists Iceberg only when `lake.enabled`; the route is mounted only with the connections service, so a `404
ROUTE_NOT_FOUND` there means no Iceberg), which the Studio reads for `icebergWrite`; no second advertisement is added.
The loops, `switch`, `approval` and `continueAsNew` need no name: publication accepts them in the shapes of Technical
Plan 4 on every 0.3.0 server, and against a server without the field the Studio offers every kind that needs no name
and lets validation decide.

A field whose capability is absent (the collection-operator stage list of `forEach`, an Update's `reapplyOnReset`,
`stepDestinations`) is shown
read-only with the reason and kept on save, as a node of a disabled kind is.

Each other 0.3.0 feature adds its names, with a variant and a row here, when it ships: feature 022
(`workflow.flow-control.concurrency`, `workflow.flow-control.rate`), 027 (`apps.*`), 028 (`testing.virtual-clock`, `testing.history-replay`, `tooling.git-sync`, `tooling.template-catalog`), 029
(`access.*`), 013 (`identity.*`). The instance registry
([Architecture decision 0082](../adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md))
is unrelated to the Studio's node registry: every server process registers in it as a member and sends a beat every
few seconds; an operator records a requirement there (a capability every process must have), which raises an epoch;
a process that lacks a recorded requirement keeps a closed gate (it stays up and serves nothing it would get wrong).
Design 050 changes nothing in the registry. A name that follows an operator's activation in the instance registry
(022's, 029's restriction names, 013's) needs the registry member (the process's own client of the registry) to keep
the activated requirement set: today the registry member reads the set only when a beat sees a new epoch and does not
keep it (`Member::decide`), so a replica started after an activation would never learn it. The first feature that
ships such a name (022, 029 or 013) makes the registry member read the set after a
successful registration and on every epoch change, keep it beside the gate, and derive its names from it through one
tested mapping, with the test `a_replica_started_after_an_activation_advertises_it` (Architecture decision 0143
decision 2).

### 4. The registry and the editor

`model/registry.ts` keeps one entry per `type` constant, with `requires`, its scopes (`root`, `body` or both), its
fields and handles per scope (a root `activity` and a body `activity` are different schema members with different
required fields), and a one-line note of where it publishes in 0.3.0. `registry.test.ts` reads
`specs/001-platform-foundation/contracts/workflow.schema.json` (imported by path in the test, not copied) and asserts:
every `type` constant of the members of `$defs/node` and `$defs/bodyNode` has one entry with exactly those scopes;
every required property of the member of each scope other than `id` and `type` is a field or a handle of that scope,
or is listed by the entry as owned by its custom editor (`DatabaseFields`, the connection picker of `icebergWrite`);
the legacy kinds list their required fields for the read-only inspector (`humanApproval`'s `formSchema` among them);
the test resolves `allOf`, where the legacy members declare their `type` and `required`; no entry names a kind outside
the schema.

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
| `icebergWrite` | the fields of `$defs/icebergWrite`, with the connection picker of the database node | `next`, `onError` | none; requires `{ driver: 'iceberg' }`, Iceberg in the database drivers list |
| `database` | exists (`DatabaseFields`) | | none |

**The publishable subset of 0.3.0.** The schema allows more than publication accepts. On `develop` at 2026-11-24
(`crates/infra-ir/src/lib.rs`, `published_definition`; `crates/infra-ir/src/convert.rs`;
`docs/runbooks/workflow-primitives.md`):

| Shape | Publishes | Refused |
|---|---|---|
| Basic graph | activity, database, script, condition, parallel, timer, waitEvent, child, saga, switch, approval, end, and `icebergWrite` with lake writes on | |
| Collection workflow | source activities (the activities before the collection that produce its items), then a chain of `forEach` nodes ending at `end`; a body of one to eight activities, conditions, switches, timers and event waits with local routes and one `return` | triggers, `chainPolicy`, an approval in a body or beside a collection, `while`, `repeatUntil`, `continueAsNew` beside a collection |
| Loop workflow | a chain of `while` or `repeatUntil` nodes ending at `end`, recovery activities (the activities a loop's error route leads to); a body that is an activity chain | an activity after a loop's success edge, triggers, `chainPolicy`, an approval in a body |
| Declared state | `state` with Queries and Updates; `continueAsNew` with `chainPolicy` in the continuation shape | `bodies` with `state` |

The Studio writes this table nowhere as a rule: it shows each kind's note in the palette, and the editor's live
validation (the existing `useValidation`, which asks `validate` after a quiet delay) marks a refused combination at its
node with the codec's reason. A codec problem about a body node names it by its bare id today (`problems_of`,
`crates/application/src/definitions.rs`), which the Studio cannot place inside a body or tell from a root node of the
same id; the codec therefore sets the problem's existing JSON pointer to `/bodies/{name}/nodes/{index}` for a problem about one
body node, and to `/bodies/{name}` for a problem about a whole body (as `EscapingBodyEdge` already does in
`problems_of`). The refusals about body nodes are raised at seven sites, each of which learns the body's name and the
node's index from the raw document by iterating `bodies` with its keys and keeping each node's position: the body
loop of `structured_definition_with_policy` (`crates/infra-ir/src/convert.rs`), whose `nodes(...)` errors (a kind not
allowed in a body, `DESTINATIONS_NOT_ENABLED`) get the node's pointer and whose `BodyDefinition::new` errors other than
`UnknownTarget` (a body without exactly one `return`, an `end` inside a body, a duplicate node, a cycle, an
unreachable node) get the node's pointer when they name a node and `/bodies/{name}` otherwise, so the body canvas's
return problem is the server's, never a rule of the Studio;
`approval_in_a_body` (`crates/infra-ir/src/convert.rs`, a pre-scan of the raw document), `operators::check_surfaces`
(`crates/infra-ir/src/operators.rs`, a pre-scan), `check_expressions` (`crates/infra-ir/src/lib.rs`, after conversion:
it is given the index map the conversion builds from the raw document), `published_collection_definition` and
`published_loop_definition` (`convert.rs`, whose `Unsupported { kind: "body" }` refusals name a whole body and get the
`/bodies/{name}` pointer), and `DefinitionService::refused_collection_operator_node`
(`crates/application/src/definitions.rs`), which works on the converted definition and is given the index map the
codec returns with it. The `IrError` variants they raise gain an optional body scope (the name,
and the index when the problem is about a node); `problems_of` writes the pointer when the scope is set and keeps
`nodeId` as today. In `validationModel.ts`, `resolveNodeId` reads a `/bodies/{name}/nodes/{i}` pointer before `nodeId`,
so a colliding root id never wins, and places a `/bodies/{name}` pointer on the body itself; no field is added. The codec reports its first
structural refusal only, so a draft with two refused combinations shows the second after the first is fixed. As feature 014's remaining tasks and design 039 widen the subset during the release, the
codec changes and the Studio's behaviour needs no change; only the per-kind notes in `registry.ts` and this table are
corrected in the same delivery as the codec (documentation first).

The workflow settings panel (`WorkflowSettings.tsx`) keeps its `inputSchema` editor (gaining the subset of design 043
explained and the server's problems by pointer) and gains `state` (schema, initial value, a table of Queries and Updates, each with
its argument schema, result schema, expression, and for an Update its `validate`, `state`, `result` and
`reapplyOnReset`), `limits` and `chainPolicy`.

**Handles and the body model.** `HandleSpec` (`model/registry.ts`) gains a nested form,
`{ list: 'cases', field: 'target', edge: 'case' }`, for the targets inside a list of objects, and `EdgeKind` gains
`case`, `default`, `approve`, `reject` and `expire`; `linksOf` and the edit operations read and write the nested form.
Today every operation of `model/edit.ts` works on the root document, and `readDocument` and `openDraft` carry `bodies`
untouched; a body-document adapter (`model/bodies.ts`, Task 050-16) reads a body as a graph of its own (its nodes and
its `entrypoint`), applies the same edit operations to it, and writes it back in place, so the body canvas reuses the
root editor. The capability list is read by one hook, `useCapabilities` in `src/api/capabilities.ts` (query key
`['version']`, the Studio's first call of `GET /version`).

**Bodies.** The document's `bodies` map is listed in a side panel. A loop's `body` field picks a body by name or
creates one. Opening a body replaces the canvas with the body's graph and shows a breadcrumb (`orders` › `lines`
body); the palette switches to the kinds of `$defs/bodyNode`, each with its note. The body canvas shows a problem until
it has exactly one `return`. When two
loops name the same body, the panel says so and editing it asks for confirmation once per session.

**Legacy and disabled kinds.** A node of a kind without a registry entry, of a legacy kind, or of a kind whose
capability is absent (a variant no kind of this design uses; it is there for feature 022's entries) is drawn with the generic node card, read-only, with a reason: "The platform no longer runs this
kind of node" (legacy) or "This server does not enable this kind of node" (capability). The draft keeps it equal under
`canonical()` and in its place among the nodes. A legacy node already keeps its index (`readDocument` reads it as a
node); what moves today are entries without an `id` or a `type` and duplicate ids, which `documentIO.ts` appends after
the known nodes; `documentIO.ts` changes to keep each entry at its index, in the root graph and in every body. The legacy node's
inspector shows its values read-only beside "Add a For each here" or "Add an Approval here", which adds an empty node
of the executable kind and moves to it every link that pointed at the legacy node (each predecessor's handle now names
the new node; for an entrypoint, the entrypoint moves); the legacy node keeps its values and its own outgoing links
until the author deletes it (a read-only node can be deleted). The author fills the new node from the values shown. No automatic mapping is offered: no count of drafts with legacy nodes exists, and a guided conversion
(Gap 050-005) is built only if one shows they are common.

### 5. The run view

- **Iterations.**

  *Outcomes.* The engine stops a collection or a loop at its first permanent iteration failure: the failed
  iteration is named by `CollectionDrainStarted` or `LoopDrainStarted` with the reason `Failed { iteration, code }`,
  admission stops, and each iteration already in flight ends either with a result (`...Returned`, then
  `...Checkpointed`) or without one (`CollectionIterationDrained`, `LoopIterationDrained`); a collection's drain for the run-data
  limit, the storage quota or a cancellation, and a loop's for a timeout or a cancellation, names no failed iteration (`crates/domain/src/execution/event.rs`). An
  iteration's outcome is therefore one of `SUCCEEDED` (checkpointed), `FAILED` (the one named by the drain), `DRAINED`
  (ended without a result during the drain) and `RUNNING` (admitted, not yet settled); a collection also has items not
  admitted (`item_count` of `CollectionEntered` minus the admitted ones), which a loop, with no known total, has not. A
  retried step is `RUNNING` in its iteration: its own node shows the retry, and the iteration's outcome does not
  change. Under this rule a step that failed and was routed by `onError` belongs to a succeeded iteration, and
  compensations and reconciliations are not iterations.

  *Counts on the owner.* The runtime graph projection gains, on a collection or loop owner node, `iterations`: the counts by outcome and, for a
  collection, `notAdmitted` and `total`; an additive field of `RuntimeNode`. Every iteration event names the owner
  node, so `graph/changes` re-sends the owner whenever a count changes and the counts stay current (Decision 050-012).
  No field is added to scoped instances.

  *The iteration route.* The outcome of each iteration is read from a new paged route,
  `GET /runs/{runId}/graph/iterations?owner=&outcome=&after=&limit=` (`run:read`; the route admits the run first, as
  `crates/api-http/src/routes/execution.rs` does for its other run routes, and reads the query by hand only after, so no
  refusal about the query comes before the refusal about the run; `owner` is the owner's `RuntimeNode.id`, at most 1 KiB
  and within design 039's nesting limit, resolved by the mapping that produced it, and an unknown owner answers an empty
  page with no detail about the run; answers carry `Cache-Control: no-store`, as the per-run list does; `limit` 1 to 500, default 100; `after` the last index
  returned, with the outcome filter applied in the query, so the keyset holds under a filter; items `index`,
  `outcome`, `failureCode` on the failed one, and `entryNodeId`, the `RuntimeNode.id` of the iteration's entry
  instance, which the Studio uses to draw that iteration).

  *The source.* Both read the engine's progress tables, written in the transactions that append the iteration
  events: `iteration_scopes` and `collection_iterations` for a collection, `loop_scopes` and `loop_iterations` for a
  loop (iteration lifecycles `ADMITTED`, `CHECKPOINTED`, `DRAINED`). The failed iteration is already stored: the drain
  writes `drain_kind`, `drain_iteration` and `drain_failure_code` on `iteration_scopes` and `loop_scopes` (migrations
  `20261103000000_collection_drain` and `20261107000000_loop_drain`; the statements in
  `crates/infra-postgres/src/primitives.rs` and `loop_progress.rs`), in the transaction that appends the drain event, so
  this design adds no column and no write. The mapping, whose first rule takes precedence over the iteration's own lifecycle
  (the failed iteration's row stays `ADMITTED` until it is drained, then becomes `DRAINED`): the index equal to
  `drain_iteration` when `drain_kind` is `FAILED` is `FAILED`, with `drain_failure_code`; `CHECKPOINTED` is
  `SUCCEEDED`; another `DRAINED` is `DRAINED`; `ADMITTED` is `RUNNING`; not admitted is `item_count` minus `admitted_count`. The counts are one grouped query per run (grouped by scope), never one per owner, so design 039's nested
  owners do not multiply the reads of each `graph/changes` poll; they are added to `GraphSnapshot`
  (`crates/application/src/runs.rs`) as a field `iterations` by `RunStore::graph`'s adapter
  (`crates/infra-postgres/src/execution.rs`) and written onto owner nodes by `runtime_graph` in
  `crates/api-http/src/dto_exec.rs`; `graph_changes` reads the snapshot again on every poll, so a re-sent owner carries
  current counts. The page is `RunService::iteration_page` over a new `RunStore::iteration_page`, adapter
  `crates/infra-postgres/src/iteration_read.rs`, route in `crates/api-http/src/routes/execution.rs`.

  *Drawing.* `model/runGraph.ts` groups `RuntimeNode`s by `instancePath` and `nodeId` (never by parsing `id`) under their
  outermost scope node and draws: the card shows the reported counts; the iteration strip (`useWindowedRows` of
  `features/runs/useWindow.ts`) pages the iteration list for the rows in view, filterable by outcome, and reads the
  rows in view again when the owner arrives in `graph/changes`; choosing an iteration draws its body graph from the
  instances of the graph whose `instancePath` names it, with compensations (`~c`) and reconciliations (`~r`) beside
  their forward step. Nested scopes (design 039) nest the same way.
- **Switch.** The card of a `switch` shows the chosen case id or "default" from `switchSelection`; `UNKNOWN` shows
  "an unknown case was recorded" and the id.
- **Approvals.** A run's approvals and their `youMayDecide` come from `GET /runs/{runId}/approvals` (Technical Plan 6).
  The decide dialog uses one hook, `useIntentKey`, that makes a key with `crypto.randomUUID()` when the person sends
  and puts it in the body field the operation defines (`decisionKey` of the decision, `updateKey` of an Update; not the
  `Idempotency-Key` header): the body is frozen, only the person resends ("Send again", offered after a lost response,
  a `429` or a `5xx`; the client retries nothing by itself) and reuses the key, and a changed body gets a new key, because a refused decision or Update keeps a durable receipt under
  its key. It shows the receipt (`replayed` when a resend found the first answer); a `403` or `409` after a sent
  decision re-reads the run's approvals and shows their status.
- **Queries and Updates.** The pinned version's `state.queries` and `state.updates` (from `GET /workflows/{id}/versions/{n}`)
  list the operations. The arguments are edited in the existing `JsonField` and sent; the server's `422` shows at its
  pointer. No schema validator runs in the browser (Architecture decision 0016 decision 1). An Update uses
  `useIntentKey`, and `expectedStateRevision` is prefilled with the revision the panel last observed, so a second tab
  gets `409 REVISION_CONFLICT` instead of applying twice. Results are shown as returned; a person who lacks
  `run:query` or `run:update` sees the panel unavailable with the reason.
- **Family.** Parent (from the run), children (from `childRunId` of the graph's `child` nodes and their run status),
  chain (from the chain route when the run has a chain).
- **Download.** "Download events" reads `GET /runs/{runId}/events` (the existing `endpoints.journal`) through `collectPagesBounded`, one bounded
  variant of the existing `collectPages` (`api/endpoints.ts`) that this design adds and that the inbox and the audit
  download also use; it takes its wait from the existing `ApiError.retryAfterMs` (`src/api/errors.ts`, capped at one
  minute): it stops at a limit of pages, items or bytes (here 50,000 events or 32 MiB), waits for `Retry-After` on a `429`
  and stops after three refusals, and reports whether it stopped early. Each item is written as one compact line as the
  route returned it; the file says it is the journal as of the last page read.

### 6. The inbox

**Server, approvals.** `GET /namespaces/{namespaceId}/approvals?decidable=true|false&cursor=&limit=`
lists pending approvals only and needs `run:read` and `approval:read` (admission `ApiOperation::ApprovalRead` with
`ApiTarget::Namespace`, which answers another tenant's namespace as missing).

*Eligibility rule.* The rule stays where it is:
`crates/domain/src/execution/approval.rs` gains `ApprovalRequest::may_decide(&Principal) -> bool`,
extracted from `authorize_decision` (tenant, `approval:decide`, the policy snapshot's subjects and roles, the initiator
exclusion), and `decide` calls it with the same inputs.

*Port and adapter.* A new port
`ApprovalInboxStore` (`crates/application/src/approval_inbox.rs`) has one method,
`pending_page(tenant, namespace, after: Option<(OffsetDateTime, ApprovalRequestId)>, scan: u32) -> InboxScan`, where
`InboxScan` holds the rows (`ApprovalRequest` with its policy snapshot and initiator, the run's status, the workflow's
name and the run's business identifier) and the key of the last row scanned; its adapter
(`crates/infra-postgres/src/approval_inbox.rs`) reads a page in one statement, joining `runs`, `workflows` and the
business identifier with `tenant_id` in every join predicate and reading the policy columns in the same row, and takes
no row lock (the runtime role holds only column-level `UPDATE` on `approval_requests`). It never loads row by row as
`list_for_run` does today (one statement per request through `load_request`), which would make a 2,000-row page
2,000 statements; the test `an_inbox_page_is_one_statement` counts them. The use case takes the caller's principal from the request,
which the request path builds from the current membership on every request (a point-in-time hint; the decision reads
the role again in its own transaction), and computes `youMayDecide` with `may_decide(principal)`, then drops the
undecidable rows when `decidable=true`. The per-run list computes the flag from the same row type, read for one run by
one statement as well.

*Ended runs.* A run's end cancels its pending approvals in the run's own transaction
(`crates/infra-postgres/src/execution.rs`), so a pending approval of an ended run should not exist; the probe
`every_run_end_cancels_its_pending_approvals` is Task 050-4's first red test and checks every run-end path: completion, failure, cancellation, termination, a
run timeout, continue-as-new and a reset. The inbox's
scan reads pending rows of runs that have not ended (a run-status predicate of the scan, a data filter like
`status = 'PENDING'`, not the eligibility rule), and `may_decide` and `decide` keep their inputs. The decision's
adapter (`crates/infra-postgres/src/approval_progress.rs`) today refuses an ended run before it reads the caller's
principal, with a retryable `503`; that guard moves after authorization and reads the status captured before `decide`
(which changes the status in memory), so an ineligible caller is refused for eligibility and never learns that the
run ended, and an eligible caller still gets the `503` it gets today, with no receipt, route or outbox row written
(`an_eligible_caller_on_an_ended_run_still_gets_503`, the row seeded). No SQL
predicate repeats the eligibility rule; the snapshots never leave the use case.

*Paging and access filters.* A page examines at most 2,000 pending rows of the namespace in key order: the statement
takes them from the partial index in an inner query with `LIMIT 2000` after the cursor's key, and only then applies
the data and access predicates (the run not ended, and 029's `visibleFolders` when it exists) and the joins, so the
work of a page is bounded whatever the predicates drop, and `inbox_rows_scanned{list="approvals"}` counts the rows examined.
The use case keeps rows until it has `limit` items or the examined rows run out: when the page fills, the cursor is
the key (`deadline`, `request_id`) of its last item; otherwise the key of the last row examined; it is null only when
no row remains (Architecture decision 0144, decision 5). Because a cursor can then be the key of a row the caller may
not see, the cursor is sealed.

*Cursor sealing.* The key is a deployment-wide cursor key kept in the table `inbox_cursor_keys` (`key_id`, an
identity column whose values are never reused, so the newest key is the one with the highest id; `key_bytes`, 32
random bytes; `created_at`; `created_by`, the `--operator` value, bounded and printable as `handover.rs` checks, or
`migrate`; `created_db_role`, `session_user`, as the trigger handover's `changed_db_role` records it; the class
`protected_system`, with its entries in `crates/infra-postgres/storage-classification.json`; the runtime role has
`SELECT` only). `sagat_infra_postgres::migrate`, with the schema-owning role, creates the first key after the schema
migrations, in one transaction that takes `LOCK TABLE inbox_cursor_keys IN SHARE ROW EXCLUSIVE MODE` and inserts only
when the table is empty, so concurrent migrations and later upgrades create no other key; the bytes come from
`getrandom` 0.4, as `infra-secrets` draws them, and a failed random source is returned as `MigrateError::Source`, so
`migrate` keeps its signature (the test harness, which calls the same function, gets the key too). `control-plane
inbox-cursor-key rotate --operator <name>` (schema-owning role, printing only the new key id) runs one transaction
under the same lock that inserts a new key and deletes every key but the two newest, so the previous key opens cursors
until the next rotation; the runbook asks for a rotation every year. The key bytes are redacted in `Debug`, never
logged or printed, and the table is never exported. It is not a secret of the platform: a cursor hides only the key of
a row from a caller, and whoever can read the table can read the rows themselves, so it needs neither OpenBao nor
`SAGAT_SECRETS_KEYS` and the inbox works on every deployment.

Each cursor is sealed with AES-256-GCM (the `aes-gcm` crate that `infra-secrets` already uses, moved to
`[workspace.dependencies]`) under a key of its own, derived by HKDF-SHA256 (`hkdf` 0.13, matching the workspace's
`sha2`; `aes-gcm`, `hkdf` and `getrandom` become direct dependencies of `infra-postgres`, Task 050-4) from the cursor
key, with a random 16-byte salt drawn for that cursor and, as `info`, the label `approval-inbox-cursor` or
`publication-inbox-cursor` followed by the tenant. Two cursors share a key only when they share a salt, so the
random-nonce limit of 2^32 messages per key never applies, whatever the number of replicas and the request budget,
and the yearly rotation is hygiene rather than a bound. The associated data is the tenant, the namespace and the
caller (the principal's `Subject`), so a cursor reveals no row key and opens only for its caller; every field of the
`info` and of the associated data is length-prefixed (a 4-byte big-endian length, then the bytes), so no two different
tuples encode alike. Its wire form is
`<key id>.<salt>.<nonce>.<ciphertext>` (the key id in decimal, the rest in base64url), and `open` derives the key from the cursor's own key id and salt.

The use cases seal and open cursors through a port, `InboxCursorSealer`, whose methods are asynchronous and
object-safe, so it is wired as `Arc<dyn InboxCursorSealer>` as the application's other ports are (`async
seal(&InboxScope, &InboxKey) -> Result<String, SealerError>`, `async open(&InboxScope, &str) ->
Result<Option<InboxKey>, SealerError>`, where `InboxKey { at: OffsetDateTime, id: Uuid }` is the deadline and the
approval request's id, or the request time and the publication request's id, and `InboxScope { list: InboxList,
tenant, namespace, subject }` names the closed `InboxList` (`Approvals`, `Publications`) that picks the HKDF label, so
a cursor of one list never opens on the other: `an_approval_cursor_does_not_open_on_the_publication_list`) in
`crates/application/src/approval_inbox.rs`, shared by both inbox use cases; its adapter is
`crates/infra-postgres/src/inbox_cursor.rs`, wired by `apps/control-plane` and `tests/support/api.rs`, so no
dependency edge is added. The adapter loads the keys at start and reads the table again on a timer every minute,
outside any transaction; a failed read keeps the cached keys. Each read takes every key's age from the database
(`now() - created_at`), so no replica's clock is compared with another's, and the adapter adds the time elapsed on its
own monotonic clock, which tests move instead of waiting. It seals with the newest key older than two timer periods
(two minutes), or with the oldest key when none is that old, as with the first key; so every replica whose reads
succeed holds a key before any cursor uses it, and the adapter never reads on demand. A `rotate` while the newest key
is younger than two minutes is refused, so a sealing key always remains. A cursor that is not in canonical form is the malformed-cursor `400` before any key lookup: the key
id in decimal, `[1-9][0-9]{0,18}` and at most `i64::MAX`; the salt, the nonce and the ciphertext in base64url without
padding and with zero trailing bits (the decoder's strict default), the salt exactly 16 bytes, the nonce exactly 12
and the ciphertext at least 16 (`a_cursor_with_a_wrong_field_is_malformed`: `+1`, `01`, `0`, an overflowing id, a
15-byte salt, an 11-byte nonce, a salt whose last character sets a trailing bit); and one whose key id the replica does not hold is answered
as a malformed cursor (`400`), which costs the Studio only a fresh first page. With no key loaded (an empty table, or
no successful read since start), `SealerError::Unavailable` answers `503` with `Error.code` `UNAVAILABLE`,
`details.reason` `INBOX_CURSOR_KEYS_UNAVAILABLE`, retryable, and `Retry-After` set to the seconds until the next timer
read, and the metric outcome is `unavailable`; start-up still succeeds. Every timer read is counted,
`inbox_cursor_key_reads_total{outcome}` (`ok`, `failed`), the gauge `inbox_cursor_keys_last_read_age_seconds` gives the
time since the last successful read (no tenant label), and a failed read is logged once per run of failures, so a
replica sealing under a deleted key is seen before its cursors fail elsewhere
(`a_failed_key_read_is_counted_and_logged_once`, which also checks that the gauge grows while reads fail). The envelope `<key id>.<salt>.<nonce>.<ciphertext>` has the same
form as design 013's sign-in attempt cookie; each adapter keeps its own codec, so no crate edge is added, and both are
tested against one table of malformed inputs, `tests/fixtures/sealed_envelope_malformed.json` (each entry at most 512 bytes, so
within both length limits, and each breaking the shared form; whichever of Tasks 050-4 and 013-8 merges first creates
the file, the other only adds entries); `a_cursor_with_a_wrong_field_is_malformed` reads its cases from that table. Tests:
`a_cursor_sealed_by_a_replica_after_a_rotation_opens_on_another`, `a_new_key_seals_only_after_two_timer_periods`,
`a_rotate_within_two_minutes_of_the_last_is_refused`, `an_unknown_key_id_is_a_malformed_cursor_without_a_read`,
`a_replica_with_no_key_loaded_is_unavailable`, `migrate_after_two_rotations_creates_no_key`.

*Cursor refusals.* Admission (the namespace, `run:read` and `approval:read`, or `definition:read` for the publication
inbox) runs before the cursor is decoded or opened, so a caller refused there never reaches the key cache
(`a_cursor_refusal_never_precedes_the_namespace_refusal`). A `cursor` parameter over 512 bytes is refused before decoding, and every cursor that does not open
(another caller's, a replaced key, tampered) gets the same `400` as a malformed one; the Studio then starts again from
the first page. Because the bound is counted before the folder predicate, a page
that is not full tells a folder-restricted caller how many hidden pending rows its window held, a count feature 029
allows aggregates to disclose (Requirement 029-017, its Gap 1442); no hidden row's identifier or content is
disclosed. The tests:
`a_full_page_resumes_after_its_last_item` (300 decidable rows inside the first 2,000, `limit=200`, all 300 found in
two requests, with a publication twin), `a_page_examines_at_most_two_thousand_rows_whatever_the_folders`, and
`a_cursor_reveals_nothing_and_opens_only_for_its_caller`; the publication inbox follows the same rules.

*Per-run list.* The run view does not use this route: the items of the existing
`GET /runs/{runId}/approvals` gain the same optional `youMayDecide`, which the use case computes as
`status == PENDING && !run_ended && may_decide(principal)`, because the per-run list returns settled requests and
requests of an ended run too (`ApprovalListItem`, a closed schema, gains the property; the flag concerns only the
caller and reveals no policy, initiator or decider); `may_decide` itself stays the decision's own check. Tests:
`a_settled_approval_is_not_flagged_decidable`, `a_pending_approval_of_an_ended_run_is_not_flagged_decidable`.

*Run ends.* A run-end path that the probe finds leaving approvals pending becomes a follow-up task of its own scope;
until it is fixed, the scan's run-status predicate keeps such rows out of the inbox.

*Index.* A migration adds `CREATE INDEX IF NOT EXISTS approval_requests_pending_deadline_idx ON approval_requests
(tenant_id, namespace_id, deadline, request_id) WHERE status = 'PENDING'`; the runbook tells an operator with a large
table to build it `CONCURRENTLY` before upgrading, as earlier migrations do, and to check `pg_index.indisvalid`
first, dropping and rebuilding an invalid index left by a failed concurrent build, which `IF NOT EXISTS` would
otherwise skip; its `down` drops it.

*Items.* `approvalId`, `runId`, `workflowId`, `workflowName`, `businessId`,
`nodeId`, `instancePath`, `status`, `deadline`, `openedAt` (the request's `created_at`), `youMayDecide`; `limit` 1 to
200, as the per-run list. No policy member, initiator, decider or
reason. Feature 026 adds its summary flag additively when it builds the summary.

**Server, publication requests.** `GET /namespaces/{namespaceId}/publication-requests?decidable=&cursor=&limit=`
lists the open requests that still need an approval under the namespace's current effective policy (`PENDING`, and
`APPROVED` with fewer approvals than the policy now requires, because publication recounts) and needs
`definition:read` (admission `ApiOperation::DefinitionRead` with `ApiTarget::Namespace`).

*Eligibility rule.* `crates/domain/src/versioning.rs` gains `PublicationRequest::may_decide(&Subject) -> bool` with the
open-state clause and the three clauses of Architecture decision 0018 decision 4 (not the requester, not the last
editor, not a person who already decided it), which `decide` calls; the use case adds `publication:approve`.

*Paging, port and index.* Order:
`requested_at`, then `id`, with the same `limit` (1 to 200), the same 2,000-row bound and the same cursor rule (the last item when the page
fills, the last row scanned when it does not); a port `PublicationInboxStore::open_page` returns raw rows in one query per page: each request with its decisions,
requester and last editor (for `may_decide`, never in the item) and the namespace's stored policy. The use case
resolves the policy once per page (it is namespace-wide) by a pure function over the stored policy and the
environment, extracted from `PublishingService::effective_policy` (`crates/application/src/publishing.rs`) and called
by publication's recount and by the inbox alike; the adapter
`crates/infra-postgres/src/versioning.rs::effective_policy`, which `DefinitionService::publish` reaches through
`publication_policy`, is changed to return the stored value and the environment and to call the same function, so
the resolution exists once; it then computes
`approvalsRequiredNow` and keeps the `APPROVED` requests with fewer approvals than that; a migration adds `publication_requests_open_requested_idx ON publication_requests
(tenant_id, namespace_id, requested_at, id) WHERE state IN ('PENDING', 'APPROVED')`, with its `down`.

*Items.* `requestId`, `workflowId`, `workflowName`, `kind`, `state`, `draftRevision`, `requestedBy`, `requestedAt`,
`approvals`, `approvalsRequiredNow` (what the current effective policy requires), `youMayDecide`. The item never
carries `decisions` or `editedBy`: the inbox says what waits and whether the caller may decide it, and who decided or
last edited stays on the workflow's own list, which discloses them under the same `definition:read`; the requester is
named because a person needs to know whose change they review, and the per-workflow list names them too. The contract
test `the_publication_inbox_item_has_exactly_its_fields` holds the list. The items of the existing
`GET /workflows/{id}/publication-requests`, which the versions page reads, gain the same `youMayDecide`
(`PublicationRequest` gains the property).

**Metrics.** `approval_inbox_requests_total{outcome}` and `publication_inbox_requests_total{outcome}` (`outcome` in
`ok`, `denied`, `invalid`, `unavailable`), `inbox_rows_scanned{list}` (a histogram of the rows a page examined, `list` in `approvals`, `publications`),
`inbox_cursor_key_reads_total{outcome}` (`ok`, `failed`) and the gauge `inbox_cursor_keys_last_read_age_seconds`; no
tenant, namespace, workflow or subject label.

**Studio.** `features/inbox/`, its refresh built on the Studio's existing pattern (TanStack Query's
`refetchInterval` with a function, as `BatchPage.tsx` uses; a `400` on a later page ends that refresh, which shows what it collected with "more may wait", and the next
refresh starts from the first page as every refresh does; which skips a hidden browser tab, with the back-off as that
function; `RunListStore` of `features/runs/runList.ts` shows the same rules): two tabs, approvals (deadline ascending, "time left" in words and as a number) and
publication requests (oldest first), each read with `decidable=true` through `collectPagesBounded` (at most five
pages per list per refresh), saying "more may wait" when it stopped with a cursor left. Every refresh starts from the
first page and reads only the inbox tab in view, and only while the browser tab is visible. The interval is 30 seconds and
doubles, up to four minutes, after each refresh that returns the same items in the same states; it returns to 30
seconds when the list changes, when the person decides or asks to refresh, and when the browser tab becomes visible
again. The time of the last refresh is shown. A refresh that fails (a `503`, such as
`INBOX_CURSOR_KEYS_UNAVAILABLE`, or a network error) keeps the last list, says "could not refresh, temporarily
unavailable" with the time of the last good refresh, and doubles the interval as an unchanged list does, waiting at
least the `Retry-After` it was given. A `401`, `403` or `404` is not a failed refresh: it clears the list, so the
page never keeps showing items the person can no longer read. Deciding uses the existing operations; an item that settled meanwhile
shows its new status instead of an error.

The load this puts on a tenant: one open inbox sends at most five requests per refresh, so at the base interval at most
one request every six seconds, and at the longest interval one every 48 seconds. 300 open inboxes of one tenant whose
lists change at every refresh send at most 50 requests per second, half of the default tenant budget of 100 per
second (`crates/api-http/src/limits.rs`); with unchanged lists they settle at about 6.3 per second; a list that fits
in its first page sends one request per refresh, a fifth of these figures. Task 050-14 measures the figure for 300 inboxes; a request refused with
`429` waits for `Retry-After` (`collectPagesBounded`).

### 7. The audit page

`features/audit/` at the tenant level (beside `namespaces`), shown in the navigation only with `audit:read`. Its
filters live in component state, never in the address, so actor subjects and resource identifiers stay out of the
browser history and of shared links; the download and the journal download are labelled as holding personal data, and the
server records reads of the audit trail in `audit.read`, one record per caller and filter per five minutes (the
action collapses repeats), so a download of many pages leaves about one record. `path()` in
`src/api/client.ts` refuses a segment equal to `.` or `..`, which percent-encoding leaves as it is, so a business
identifier or a resource identifier can never move a request to another route. Filters:
action (a picker that suggests the known actions, without `access.denied.cross_tenant`, which `GET /audit` never
returns, and accepts any text), resource type and id, actor, from and to. The table shows time, actor, action,
resource and reason; a record view adds the trace link (the existing `TraceLink`). A resource link is made for the
types that have a page (run, workflow, schedule, script, connection, namespace). The download reads pages until 10,000
records or the end and saves JSON lines, saying which. The known actions are in `frontend/apps/studio/src/api/audit-actions.json`, which the picker imports (Decision
050-006). `crates/domain/src/audit.rs` builds `AuditAction`, its codes and `AuditAction::ALL` from one table with a
macro, as `crates/domain/src/database/failure.rs` does with `failures!`, so a variant cannot exist outside `ALL`. The
contract test `the_known_audit_actions_equal_the_domain_actions` in `contract_rest_workflow_studio` compares the JSON
file with `ALL`; `SAGAT_CONTRACT_BASELINE_BLESS=1` rewrites the JSON after the diff is read. The server already records each reading
(`audit.read`, collapsing within five minutes).

### 8. Versions, settings, events, scripts, palette

- **Versions** (grown from `features/workflows/VersionsPanel.tsx`, which already lists, shows and compares versions,
  into a page at route `workflows/:workflowId/versions`; design 044's Task 044-23 adds its column of open runs per code
  digest): versions list, activation history
  (`GET /workflows/{id}/activations`), activate or roll back (`PUT /workflows/{id}/active-version` with reason and the
  version seen; a `409 BREAKING_CHANGE` asks the person to confirm and resends with `acceptBreaking`, and
  `ACTIVE_VERSION_CHANGED` re-reads the history), promote (`POST .../versions/{n}/promote`, destination picker, missing names shown), publication
  requests of the workflow, compensation plan (`GET .../versions/{n}/compensation-plans`) drawn as the reverse-order
  list with `after` dependencies.
- **Settings** (`features/settings/`, route `settings`): publication policy (get, set with a reason), the namespace's
  quotas and activity quotas and the tenant's quotas (`/quotas`, including `iteration_concurrency`, without which a
  collection admits nothing; read, and set where the person holds `quota:manage`), worker bindings (list, bind, unbind), payload
  reveal grants (list, grant, revoke), and on the secret page the access list.
- **Events** (`features/events/`): event endpoints, webhook and broker subscriptions with their deliveries, connector
  dead letters, redrive of one item or of a subscription's failures, each confirmed and its receipt shown. Feature 021
  later adds resources and triggers next to them.
- **Scripts**: `features/scripts/languages.ts` marks Go and Java `built: true`; a `LANGUAGE_NOT_BUILT` answer of the
  create-script route is shown at the language picker of `CreateScriptDialog.tsx`.
- **Palette** (`src/app/CommandPalette.tsx`): opened by Ctrl or Command and K; searches the lists the person may read
  (workflows, scripts, schedules, connections by name prefix; a run by identifier or business identifier through
  `GET /namespaces/{namespaceId}/business-ids/{businessId}`), and the Studio's pages; never a route outside the
  person's permissions (`GET /auth/context`).

### 9. Contracts and documents

- `specs/001-platform-foundation/contracts/openapi.yaml`: `PlatformVersion.capabilities` (strings with the pattern
  and `x-known-values`); the three new operations with their schemas and errors, marked `x-planned: '050'` (each task that adds its route
  switches it to `x-implemented: '050'`, as the repository does, because the cross-tenant battery attacks every
  implemented operation); `frontend/apps/studio/src/api/schema.d.ts`
  regenerated. Two pinned digests change and are renewed after their diff is read: the operation baseline of
  `contract_iceberg_writes` (`SAGAT_CONTRACT_BASELINE_BLESS=1 cargo test -p sagat-tests --test contract_iceberg_writes`)
  and the byte-stability digest of `contract_payload_classification` (recomputed in its `openapi.rs`).
- `docs/engineering/architecture-principles.md`: four rows with their checks (the registry against the schema; the
  capability names against `AdvertisedCapability`; `may_decide` against `decide` for approvals
  and publication requests; `AuditAction::ALL` against the Studio's list).
- `docs/runbooks/studio.md`: the inbox (its cursor key and the yearly `control-plane inbox-cursor-key rotate`), the audit page, the capability list and how an operator reads it.
- `specs/007-workflow-studio/design/README.md`: the new screens.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The inbox adds a disclosure | It adds none beyond what the caller can already read: an approval's policy is in the workflow version (`definition:read`), and the initiator and deciders are actors of the journal (`run:read`); the inbox returns only the caller's own eligibility, and a contract test holds its item to its field list (Architecture decision 0144) |
| The inbox scan grows with pending approvals | A partial index in deadline order; each page examines at most 2,000 rows before any predicate, with a cursor sealed under a key derived per cursor (a random salt, the tenant in `info`) from the cursor key; the performance journey includes a caller who sees one folder of many; the count of hidden rows per window is the one aggregate disclosed (Requirement 029-017) |
| A legacy node is lost on save | The canonical form keeps unknown members; a test saves and reloads one |
| A capability shown by one replica is refused by another | The server's validation decides; the problem appears at validation (Gap 050-001) |
| Parallel Studio branches rewrite the same files | The order of Technical Plan 1; new primitives in new files |
| The bundle grows with every screen | The bundle budget test of Decision 050-001; pages load on demand as today |
| A run with 10,000 iterations freezes the page | Counts on the owner, a paged iteration list read for the rows in view, a virtualized strip; the graph draws one iteration at a time |
| The runtime graph of a large collection is too large to download and parse | Measured by the performance journey (Non-functional requirement 050-002); a graph read by scope is the follow-up if the figure exceeds the budget (Gap 050-006) |
| Open inboxes use the tenant's request budget and starve its workers | Only the tab in view refreshes, only while visible, with a doubling interval; the load for 300 inboxes is estimated in Technical Plan 6 and measured by Task 050-14 |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No new state transition: decisions, updates, activations and redrives use the existing operations and their transactions; the new routes only read |
| II. Modular domain, ports and adapters | Pass | The eligibility rules stay in the domain (`ApprovalRequest::may_decide`, `PublicationRequest::may_decide`) and their decision paths call them; the use cases in `application` filter with them; the adapter only reads bounded pages; the capability names are a typed enumeration in `application`; no new dependency edge |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect is added |
| IV. Compatibility is versioned and pinned | Pass | Additive REST fields and routes; the Studio draws the IR and keeps nodes it cannot run; no IR change |
| V. Code, tenant boundary and secrets are isolated | Pass | The three new routes take the tenant from the principal and answer another tenant's namespace as missing (battery); nothing of a policy, a payload or a secret leaves; no token in a URL or storage |
| VI. Observability and audit are part of the feature | Pass | Spans and closed-label metrics on the new routes; decisions and changes keep their existing audit; audit reading is audited |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | No message is added; redrive uses feature 004's routes |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Property test of eligibility, contract tests of the capability list and the inbox schema, journeys with real PostgreSQL; nothing claimed before it runs |
| Delivery Constraints | Pass | One new frontend dependency only through pull request #267 (Astryx, accepted with conditions); no new server technology |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A capability list on `GET /version` | The palette must follow the server it talks to, and three other features need the same advertisement | A field per feature; the Studio's constant list (how it drifted) |
| Two list routes | A person cannot find what waits for them without them | The Studio asks every run (one request per run, no eligibility) |
| A sealed inbox cursor | The cursor may name a row in a hidden folder, and the 2,000-row bound must count rows before the access predicates | An unsealed keyset cursor (names hidden rows); a bound after the predicates (an unbounded scan) |
| The table `inbox_cursor_keys` and its `rotate` command | Sealing needs a key that every replica shares and that works on a deployment without OpenBao or `SAGAT_SECRETS_KEYS` | A key from the secrets ring (couples the inbox to an optional component); a key per process (cursors would not open on another replica) |
| A paged iteration list | Per-iteration outcomes on 10,000 scoped instances would go stale under `graph/changes` and grow every node | A status on each scoped instance (Decision 050-012) |
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
with its environment (050-002); the existing contract baselines (with the two digests renewed after reading their diff) and the Studio suites
unchanged (050-003); the metric label test of the new routes (050-004).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Documents first (constitution, Documentation first): Task 050-2 lands before the code that serves the new fields. Tests
are seen failing first; a test listed under "Guard" passes today and protects behaviour that must not regress. A task
line writes its identifier as `T050-n`, the form the design-doc check reads; prose writes "Task 050-n". Studio tests are
`it()` titles in prose in the named file; the names below are their identifiers.

**Builders and gates.** The server builder owns `crates/`, `apps/`, `tests/` (every Rust target, the harness changes of
the journeys and the performance driver), the OpenAPI document, the SDK descriptions and unit tests under `sdks/`, and
the documents of Task 050-2. The Studio builder owns `frontend/apps/studio/` and `frontend/e2e/`, and regenerates
`schema.d.ts` after Task 050-2. Gates outside this design: every Studio task waits for pull request #267 (Astryx) to
merge; Task 050-17 waits for feature 041's Logs tab; Task 050-15 moves feature 022's registry entries to `capability`
and `scope` when 022's branch has merged, or 022 does it when it merges second; Tasks 050-4 and 050-5 add feature 029's
`x-access` and `visibleFolders` filter when 029's slice 0 has merged, or 029 adds them when it merges second. A new
primitive goes in a new file of `src/ui/`.

- [ ] T050-1 Studio builder, after pull request #267: accept Architecture decision 0101 with its two conditions (its
  status set in #267, or a follow-up on `develop`, to accepted under OD-2026-11-24-700); the Overview's link to it
  becomes the relative `../adr/` link once #267 has merged; the bundle budget test in
  `frontend/apps/studio` with the figures measured on the merged branch; the performance journey recorded in
  `specs/007-workflow-studio/quickstart.md`.
  Verification: `npm test -- bundle-budget` fails on an entry script 11% larger; the quickstart names the result and
  the environment.
  Refs: Requirement 050-022
- [ ] T050-2 Server builder: the contracts and documents of Technical Plan 9 (`x-planned: '050'` on the three new
  operations, each added to `not_built` of `tests/contract/security-multitenancy/rest/battery.rs`; `youMayDecide` on `ApprovalListItem` and `PublicationRequest`; `RuntimeNode.iterations`;
  `PlatformVersion.capabilities`; the `503` of the two inbox operations with the reason
  `INBOX_CURSOR_KEYS_UNAVAILABLE` and `Retry-After`), `frontend/apps/studio/src/api/audit-actions.json`, the
  SDK type descriptions of the per-run approvals list, the two digests, the four rows of
  `docs/engineering/architecture-principles.md`, `docs/runbooks/studio.md` with the publishable subset of Technical
  Plan 4, the screen list in `specs/007-workflow-studio/design/README.md`; then the Studio builder regenerates
  `schema.d.ts`.
  Verification: `npx --yes @redocly/cli@1 lint specs/001-platform-foundation/contracts/openapi.yaml`;
  `cargo test -p sagat-tests --test contract_iceberg_writes --test contract_payload_classification`;
  `(cd frontend/apps/studio && npm run check:api)`; the docs checks.
  Refs: Requirement 050-002, Requirement 050-010, Requirement 050-011, Requirement 050-013
- [ ] T050-3 Server builder: `AdvertisedCapability` (its two names) and `advertised_capabilities` in
  `crates/application/src/capabilities.rs`; the `AppState` field and `platform_version` on both routers; the control
  plane and the harness call the function.
  Red tests: `capabilities_follow_the_switches_a_server_was_started_with` and `a_reader_ignores_an_unknown_capability`
  in `tests/contract/workflow-studio/rest/capabilities.rs`; `every_advertised_capability_is_a_known_value` (the variants
  equal the OpenAPI `x-known-values`).
  Verification: `cargo test -p sagat-application capabilities`;
  `cargo test -p sagat-tests --test contract_rest_workflow_studio capabilities`.
  Refs: Requirement 050-002
- [ ] T050-4 Server builder: first the probe `every_run_end_cancels_its_pending_approvals`; then
  `ApprovalRequest::may_decide`, called by `decide` (`crates/domain/src/execution/approval.rs`); the port
  `InboxCursorSealer`, its adapter `crates/infra-postgres/src/inbox_cursor.rs`, the table `inbox_cursor_keys` in the
  migration with its entries in `crates/infra-postgres/storage-classification.json`, its first key created by
  `sagat_infra_postgres::migrate` and `control-plane inbox-cursor-key rotate --operator`, both with the schema-owning
  role under the table lock, the runtime role granted `SELECT` only; `aes-gcm` and `hkdf` 0.13 moved to (or added to) the
  workspace's dependencies, `aes-gcm`, `hkdf` and `getrandom` 0.4 added to `infra-postgres`; the table
  `tests/fixtures/sealed_envelope_malformed.json`; `ApprovalInboxStore`, its use case and adapter, the migration with the
  partial index and its `down`; the route, its metrics and `x-implemented`; `youMayDecide` on the per-run list through
  `list_for_run`; the adapter's ended-run guard moved after authorization; the route removed from `not_built`; the new
  target `integration_studio_catch_up` (`tests/integration/studio-catch-up/main.rs` and its `[[test]]` entry in
  `tests/Cargo.toml`).
  Red tests: `may_decide_equals_decide` (property, `crates/domain`); in `integration_studio_catch_up`:
  `every_run_end_cancels_its_pending_approvals` (the probe, first: a failing path opens a follow-up task),
  `an_ended_run_is_left_out_of_the_inbox` (the inconsistent row seeded directly), `a_full_page_resumes_after_its_last_item`,
  `a_settled_approval_is_not_flagged_decidable`, `a_pending_approval_of_an_ended_run_is_not_flagged_decidable`,
  `one_decidable_row_after_2001_undecidable_is_found_in_two_requests`, `a_page_stops_at_the_scan_bound_and_continues`,
  `an_inbox_page_is_one_statement`, `a_page_examines_at_most_two_thousand_rows_whatever_the_folders` (2,000 pending rows of ended runs, seeded, ahead of
  one live row: the first page is empty with a cursor, the second returns the live row),
  `a_cursor_reveals_nothing_and_opens_only_for_its_caller`,
  `an_unopenable_cursor_is_a_malformed_cursor`, `a_cursor_sealed_under_a_dropped_key_is_a_malformed_cursor` (two rotations back),
  `a_cursor_sealed_by_a_replica_after_a_rotation_opens_on_another`, `a_new_key_seals_only_after_two_timer_periods`, `a_cursor_with_a_wrong_field_is_malformed`,
  `a_rotate_within_two_minutes_of_the_last_is_refused`, `an_unknown_key_id_is_a_malformed_cursor_without_a_read` (an
  overflowing key id included), `a_replica_with_no_key_loaded_is_unavailable`, `migrate_after_two_rotations_creates_no_key`,
  `two_concurrent_rotations_leave_the_two_newest_keys`, `a_cursor_refusal_never_precedes_the_namespace_refusal`,
  `an_approval_cursor_does_not_open_on_the_publication_list`, `a_failed_key_read_is_counted_and_logged_once`, `the_runtime_role_cannot_write_the_cursor_key`, `the_cursor_key_is_created_by_migrate_and_never_printed`,
  `an_oversized_cursor_is_refused_before_decoding`, `an_ineligible_caller_on_an_ended_run_is_refused_for_eligibility` (the inconsistent
  row seeded directly), `an_eligible_caller_on_an_ended_run_still_gets_503`, `the_per_run_list_flags_the_callers_own_eligibility_only`; in `contract_rest_workflow_studio`: `the_inbox_never_names_a_policy_member`
  (`rest/approvals_inbox.rs`); the battery entry; `the_runtime_role_can_read_the_inbox`.
  Verification: `cargo test -p sagat-domain approval`;
  `cargo test -p sagat-tests --test integration_studio_catch_up --test contract_rest_workflow_studio`, and again with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`; the battery target.
  Refs: Requirement 050-009, Requirement 050-010
- [ ] T050-5 Server builder: `PublicationRequest::may_decide`, called by `decide` (`crates/domain/src/versioning.rs`);
  `PublicationInboxStore`, the use case with `publication:approve`, the reduced item, order, cursor, bound and partial
  index; the route and `x-implemented`; `youMayDecide` on `GET /workflows/{id}/publication-requests`.
  Red tests: `publication_may_decide_equals_decide` (property, `crates/domain`); in `integration_studio_catch_up`:
  `the_requester_and_the_last_editor_may_not_decide`, `a_person_who_decided_may_not_decide_again`,
  `a_policy_raised_after_an_approval_lists_the_request_for_another_approver`, `an_approver_may_decide`,
  `paging_holds_across_a_supersede`, `a_publication_inbox_page_is_one_statement`, the publication twins of
  `a_full_page_resumes_after_its_last_item` (300 decidable open requests inside the first 2,000, `limit=200`),
  `a_page_stops_at_the_scan_bound_and_continues` and
  `one_decidable_row_after_2001_undecidable_is_found_in_two_requests`, the publication twin of
  `a_cursor_reveals_nothing_and_opens_only_for_its_caller`,
  `an_approval_cursor_does_not_open_on_the_publication_list`, and the publication twins of
  `an_oversized_cursor_is_refused_before_decoding` and `an_unopenable_cursor_is_a_malformed_cursor`;
  `the_publication_inbox_item_has_exactly_its_fields` (`rest/approvals_inbox.rs`); the battery entry, with the route
  removed from `not_built`.
  Verification: `cargo test -p sagat-domain versioning`;
  `cargo test -p sagat-tests --test integration_studio_catch_up --test contract_rest_workflow_studio`, and again with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
  Refs: Requirement 050-011
- [ ] T050-6 Server builder: the body scope on the `IrError` variants of Technical Plan 4 and the
  `/bodies/{name}/nodes/{index}` pointer in `problems_of`.
  Red tests: `a_body_problem_names_its_body_and_index`, `a_body_problem_keeps_its_node_id` and
  `a_body_without_its_return_is_placed_in_the_body_even_when_its_entrypoint_id_is_a_root_id` (`crates/application`
  unit tests of `definitions.rs`).
  Verification: `cargo test -p sagat-application definitions`; `cargo test -p sagat-infra-ir`.
  Refs: Requirement 050-003
- [ ] T050-15 Studio builder, after Task 050-6: modify `model/registry.ts`, `features/workflows/AddNode.tsx`
  (`Palette`, `AddNodeDialog`), `NodeInspector.tsx` and `NodesTable.tsx` (which read today's `runs` flag), create
  `src/api/capabilities.ts`: the registry (`requires`, scope, nested handles, new edge kinds, the
  new kinds of Technical Plan 4), the schema drift test, `useCapabilities`, the palette filtered by capability, legacy
  and disabled kinds read-only, `documentIO.ts` keeping every root entry at its index, `resolveNodeId` reading body
  pointers first.
  Red tests: `without_the_operators_capability_the_stage_list_is_read_only_and_kept`
  (`src/features/workflows/inspectors/ForEach.test.tsx`); `registry_matches_the_schema` and
  `a_schema_with_a_new_kind_fails_the_registry_test`
  (`src/model/registry.test.ts`); `a_missing_capability_hides_the_lake_write` (`src/features/workflows/Palette.test.tsx`);
  `an_unreadable_entry_between_two_nodes_keeps_its_index` (`src/features/workflows/documentIO.test.ts`, fails today);
  `a_body_problem_is_placed_in_its_body_even_with_a_colliding_root_id` (`validationModel.test.ts`).
  Verification: `(cd frontend/apps/studio && npm test -- registry documentIO validationModel Palette && npm run lint && npm run typecheck)`.
  Refs: Requirement 050-001, Requirement 050-003, Requirement 050-004
- [ ] T050-16 Studio builder, after Task 050-15: the body-document adapter `src/model/bodies.ts` (read a body as a
  graph with its `entrypoint`, apply the edit operations of `edit.ts` to it, write it back in place).
  Red tests: `an_edit_inside_a_body_changes_only_that_body`, `a_body_entrypoint_moves_with_its_node`,
  `an_unreadable_entry_in_a_body_keeps_its_index` (`src/model/bodies.test.ts`).
  Guard: `a_duplicate_id_entry_in_a_body_keeps_its_index` (passes today, since bodies are carried untouched).
  Verification: `(cd frontend/apps/studio && npm test -- bodies)`.
  Refs: Requirement 050-004, Requirement 050-007
- [ ] T050-7 Studio builder, after Task 050-15: the inspectors of `switch`, `approval`, `forEach` (with the stage
  list), `while`, `repeatUntil`, `return`, `continueAsNew`, `icebergWrite`; the workflow settings for `state`,
  `limits` and `chainPolicy` (and the `inputSchema` editor's problems by pointer); server problems at their fields; the
  read-only inspector of legacy nodes with its "add here" action.
  Red tests: one component test per kind and setting in `src/features/workflows/inspectors/*.test.tsx`, each building
  the node, filling it and showing a mocked `validate` problem at its field; `a_legacy_inspector_shows_the_old_values`;
  `add_here_moves_every_incoming_link_to_the_new_node` (two predecessors, and an entrypoint). Publication of each kind
  in a publishable shape is a journey case of Task 050-13.
  Verification: `(cd frontend/apps/studio && npm test -- inspectors)`.
  Refs: Requirement 050-005, Requirement 050-006
- [ ] T050-8 Studio builder, after Tasks 050-16 and 050-7: the body canvas (bodies panel, breadcrumb, body palette,
  exactly one `return`, shared-body confirmation).
  Red tests: `a_body_without_its_return_shows_the_problem`, `a_shared_body_asks_once_before_a_change`
  (`src/features/workflows/BodyCanvas.test.tsx`).
  Verification: `(cd frontend/apps/studio && npm test -- BodyCanvas)`.
  Refs: Requirement 050-007
- [ ] T050-9 Server builder, read-only: the counts in `GraphSnapshot` and on owner nodes, from the progress tables
  and their drain columns; `RunStore::iteration_page`, the use
  case, the route and `x-implemented`, its metric and battery entry.
  Red tests in `integration_studio_catch_up`: `the_counts_and_the_listed_outcomes_equal_the_iteration_events`,
  `a_failed_iteration_drains_the_others_and_counts_them_drained` (also asserting the counts while the drain is held
  open by a barrier),
  `graph_changes_across_a_retry_a_return_and_a_drain_equals_a_fresh_graph`, `the_iteration_list_pages_by_index`,
  `an_outcome_filter_keeps_the_keyset`, `an_unknown_owner_answers_an_empty_page`,
  `a_query_refusal_never_precedes_the_run_refusal`; the battery entry with the query string the route needs, the
  route removed from `not_built`.
  Verification: `cargo test -p sagat-tests --test integration_studio_catch_up iteration`, and again with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
  Refs: Requirement 050-008
- [ ] T050-17 Studio builder, after Tasks 050-9 and 050-4 and feature 041's Logs tab: the run view (iterations
  grouped and drawn with `useWindowedRows` over the paged list, the switch selection, the approvals panel and decision
  with `useIntentKey`, the Queries and Updates panel with `JsonField`, the family, the journal download with
  `collectPagesBounded`).
  Red tests (`src/features/runs/*.test.tsx`): `iteration_312_of_500_shows_its_steps`,
  `an_iteration_with_a_handled_failure_counts_as_succeeded`, `compensations_are_drawn_beside_their_step_and_not_counted`,
  `an_unknown_switch_selection_says_so`, `a_resent_decision_reuses_its_key_and_a_changed_one_does_not`,
  `a_corrected_update_after_a_stale_revision_succeeds`, `a_second_tab_gets_a_revision_conflict`,
  `a_403_after_a_sent_decision_rereads_the_status`, `the_download_lines_equal_the_page_items`,
  `the_download_stops_at_its_bound_and_says_so`, `collect_pages_bounded_waits_for_retry_after`.
  Verification: `(cd frontend/apps/studio && npm test -- runs)`.
  Refs: Requirement 050-008, Requirement 050-009, Requirement 050-014, Requirement 050-015, Requirement 050-021
- [ ] T050-10 Studio builder, after Tasks 050-4 and 050-5: the inbox page (`src/features/inbox/`).
  Red tests (`src/features/inbox/InboxPage.test.tsx`): `only_decidable_items_are_under_waiting_for_me`,
  `an_empty_first_page_with_a_cursor_is_followed`, `each_refresh_starts_from_the_first_page`,
  `a_settled_item_shows_its_status_not_an_error`, `the_inbox_refreshes_only_while_visible`,
  `a_bad_cursor_ends_the_refresh_and_the_next_starts_from_the_first_page`,
  `only_the_tab_in_view_is_refreshed`, `an_unchanged_list_doubles_the_interval_up_to_four_minutes`,
  `a_route_not_found_says_the_page_is_not_available`, `an_unavailable_inbox_keeps_the_last_list_and_backs_off`,
  `a_lost_permission_clears_the_inbox`.
  Verification: `(cd frontend/apps/studio && npm test -- inbox)`.
  Refs: Requirement 050-012
- [ ] T050-11 Server builder: `AuditAction` and `AuditAction::ALL` built from one table (`crates/domain/src/audit.rs`);
  `the_known_audit_actions_equal_the_domain_actions` in `contract_rest_workflow_studio`.
  Verification: `cargo test -p sagat-domain audit`;
  `cargo test -p sagat-tests --test contract_rest_workflow_studio audit`.
  Refs: Requirement 050-013
- [ ] T050-18 Studio builder, after Task 050-11: the audit page (`src/features/audit/`).
  Red tests (`src/features/audit/AuditPage.test.tsx`): `the_filters_never_reach_the_address`,
  `a_path_segment_of_dot_or_dot_dot_is_refused` (`src/api/client.test.ts`), `the_download_stops_at_ten_thousand_and_says_so`,
  `the_page_is_absent_without_audit_read`, `the_picker_suggests_the_known_actions_and_accepts_any_text`.
  Verification: `(cd frontend/apps/studio && npm test -- audit)`.
  Refs: Requirement 050-013
- [ ] T050-12 Studio builder, after Task 050-5: the versions page, the settings page, the events page, Go and Java
  marked built in `languages.ts` with a Go and a Java sample in `SAMPLE_SOURCE` (taken from `runners/go/README.md` and
  the Java runner's), the command palette.
  Red tests: one component test per section with each role; `go_and_java_are_built_with_their_own_samples`;
  `a_language_not_built_answer_of_create_is_shown_at_the_picker`; `the_palette_never_calls_a_route_the_person_cannot_call`.
  Verification: `(cd frontend/apps/studio && npm test -- versions settings events scripts CommandPalette)`.
  Refs: Requirement 050-016, Requirement 050-017, Requirement 050-018, Requirement 050-019, Requirement 050-020
- [ ] T050-19 Server builder, before Task 050-13: the journey harness. `tests/e2e/workflow-studio/studio_journeys.rs`
  starts the server with `Options.collection_operators` and the engine with the matching environment, and sets the
  tenant's collection capacity through `PUT /quotas/iteration_concurrency` (a collection admits no iteration while it
  is unset); the job already declares 14 journeys of up to 240 seconds each (56 minutes) against its 60-minute cap
  (`studio_journeys.rs`), so the new journeys carry the title prefix `@catch-up` and run as a second job selected with
  `SAGAT_STUDIO_GREP=@catch-up`, each within the same 240 seconds, while the existing job excludes them with a new
  `SAGAT_STUDIO_GREP_INVERT=@catch-up` that the harness passes as `--grep-invert`.
  Verification: `npx --prefix frontend/e2e playwright test --list --grep-invert @catch-up` lists exactly the existing
  14 journeys; `cargo test -p sagat-tests --test e2e_workflow_studio --locked -- --ignored journeys` runs them
  unchanged.
  Refs: Success criterion 050-001
- [ ] T050-13 Studio builder, after the tasks whose journeys it runs and Task 050-19: journeys in `frontend/e2e/tests/`
  driven by `e2e_workflow_studio`: the journey of Success criterion 050-001, the journeys named by the requirements,
  and one publication per kind in a publishable shape; axe in three themes and reflow at 320 pixels on every new page.
  Verification: `cargo test -p sagat-tests --test e2e_workflow_studio --locked -- --ignored journeys`, with its result
  recorded.
  Refs: Requirement 050-006, Requirement 050-009, Requirement 050-012, Requirement 050-014, Requirement 050-016, Requirement 050-018, Requirement 050-022
- [ ] T050-14 Server builder, with the Studio builder's browser part: the performance journey. A new target
  `perf_studio_catch_up` (`tests/performance/studio-catch-up/main.rs`) seeds 10,000 pending approvals and a run of
  10,000 iterations of an eight-step body through the test support as the schema owner, times the inbox and audit
  pages (with, once feature 029's folders exist, a caller who sees one folder of many), and measures the inbox routes' latency at a fixed rate of 50 requests per second from one tenant, the worst
  case Technical Plan 6 bounds for 300 open inboxes, against the tenant budget (no copy of the Studio's schedule); the existing `frontend/e2e/tests/perf.spec.ts`, which already benchmarks a run
  of about 10,000 nodes when `STUDIO_PERF=1`, is extended with the graph's size and parse time and the inbox page. Both
  report to `tests/performance/reports/workflow-studio.md` through `frontend/e2e/scripts/perf-report.mjs`, which gains
  a second input (the Rust target's `perf_studio_catch_up.json`) and the sections for it, the report Decision 050-001's
  condition also reads.
  Verification: `cargo test -p sagat-tests --test perf_studio_catch_up --locked -- --ignored --nocapture`;
  `STUDIO_PERF=1 npm --prefix frontend/e2e test -- perf.spec.ts`.
  Refs: Non-functional requirement 050-002

#### Follow-up tasks

- [ ] T050-20 Server builder, with Task 050-4 (OD-2026-11-26-900, OD-2026-11-26-901): `rotate` checks the age of the
  newest key after taking the table lock, in READ COMMITTED, and the refusal reads the database's `now() -
  created_at`; a new key's `created_at` is `clock_timestamp()` taken after the lock; the rotation tests backdate
  `created_at` as the schema-owning role and never sleep; the concurrent test becomes
  `two_concurrent_rotations_insert_one_key_and_refuse_the_other` (one success, one refusal, keys K1 and K2); the
  `INSERT`, `DELETE` and `LOCK TABLE` on `inbox_cursor_keys` in `migrate` and `rotate` get `ALLOWED` entries with
  their reasons in `tests/integration/platform-foundation/runtime_privileges.rs`, as the trigger handover has, with
  `the_runtime_role_cannot_write_the_cursor_key` as the proof.
  Verification: `cargo test -p sagat-tests --test integration_studio_catch_up rotat`; the runtime privilege target.
  Refs: Requirement 050-009, Requirement 050-010
- [ ] T050-21 Server builder, with Task 050-9 (OD-2026-11-26-901): the iteration tests live in the module
  `tests/integration/studio-catch-up/iteration.rs`, so the verification filter `iteration` selects all seven red tests.
  Refs: Requirement 050-008
- [ ] T050-22 Studio builder, with Task 050-10 (OD-2026-11-26-900): the refresh follows Technical Plan 6 "Studio"
  read with its `400` clause after the parenthesis: the refresh is TanStack Query's `refetchInterval` with a function,
  which skips a hidden browser tab and carries the back-off; a `400` on a later page ends that refresh, which shows
  what it collected with "more may wait", and the next refresh starts from the first page.
  Refs: Requirement 050-012

### Verification Plan

- Unit: the registry drift test; the inspectors; the run graph grouping; the inbox, audit, versions, settings, events
  and palette components (`npm test`).
- Property: `may_decide_equals_decide` and `publication_may_decide_equals_decide` in `crates/domain`.
- Integration (real PostgreSQL): the three new routes, the inbox scan bound, their runtime-role privileges
  (`SAGAT_TEST_AS_RUNTIME_ROLE=1`), the iteration counts and list, in the new target `integration_studio_catch_up`
  (Task 050-4); the capabilities derived from the start-up switches in `contract_rest_workflow_studio`.
- Contract: the OpenAPI lint, the capability list against the switches, the inbox schema without policy fields, the
  cross-tenant battery, the audit action list.
- Journeys: `e2e_workflow_studio` with the new journeys; axe in light, dark and forced colours.
- Performance: `perf_studio_catch_up` and `perf.spec.ts` (Task 050-14), reported with their environment, never as a
  promise.
- Gates: `npm run lint`, `typecheck`, `test`, `build`, `check:api`; `python3 scripts/check-architecture.py`; the
  design-doc and docs checks.
