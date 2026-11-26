# Resolution of the correctness and architecture reviews of round 2 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed commit of these reviews: 19d226028a2a387d77c637ac27b7bc0f88cf5f8a

How each Blocker and Major of `correctness.md` and `architecture.md` was answered, by the author under the owner's
delegation (OD-2026-11-20-6, OD-2026-11-24-4). The decisions are Decision 044-069 to Decision 044-097 of
`specs/044-workflow-as-code/research.md`. Every Nit was applied. Nothing was contested.

Counts before fixes: correctness Blocker 3, Major 21, Nit 8; architecture Blocker 2, Major 23, Nit 7.

| Finding | Answer |
|---|---|
| correctness 1 (Blocker): the engine reads every node from the published graph | The per-run node overlay, the audit of every `node_for_instance` call site, Requirement 044-034 (Decision 044-070) |
| correctness 2 (Blocker): a run moved off a review waits for ever | Pending derived from the journal; a move off a review makes the step `READY` with a hint (Decisions 044-072, 044-076) |
| correctness 3 (Blocker): an output too large is not a known failure | `STORAGE_QUOTA_EXCEEDED`, an unknown outcome under feature 037 (Technical Plan 10.4) |
| correctness 4, 6, 7; architecture 6, 7: waking paths, visible against waking, the counter | Exhaustive `is_visible` and `is_waking` in the domain; pending derived in one journal append; the reconciler repair; a test over every waking kind (Decision 044-072) |
| correctness 5: a command overtaken by an event | An audited no-op, tested at a barrier in both orders (Decision 044-096) |
| correctness 8: the protocol-minor gate is per run | Per step kind (Decision 044-091) |
| correctness 9; architecture 23: derived steps' secrets, risk metadata from the worker | Risk metadata from the envelope; derived steps get their own entry's secrets (Decision 044-081) |
| correctness 10: eager leases and lost answers | Provisional leases of feature 006 (Decision 044-090) |
| correctness 11: a move and the sticky session | A move clears the sticky fields; the eager workflow task checks digest and binding |
| correctness 12; architecture 10: the sticky expiry wakes nobody; the claim changes | The sticky name and the timer pass that rewrites it and writes the hint; the claim's SQL unchanged; a claim-budget line for graph steps (Decision 044-089) |
| correctness 13, 16; architecture 9: unhandled events, continue-as-new, dropped window events | `applied_through_seq`, the terminal command waits for the window, continue-as-new waits through feature 014's handoff (Decisions 044-078, 044-079) |
| correctness 14; architecture 16: the closing window | Sized to the outstanding work, exhaustion fails the run through the stop drain, the replay budget on 56,320 events (Decision 044-080) |
| correctness 15: a terminal command with leased work | The stop drain (Decision 044-078) |
| correctness 17: retry for worker-caused causes | Retry offered first to another session (Technical Plan 6.3) |
| correctness 18: where a replay check stops | At the last completed activation; `S(k)` compared (Decision 044-076) |
| correctness 19; architecture 14: reset | To the current code version only, eligible points, rematerialized steps, the seed kept, a rule per new kind; ADR-0050 refined in Architecture decision 0105 (Decision 044-085) |
| correctness 20: the failure count | `code_consecutive_failures`, tested across a restart |
| correctness 21: the cancellation grace | A deadline committed in the request's transaction, checked in the completion, a barrier test (Requirement 044-022) |
| correctness 22: an update result outside its schema | The update fails, the run is left alone (Decision 044-084) |
| correctness 23: reading a long history in the lease | The lease extends while history is read, up to 120,000 ms; a fault test on 51,200 events (Decision 044-093) |
| correctness 24; architecture 11: tests that cannot fail, the statement budget | The window property test against PostgreSQL, the barrier tests owned by tasks, the table over waking kinds, a budget relative to today's activity completion with reads, the differential test (Decision 044-077) |
| architecture 1 (Blocker): buffered signals against ADR-0050 and ADR-0030 | Architecture decision 0111's inbox for code versions, refined for code consumers in Architecture decision 0108; Gap 044-017 (Decision 044-073) |
| architecture 2 (Blocker): a tenant's publication records a fleet requirement | An operator records `code-workflows`; publication is refused until then (Decision 044-074) |
| architecture 3: pipeline moves, two versions per run | Moves only by a person, frozen batch selections; the start version is the identity origin; the overview and the principles page updated by Task 044-1 (Decision 044-075, Owner question 044-016) |
| architecture 4: no `Owner-gate:` line | Added to Tasks 044-1, 044-2, 044-7, 044-12, 044-13, 044-21 |
| architecture 5: the chain in the domain | A port over opaque digests; hashing in `sagat-application` (Decision 044-082) |
| architecture 8: the reserved node | `is_workflow_task_step`, tested first everywhere (Decision 044-071) |
| architecture 12: code sagas and ADR-0010 | Superseded for code runs in Architecture decision 0107; journal-derived dependencies; `code_saga_plan_entries` (Decision 044-083) |
| architecture 13: updates and ADR-0031 | Superseded for code versions in Architecture decision 0108 (Decision 044-084) |
| architecture 15: classified values in literal inputs; the developer grant | Entry `inputSchema` classifies inputs; `history:export` admin only (Decision 044-086, Owner question 044-015) |
| architecture 17: durable claim of queries and checks | Rows claimed through `PollTasks`, re-announced by the reconciler (Decision 044-087) |
| architecture 18, 19: storage and the down | Technical Plan 14 lists every column and table; the down refuses while a code run or version exists; the check `NOT VALID` then validated |
| architecture 20: workflow-task rate | One workflow task per 100 ms per run (Decision 044-092) |
| architecture 21: `workflow:<type>` in the activity space | A separate `workflowTypes` on bindings (Decision 044-088) |
| architecture 22: design 045 | The interaction row, the code-profile fields, Gap 044-020 (Decision 044-097) |
