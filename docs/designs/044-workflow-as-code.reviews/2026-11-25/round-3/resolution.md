# Resolution of round 3 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed commit of round 3: 2249e90100039f5c012b6217e4e5bb454ace1296

How the Blockers and Majors of round 3 were answered, by the author under the owner's delegation (OD-2026-11-20-6,
OD-2026-11-24-4). The decisions are Decision 044-098 to Decision 044-123 of `specs/044-workflow-as-code/research.md`.
Nits were applied except where noted.

Counts before fixes: intermediate zoom-out Blocker 0, Major 5, Nit 4; implementation readiness 4, 13, 9; codebase
details 2, 20, 9; security 0, 12, 9; architecture 1, 25, 9; correctness 3, 21, 9. Total Blocker 10, Major 96, Nit 49 (many
findings are the same defect seen by two or three lenses).

| Findings | Answer |
|---|---|
| zoom-out 1; readiness 1; codebase 8; architecture 8; correctness 4 (node lookups) | `DefinitionGraph::Code` composed with the overlay, so every accessor resolves; a source-text check of every call site; fail-closed arms; the store's side in the new Task 044-31; the acceptance in an integration target (Decision 044-098) |
| codebase 1 (Blocker); architecture 19; correctness 27 (the reserved step) | `steps.kind` `WORKFLOW_TASK` with rewritten constraints, `NodeKind::WorkflowTask`, the domain predicates changed themselves, the claim's workflow-task arm and stop-reason rule (Decision 044-099) |
| correctness 1 (Blocker) (reviews released by waking) | Only `WAITING` with kind `DECIDER` is woken; the reconciler never touches a review (Decision 044-100) |
| correctness 2 (Blocker) (no-ops break replay) | `command.noop` with its record, counted and chained (Decision 044-101) |
| correctness 3 (Blocker); architecture 10 (saga order) | Dependencies by journal sequence; plans in `run_sagas` and `run_saga_entries` (zoom-out 2) (Decision 044-102) |
| architecture 1 (Blocker); security 2 (envelope secrets and overlaps) | Exact entries only for secrets; precedence by exactness and length (Decision 044-108) |
| codebase 2 (Blocker) (immutable receipts) | The append-only `code_update_transitions` (Decision 044-105) |
| readiness 2 (Blocker); codebase 15; zoom-out 5; architecture 3, 4; correctness 16 (the inbox) | Architecture decision 0111's shape and `signal-inbox-v1`; consumption by the function; Task 044-21 builds on that design's inbox task or builds it to its decisions (Decision 044-104) |
| readiness 3 (Blocker) (goldens) | The new Task 044-30: the corpus, its format, the vectors, the review rule and the fake server |
| readiness 4 (Blocker); zoom-out 4, 6 (the core and deferrable split) | Only Tasks 044-14, 044-29 and 044-27 are deferrable, with what slips named; patches, the pause, telemetry and measurements are core; worker-query pieces moved into Task 044-29 |
| zoom-out 3 (sticky) | Deferrable in Task 044-14; the core reuses a cache by window start (Decision 044-109, Owner question 044-018) |
| zoom-out 7; codebase 17, 29; architecture 15 (moves) | All moves are batch jobs; verdicts and retries on `batch_items`; `code_replay_checks` holds the lease; `NO_TARGET_WORKER`; `REPIN` refused for code runs (Decision 044-117) |
| zoom-out 8 (the overlay table returns) | A Complexity Tracking line says why; the digest is kept only in the journal and the chain |
| readiness 5, 7; codebase 12, 13 (contracts and scaffolding) | Technical Plan 13.1 to 13.3 (protocol fields, routes with permissions and errors, the error list); stubs, `x-planned`, the battery, the Studio types and the reference goldens in Task 044-2 |
| readiness 6; codebase 5; security 1 (bindings) | `worker_binding_workflow_types`, the four binding checks, session closing, the request schema (Decision 044-107) |
| readiness 8; codebase 3, 4 (targets and files) | Three targets with paths and entries; the security tests in `integration_workflow_as_code::security`; files per task where they are new |
| readiness 9 (task order) | Tasks 044-4 and 044-5 before Task 044-3; the with-start calls in Task 044-21; the creators per task |
| readiness 10 (missing tests) | Named in their tasks |
| readiness 11; codebase 20 (storage details) | The check lists its columns; `input_digest` and the pause column defined; both down tests |
| readiness 12; architecture 9 (statements) | 57 against `COMPLETION_WITH_A_NEXT_STEP`, the seven additions listed; the overlay cached (Decision 044-115) |
| readiness 13 (crash points, generator, differential) | The kill after commit, the seeded generator in the test with its oracle, the differential harness (Detailed Implementation Plan) |
| readiness 14; codebase 14 (telemetry) | The new Task 044-33 with the metric enumerations, spans, audit actions, the version list and the alert |
| readiness 15; codebase 18 (SDK surface) | Technical Plan 1.12 and `publishCode` over today's routes (Decision 044-122) |
| readiness 16; codebase 22 (conformance harness) | Task 044-26 names the format, the steps and the "not offered" outcome |
| readiness 17 (gates) | Exact commands |
| codebase 6; architecture 5 (capability) | `code-workflows-v1`, its enumeration and checks, removal refused (Decision 044-106) |
| codebase 7 (canonical JSON) | A new public RFC 8785 function, vectors in the corpus (Decision 044-103) |
| codebase 9; architecture 17 (minors and quotas) | Per-step minor through design 045's column; cumulative minors stated; the workflow-task arm and the `sagat.workflow` bucket (Decision 044-123) |
| codebase 10, 11 (schema fields, the example) | The code profile rewritten against `workflow.schema.json`, additions marked; the example corrected |
| codebase 16; architecture 20; correctness 23 (the move function) | The move is a domain command persisted by `PgRunTx`; the function only updates the pin after checking the replay check and the event (Decision 044-117) |
| codebase 19 (cumulative minors) | Stated in Technical Plan 13.1 and the build order |
| codebase 21; correctness 24 (saga cancel policy, engine stops) | `cancelPolicy` on the open-saga command; engine-driven stops apply it (Decision 044-102) |
| security 3, 4, 17 (scripts, events, child version) | Script entries, `envelope.events`, child versions in the envelope (Decision 044-108) |
| security 5 (approvals not bound) | `requiresApproval`: the engine holds the step behind an approval of that step (Decision 044-108, Owner question 044-017) |
| security 6 (update replay leaks) | The read rule applies to replays; `UPDATE_KEY_IN_USE` (Decision 044-105) |
| security 7; architecture 14; correctness 12 (headroom) | Defined at 50,176 and during the window; only the first cancellation journaled; the 56,320 bound is the largest replayed history (Decision 044-113) |
| security 8; architecture 22 (arguments, state, quotas) | `argsSchema` for updates and worker queries, 256 KiB caps, set-state validated, all retained content charged (Decision 044-120) |
| security 9 (business literals in the overlay) | Classified payloads; the spec keeps names and options (Decision 044-116) |
| security 10 (the lease's disclosure) | Stated in Technical Plan 7.4, the security documentation and the binding screen; audited per lease |
| security 11; correctness 21; architecture 31 (the pause) | A permission, checked in the claim and the completion; tested with a session registered before |
| security 12 (batch re-authorization) | Per item (Decision 044-117) |
| architecture 2 (two versions per run) | Gap 044-021 to the 5.0.0 draft's owner; the start version as identity origin in the overview (Task 044-1) |
| architecture 6; correctness 6, 7 (one append, origin) | `PgRunTx::append_events(events, origin)`, the architecture check, origin-aware waking, the reconciler on the journal (Decision 044-100) |
| architecture 7 (expiry in SQL) | A parity test of every expiry path |
| architecture 11 (worker queries and committed state) | Answered from the last committed completion; ADR-0031 decision 7 superseded for them (Decision 044-118) |
| architecture 12 (`202` to old clients) | `acceptPending` opts in (Decision 044-105) |
| architecture 13; correctness 18 (reset of a moved run) | Starts on the current code version with the prefix's overlay; `EXCLUDE` for the move event (Decision 044-112) |
| architecture 16 (query rows, waits) | Arguments and answers in `payloads`; waits bounded per process, Gap 044-023 |
| architecture 18; correctness 17 (fits; publisher after a move) | Defined in Requirement 044-023 and Technical Plan 7.3 |
| architecture 21 (export format) | Versioned with a baseline (Decision 044-119) |
| architecture 23; correctness 10, 11 (continue-as-new) | Refused while obligations are open, cancels timers and waits, `same` is the current version, ADR-0030 decision 16 superseded (Decision 044-111) |
| architecture 24 (replay equals projection) | `verify_run` compares every code column; sticky fields are routing state (Technical Plan 4.3, 11.1) |
| architecture 25 (ADR sections) | Compatibility, operational cost and the measurements under "Required before acceptance" in Architecture decisions 0105, 0107 and 0108 |
| architecture 26 (gates on other features) | Every task carries an `Owner-gate:` line; tasks inside other features cite owner decision 2026-11-24-1 |
| correctness 5 (lease input) | The literal input; the derived steps' input defined (Decision 044-116) |
| correctness 8, 9, 22 (terminal state, success stop, obligations) | `CLOSED`; `StopReason::Completed`; obligations not counted (Decision 044-110) |
| correctness 13 (100 ms rule) | Withdrawn; the namespace bucket bounds workflow tasks (Decision 044-114) |
| correctness 14 (sticky window twice) | The lease carries its window start; a failure drops the cache (Decision 044-109) |
| correctness 15 (bodies outside the chain) | Every command's body is chained (Decision 044-103) |
| correctness 19 (Python preloaded modules) | A private module namespace with the SDK's importer (Decision 044-121) |
| correctness 20 (time zone) | UTC required in TypeScript, Python and C# (Decision 044-121) |

## Nits not applied

- Zoom-out 9 (numbers below design altitude): the update-wait schedule moved to the contract; the bounds, the replay
  arithmetic and the type-name pattern stay, because the owner asks for exact arithmetic for every budget and bound.
- Architecture 33 (a diff of two code versions): recorded as Gap 044-022.
- Architecture 35 (the 5.0.0 draft's wording on version markers): sent to that draft's owner through Gap 044-021.
- Codebase 25 (`AppendStepLogs`): the refusal applies once design 041 serves it, as Requirement 044-003 now says.
