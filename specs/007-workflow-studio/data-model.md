# Data Model: Visual Studio, Monaco and runtime graph

## Context

The Studio adds **no table, no column and no migration**. Everything it shows is read from the platform (the
definitions of 001/002, the journal and the runtime graph of 002/003, the scripts of 005), and everything it
writes goes through the operations that already existed (`PUT /workflows/{id}`, `publish`, `operations`, the scripts
API). What this feature defines is the **wire contract of four new read operations** and **what the browser holds
in memory**. Behaviour and decisions: [`research.md`](./research.md) (D138–D150), ADR-0016.

## The platform's side

| Operation | Reads | Guarantees |
|---|---|---|
| `POST /workflows/{id}/validate` | the document in the body, or the stored draft | The same checks as publish (schema, graph rules, expressions, name, secrets the caller may use, script artifacts built for the tenant), answered as a list; nothing stored, nothing audited. Needs `definition:write`. |
| `GET /workflows/{id}/diff?from=&to=` | two documents (`draft` or a version number) | Compared as written; sorted, so the same two documents give the same answer; `breaking` and `attention` as defined in D140. Needs `definition:read`. |
| `GET /runs/{id}/graph/changes?since=N` | the run's graph, the journal after `N` | The events after `N` (≤ the graph's sequence) and the full objects of the nodes they name; `reset` with the whole graph when `N` is ahead or more than 500 behind. |
| `GET /runs/{id}/stream[?after=N]` | the same, pushed | `text/event-stream`: `graph` events (id = sequence), `: ping` every 15 s, `end`, `reconnect`; resumable with `Last-Event-ID`; closed at the token's expiry and after 15 minutes. At most 20 per subject and 200 per tenant. |

Invariants (tested in `tests/e2e/007-workflow-studio`):

- A node changes only through a journal event that names it or activates it (`RunEvent::changed_nodes`); therefore **applying
  every delta in order gives exactly the graph the platform would send whole** (`us03_applying_every_change_in_order…`, and
  `us03_the_deltas_still_equal_the_whole_graph_when_a_plan_activates_steps_that_wait_their_turn` for steps that wait their turn).
- A delta never contains an event with a sequence above the graph it ships with.
- Another tenant's workflow or run is `404` for all four, and a refusal never reveals whether it exists.
- Nothing returned holds a payload, an input, an output or a secret: the journal carries a digest, the graph a
  failure *code*.

## The browser's side (in memory, none of it persisted by the platform)

| Value | Fields | Lifetime and rule |
|---|---|---|
| **Session** | bearer token, `AuthContext` (tenant, subject, role, permissions) | The token only in memory (D144); ends at its `exp`, on sign-out, or when the platform answers 401; clears every cached query. |
| **Namespace** | a slug | Chosen in the header; may be remembered (it is not a secret); sent as `X-Octopus-Namespace`. |
| **Document** | the IR as the platform stores it | Edited immutably (`src/model/edit.ts`); what the Studio does not understand is kept; nodes it cannot place are listed, not dropped. |
| **Draft session** | saved revision, current document, state (`saved`, `dirty`, `saving`, `conflict`, `error`), the idempotency key of the save in flight | Autosave after 1.5 s of quiet; a retry of the same save keeps its key; a `409` keeps the author's copy and asks (D146). |
| **Layout** | node → position | Computed (dagre; layered above 1 500 nodes); the positions a user dragged are a view preference kept in the browser per workflow, never in the document (G148). |
| **Validation view** | platform problems and warnings, browser advice | Newer answers replace older ones; advice is separate and never blocks. |
| **Run model** | nodes by id, edges, sequence, status, recent events | `applyChanges` (gap → reload, repeat → ignore, reset → replace); follows the stream, falls back to polling, resumes by sequence. |
| **Operation in flight** | action, step, reason, `expectedRunVersion`, idempotency key | Never applied optimistically: the effect arrives as a change. |

## Thresholds (D147)

| Nodes | Canvas | Table / list |
|---|---|---|
| ≤ 1 000 | interactive, full cards | available |
| 1 001 – 10 000 | simplified cards, only visible elements drawn | available, windowed |
| > 10 000 | on demand ("show the canvas anyway") | default, windowed |

## Migrations

None. The change to the contract is additive (four operations, two named schemas for what the graph already
returned).
