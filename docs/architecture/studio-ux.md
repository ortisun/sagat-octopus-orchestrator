# Workflow Studio — journeys and screens

## Personas

Author (define), Developer (code/debug), Operator (run/retry/compensate), Approver (human tasks), Security Admin (policies/audit).

## Navigation

Workspace → Namespace (dev/stage/prod) → Workflows → Versions → Runs → Run Detail → Timeline / DAG / Logs / Traces / Compensation.

## Workflow Builder

- Definition canvas: add node, typed edges (success/failure/timeout/compensate), drag-drop, properties, branch/parallel/join/loop/child/wait.
- Current script authoring uses a separate script editor page with Monaco. The canvas inspector selects an already-built script version through `ScriptPicker` and stores its artifact digest. Inline Monaco editing inside the canvas inspector remains a proposed UX. See [node inspector](../../frontend/apps/studio/src/features/workflows/NodeInspector.tsx) and [script editor](../../frontend/apps/studio/src/features/scripts/ScriptEditorPage.tsx).
- Original design palette (labels, not a list of executable IR type names; see [current IR acceptance](workflow-ir-and-sdk-modes.md)): Trigger, Activity/Script, Condition, Parallel, Join, ForEach, WaitEvent, Timer, HTTP, Kafka, SQS, ChildWorkflow, SagaBoundary, HumanApproval, End.
- Validate the graph incrementally: orphan nodes, incompatible types, cycles outside an explicit loop, missing timeout, unmapped compensation, irreversible actions, secrets/permissions and fanout limits.
- Version Draft vs Published; publish requires confirmation/diff and contract compatibility. New runs receive the current version according to routing; existing runs are pinned.
- The visual editor operates on canonical declarative IR. Arbitrary code hosted by an SDK may only have a map of steps observed at runtime.

## Run Detail

- Header: run_id, workflow_version, tenant/namespace, status, duration, trigger, correlation_id, initiator, authorized actions.
- Executed graph: nodes colored by status, attempts, duration, edges actually traversed, compensations and omitted branches differentiated; data updated via read-side SSE/WebSocket.
- Timeline based on the journal (source of truth), OTel spans per attempt, filtered logs, masked payload, retries, fences, timeout and DLQ.
- On failures: correlated root cause, `retry step`, `retry run as new`, `pause`, `cancel`, `request compensation`, `resume/manual complete` with justification and audit.
- Current approval requests select eligible subjects/roles, can exclude the initiator and settle on the first authorized committed decision. A single request does not collect a two-person quorum. Versioned-form/SLA and configurable dual-approval UX are design targets requiring explicit scope and acceptance; they are not guarantees of the current approval node. See [approval state machine](../../crates/domain/src/execution/approval.rs).

## Accessibility and UX

WCAG 2.2 AA as a goal, keyboard navigation, contrast, screen reader, labels, alternative list view; virtualization for large DAGs; pagination of history and logs; messages that do not reveal sensitive data.

## Product metrics

Time to first workflow, publish-without-error rate, failures per connector, time to diagnosis, run recovery, top slow steps; instrument without capturing sensitive content.
