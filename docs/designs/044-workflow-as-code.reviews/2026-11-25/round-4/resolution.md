# Resolution of round 4 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed commit of round 4: ba5a3fdd02abfc57e03b76efc4a9fb2e237b5c45

How the Blockers and Majors of round 4 were answered, by the author under the owner's delegation (OD-2026-11-20-6,
OD-2026-11-24-4). The decisions are Decision 044-124 to Decision 044-147. Every Nit was applied.

Counts before fixes: design re-audit Blocker 0, Major 5, Nit 11; implementation readiness re-audit 1, 6, 14; security 0,
7, 12; architecture 0, 7, 18; correctness 0, 6, 14. Total Blocker 1, Major 31, Nit 69. Round 3 had Blocker 10, Major 96.

| Findings | Answer |
|---|---|
| readiness 1 (Blocker): publication waits for `signal-inbox-v1` | Only a code version that declares signals needs it; the capability `code-workflows-v1` comes with Task 044-6 (Decision 044-139) |
| design 1; correctness 3 (code sagas and the graph planner) | Every `WorkflowDefinition` method answers through the code definition; scopes are saga nodes; no automatic transitions for code scopes; `policies` fails closed (Decision 044-126) |
| design 2; correctness 4 (signal consumption) | `signal.consumed`, chained, its sequences checked (Decision 044-124) |
| design 3 (Python importer) | Shared, restricted and refused classes; the audit hook scoped to the activation (Decision 044-128) |
| design 4; correctness 2 (refused terminal commands) | Deferral: the other commands commit, the command is re-emitted last (Decision 044-125) |
| design 5; correctness 6 (reconciler) | Once per wait, the primary key backwards, the stored origin, a parity test (Decision 044-129) |
| architecture 1; correctness 5, 6 (journal self-contained) | Origin, specs, body digests and the seed in the journal; the overlay a projection (Decision 044-124) |
| architecture 2 (reset inside an open saga) | Refused (Decision 044-138) |
| architecture 3 (publication against removal) | The requirement held `FOR SHARE` through the shared definer function (Decision 044-139) |
| architecture 4 (design 045's level) | `timeoutLevel` in the envelope (Decision 044-140) |
| architecture 5 (the inbox fallback) | Removed; Task 044-21 waits for the inbox design (Gap 044-017) |
| architecture 6 (Architecture decision 0106 drift) | Rewritten to the private namespace and the UTC preconditions |
| architecture 7; security 17 (feature 022 policies from code) | The envelope entry owns them (Decision 044-140) |
| correctness 1 (cooperative cancellation) | A code-run cancel arm, `run.cancel_requested`, a deadline pass (Decision 044-130) |
| security 1 (reserved names in graph publication) | Refused at graph publication and at claim; a scan before recording (Decision 044-131) |
| security 2 (lease kinds per RPC) | The matrix of Technical Plan 4.1 (Decision 044-132) |
| security 3 (eager leases bypass admission) | Through the claim's admission function (Decision 044-133) |
| security 4 (approval initiators) | The starter and every principal whose input was delivered (Decision 044-134) |
| security 5 (update-key oracle) | Keys scoped by run and principal; `404` (Decision 044-135) |
| security 6 (worker-query payloads) | Inline, bounded, charged, deleted (Decision 044-136) |
| security 7 (feature 016 carriers) | New carriers under a new rules version; the rest `pii` (Decision 044-137) |
| readiness 2 (task order) | Task 044-6 before Task 044-3; tests whose prerequisite lands later stay red until then; Task 044-26's prerequisites |
| readiness 3 (the typed append) | A free function with `JournalRow`; graph counts unchanged (Decision 044-144) |
| readiness 4 (crash tests) | The feature, the variable, the barrier, the script, the gate (Decision 044-143) |
| readiness 5 (no workflow worker for the server lane) | The scripted decider (Decision 044-143) |
| readiness 6 (signatures and packaging) | The reference signatures; the runtime check; the bundler package (Decision 044-145) |
| readiness 7 (batch job fields) | `reason`, `params`, selector, `max_items`, rate (Decision 044-146) |
