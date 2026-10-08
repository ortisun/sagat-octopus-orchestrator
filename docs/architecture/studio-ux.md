# Workflow Studio — journeys and screens

## Personas

Author (define), Developer (code/debug), Operator (run/retry/compensate), Approver (human tasks), Security Admin (policies/audit).

## Navigation

Workspace → Namespace (dev/stage/prod) → Workflows → Versions → Runs → Run Detail → Timeline / DAG / Logs / Traces / Compensation.

## Workflow Builder

- Definition canvas: add node, typed edges (success/failure/timeout/compensate), drag-drop, properties, branch/parallel/join/loop/child/wait.
- Side panel with Monaco for inline script and node code; autocomplete, lint, isolated test, console and input/output preview by JSON Schema.
- Main nodes: Trigger, Activity/Script, Condition, Parallel, Join, ForEach, WaitEvent, Timer, HTTP, Kafka, SQS, ChildWorkflow, SagaBoundary, HumanApproval, End.
- Validate the graph incrementally: orphan nodes, incompatible types, cycles outside an explicit loop, missing timeout, unmapped compensation, irreversible actions, secrets/permissions and fanout limits.
- Version Draft vs Published; publish requires confirmation/diff and contract compatibility. New runs receive the current version according to routing; existing runs are pinned.
- The visual editor operates on canonical declarative IR. Arbitrary code hosted by an SDK may only have a map of steps observed at runtime.

## Run Detail

- Header: run_id, workflow_version, tenant/namespace, status, duration, trigger, correlation_id, initiator, authorized actions.
- Executed graph: nodes colored by status, attempts, duration, edges actually traversed, compensations and omitted branches differentiated; data updated via read-side SSE/WebSocket.
- Timeline based on the journal (source of truth), OTel spans per attempt, filtered logs, masked payload, retries, fences, timeout and DLQ.
- On failures: correlated root cause, `retry step`, `retry run as new`, `pause`, `cancel`, `request compensation`, `resume/manual complete` with justification and audit.
- Human approval: versioned form, SLA, timeout, authorization and configurable dual approval.

## Accessibility and UX

WCAG 2.2 AA as a goal, keyboard navigation, contrast, screen reader, labels, alternative list view; virtualization for large DAGs; pagination of history and logs; messages that do not reveal sensitive data.

## Product metrics

Time to first workflow, publish-without-error rate, failures per connector, time to diagnosis, run recovery, top slow steps; instrument without capturing sensitive content.
