# Data Model: Visual Studio, Monaco and runtime graph

## Context

The Studio adds **no table, no column and no migration**. Everything it shows is read from the platform (the
definitions of 001/002, the journal and the runtime graph of 002/003, the scripts of 005), and everything it
writes goes through the operations that already existed (`PUT /workflows/{id}`, `publish`, `operations`, the scripts
API). What this feature defines is the **wire contract of five new read operations** and **what the browser holds
in memory**. Behaviour and decisions: [`research.md`](./research.md) (D138–D150), ADR-0016.

## The platform's side

| Operation | Reads | Guarantees |
|---|---|---|
| `POST /workflows/{id}/validate` | the document in the body, or the stored draft | The same checks as publish (schema, graph rules, expressions, name, secrets the caller may use, script artifacts built for the tenant), answered as a list; nothing stored, nothing audited. Needs `definition:write`. |
| `GET /workflows/{id}/diff?from=&to=` | two documents (`draft` or a version number) | Compared as written; sorted, so the same two documents give the same answer; `breaking` and `attention` as defined in D140. Needs `definition:read`. |
| `GET /runs/{id}/graph/changes?since=N` | the run's graph, the journal after `N` | The events after `N` (≤ the graph's sequence) and the full objects of the nodes they name; `reset` with the whole graph when `N` is ahead or more than 500 behind. |
| `GET /runs/{id}/stream[?after=N]` | the same, pushed | `text/event-stream`: `graph` events (id = sequence), `: ping` every 15 s, `end`, `reconnect`; resumable with `Last-Event-ID`; closed at the token's expiry and after 15 minutes. At most 20 per subject and 200 per tenant. |
| `POST /runs/{id}/payload-inspections` | the run input or one immutable journal event selected by sequence | An Admin or Operator with both `run:read` and `run:payload:inspect` receives `structure-only-v1`: container types, ordinal child labels and counts, with every object key and scalar removed. The server returns verified catalog metadata for external payloads without downloading them, or a closed unavailable reason. The authorized read and collapsed audit commit together before projection. |

Invariants (tested in `tests/e2e/workflow-studio`):

- A node changes only through a journal event that names it or activates it (`RunEvent::changed_nodes`); therefore **applying
  every delta in order gives exactly the graph the platform would send whole** (`us03_applying_every_change_in_order…`, and
  `us03_the_deltas_still_equal_the_whole_graph_when_a_plan_activates_steps_that_wait_their_turn` for steps that wait their turn).
- A delta never contains an event with a sequence above the graph it ships with.
- Another tenant's workflow or run is `404` for every operation, and a refusal never reveals whether it exists.
- The journal, graph and stream responses hold no payload, input, output or secret. Payload inspection is a separate,
  authorized and audited operation whose bounded projection contains no object key or scalar value.

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
| **Payload inspection** | policy, selected provenance, content kind or bounded structural tree | Created only after the user asks. Never contains keys or scalar values; never enters clipboard, export or persistent storage. An in-flight answer is aborted and discarded when the run, node, tenant or session changes. |
| **Operation in flight** | action, step, reason, `expectedRunVersion`, idempotency key | Never applied optimistically: the effect arrives as a change. |

## Thresholds (D147)

| Nodes | Canvas | Table / list |
|---|---|---|
| ≤ 1 000 | interactive, full cards | available |
| 1 001 – 10 000 | simplified cards, only visible elements drawn | available, windowed |
| > 10 000 | on demand ("show the canvas anyway") | default, windowed |

## Migrations

None. The contract changes are additive: the original four operations and the payload-inspection operation use
existing immutable payload, journal and blob-catalog records.

### Namespace creation UI contract (T007-052)

`namespace:create` from `/auth/context` exposes **New namespace** on the `/namespaces` page.
The **Namespaces** navigation menu opens the list; the shell selector only changes context. The menu of a
namespace (Workflows, Runs, Operations, Scripts, Integrations) is shown only while one is chosen (T007-058): nothing is chosen
by default, the choice is the slug kept in `studio.namespace`, and a slug the tenant does not list is dropped.
The dialog reads all pages of `GET /workspaces` and posts generated `CreateNamespaceRequest`
(`slug`, `environment`: dev/staging/prod) to `POST /workspaces/{workspaceId}/namespaces`.
It never supplies a tenant ID: authenticated server admission remains authoritative. Successful
creation adds the returned resource to the selector and selects its slug; a refusal retains input
and the previous selection. Conflicts explain tenant-wide name uniqueness. Empty workspace lists
explain the API prerequisite. Existing identity audit, typed errors and HTTP traces/metrics apply;
no wire schema or execution semantics change.

### Integrations UI contract (T007-053)

`/integrations` lists `/namespaces/{id}/connectors` with `event:read`; `event:manage` enables
creation and editing of the existing NATS/Kafka/SQS settings and ACTIVE/PAUSED configuration.
Each namespace mounts an independent form/list. Health is the server's error category and message
receipt evidence, never a fabricated connectivity result. New connectors start paused.

New credentials require `secret:manage` and are sent only to `PUT /namespaces/{id}/secrets/{name}`
under a generated unique name. The following connector write contains only `secretName`.
The two writes are not atomic: after a successful secret write and failed connector write, the form
clears credential fields and retains the reference for retry. Cancelled/failed setup can leave an
unused encrypted secret; it is not deleted automatically or mistaken for an active integration.
Editing never retrieves a stored credential value. Rotation makes a new secret and preserves the
old one; administrators can remove unused versions/references through the existing secrets API.

`egress:manage` exposes the namespace's existing network policy with explicit save. Its scope is
all outbound connections in the namespace, not only the selected connector. No implicit network
permission or bypass is added; existing server policy checks, quotas, tenant admission, audit and
HTTP/connector telemetry remain authoritative. The UI rejects credentials in broker addresses and
requires TLS for entered credentials. Secret request errors are rendered without response messages,
which could otherwise echo a value. Secrets never enter query caches or browser persistence.
