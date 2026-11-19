# Consolidated egm-review report of design 041 (revision of 2026-11-13)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 879c550b1726870fa56392b1cff20272f114a5ed
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Preset: "Consolidated report with all review rubrics" (uninterrupted full flow); split at `## Detailed Implementation Plan`, line 829 (FeatureRange 1-828, DetailedRange 829-1047)
GoldfishContext: docs/constitution.md; docs/adr/ADR-0084-*.md; docs/adr/ADR-0085-*.md; specs/041-step-logs/research.md; docs/designs/041-step-logs.reviews/2026-11-13/{architecture,security,correctness}.md
Note: every reviewer ran as a fresh agent of its registered type (`egm:egm-zoom-out`, `egm:egm-comprehension`, `egm:egm-design`, `egm:egm-structural`, `egm:egm-style`, `egm:egm-implementation-readiness`, `egm:egm-codebase-details`), dispatched directly by the coordinator, stages in order 5 → 9, reviewers of one stage in parallel. The design was not to be edited in this task (T041-1), so the `egm-resolve` hand-off of stages 7 and 9 did not run: each loop ran one round and is not converged, and every finding below is open. The design's Status stays `Draft`.

## What ran

| Stage | Reviewers | Target | Blocker | Major | Nit | Report |
| --- | --- | --- | ---: | ---: | ---: | --- |
| 5 Initial zoom out | `egm-zoom-out` | 1-1047 | 0 | 5 | 8 | `egm-zoom-out.md` |
| 6 Comprehension | `egm-comprehension` | 1-828 and 829-1047 | 0 | 8 | 8 | `comprehension.md` |
| 7 Critic, round 1 | `egm-design`, `egm-structural`, `egm-style` | 1-828 | 0 | 20 | 24 | `critic.md` (44 entries from 50 raw) |
| 8 Intermediate zoom out | `egm-zoom-out` | 1-1047 (unchanged) | 0 | 3 | 10 | `intermediate-zoom-out.md` |
| 9 Implementation readiness, round 1 | `egm-implementation-readiness`, `egm-codebase-details`, `egm-style` | 829-1047 | 3 | 23 | 14 | `implementation-readiness.md` (40 entries from 58 raw) |

Every reviewer completed; none was retried or recorded as not run. The 126 stage entries are consolidated below into 54 actions. Stage entries are cited as `Z5-n` (stage 5), `C6-n` (stage 6), `K7-n` (stage 7), `Z8-n` (stage 8), `R9-n` (stage 9), by their number in the stage report.

Verification by the coordinator (rule 17), each in the code or the record, not taken from a report: A-M1 against `specs/018-schedules-bulk-operations/research.md` D1958 and `crates/infra-postgres/src/batch.rs:449-459, 652-657`; A-B2 against `ActivityLogger.java:7-24` and `activity.ts:87-92`; A-M20 against `crates/api-grpc/build.rs:31-34`; A-M29 against docs/designs/README.md lines 40-42, `citable()` in `scripts/check-design-docs.py` and docs/owner-decisions.md; A-N11 against the migrations directory and design line 523. All hold, except that the migration anchor (R9-34) is a Nit and not the Blocker `egm-codebase-details` raised: "the next free timestamp after …, checked against the other open branches" still picks an unused timestamp.

The stage 8 reviewer's advice on how to merge the stage 5 and 7 recommendations is folded into the actions (it rejects some recommended fixes; where it does, the action says so).

## Blocker

- **A-B1. Settle the contracts T041-3 freezes.** Proto fields, numbers and enums (level, kind, reason, verdict), the `RETRY_LATER` delay, `StepLogPolicy`; OpenAPI item schemas, stream states, the lag field, SSE payloads and `Last-Event-ID`, parameter formats, the delete response; an error table (`FEATURE_DISABLED` is not in the closed `Error.code` enum; codes for a missing permission, a missing grant, a foreign cursor, a long `q`, the tail caps); the poller's end rule. Put them in a contract appendix or `specs/041-step-logs/contracts/`. Sources: R9-1.
- **A-B2. Specify the six SDK logger surfaces before T041-15 to T041-20.** Java's `ActivityLogger` takes no fields and no `Throwable`, TypeScript, Go and Python have no `trace`, so FR-041-007 is false and three scenarios cannot pass without new public API; give a per-language table (new methods, level mapping, whether the capture floor bypasses the host's enabled check, exception placement, formatting and field flattening, excluded identifiers, option names, the Rust `ActivityContext::log()` signature, retry spacing, re-enabling after `STOP_CAPTURE`, a process-wide memory bound). Leave the redaction algorithm to each SDK (equal to `redact` by goldens; cost measured in T041-9). Sources: R9-2, K7-23, K7-24, Z8-13.
- **A-B3. Decide the script-output marker: drop the write or specify it fully.** The reviewers of stages 5 and 8 recommend listing the run's `script_executions` from the read side (own cursor, own expiry) instead of writing a `source = script` stream; this removes the nullable key, the lock-order exception and T041-13's barrier test. If the write stays: the instance path is never filled today (`NewExecution` has no instance, the insert writes none), the run-less arm, the gating by the switches, the statement order against ADR-0084 rule 11, `NULLS NOT DISTINCT`, and writing only when the execution row was inserted. Sources: Z5-2, Z8-6, R9-3, R9-8, K7-26.

## Major

- **A-M1. Replace the end-of-tail rule with D1958's.** `xmin >= xmax` passes while a transaction with id `xmax` is still open (feature 018 measured "held 916, horizon 916, xmin 916"); horizon = `pg_current_xact_id()` after the run is seen finished, condition `xmin > horizon`; cite D1958 in section 5 and ADR-0084 decision 9; add the "held id equals horizon" case to T041-12. This reopens first-round M1, which the second-round architecture and correctness reviewers marked answered. Sources: Z5-1, Z8-10.
- **A-M2. One tenant-allowance rule, and the true wait bound.** A completion queues behind every ingest commit ahead of it (sub-cap × replicas), and each commit holds the tenant row through one deferred flush call per inserted row; a tenant at its limit stays on the locking path for days. Stage 8 recommends: never take the tenant row before commit; over the limit (by the unlocked read or the finalizer's raise) store nothing, record `tenant_quota`, return the stop verdict; remove the band and the locking path; state the bound as queue depth × commit hold. That reads ADR-0027's "keep what fits" rule as applying to terminal reports only, an ADR-0084 revision for the owner. Sources: K7-1, K7-2, Z5-6, Z8-1.
- **A-M3. Run the spike before the schema is fixed, and give it one contingency.** The spike compares one row per line with chunk rows under full batches from many attempts with completions alongside, on a scratch migration before T041-5 merges; drop the grouped commit as the named first fallback unless measured; define the workload, the pass/fail computation and the harness client (`scalability-ha` completes in process with output `{}` and has no gRPC path); one stop-design-re-review contingency with one insertion point. Reject "a global per-tenant cap" and "one batch per process" unless the spike shows the need. Sources: Z8-2, R9-14.
- **A-M4. Measure and state the per-run and membership hot rows.** `run_log_usage FOR UPDATE` serialises every batch of a run (a 200-iteration `forEach`); the membership row is locked `FOR SHARE` per batch. Keep `FOR UPDATE` (stage 8 rejects lock-free variants); add fan-out and per-worker cases to the spike and T041-9; Risks rows; an approximate run ceiling goes to the owner only if the fan-out fails. Sources: K7-3, Z5-12, Z8-4.
- **A-M5. Fix the record protocol.** `STOP_ATTEMPT` must still allow an empty `final` batch carrying the highest assigned sequence, or no flooded attempt ever shows its final flush and SC-041-002 cannot hold; with that, the `DROPPED` kind can go (the server counts gaps) and its SDK reasons move to an SDK metric; settle the `DROPPED` record's own sequence, the empty-buffer final flush, field-map overflow and oversized batches. Sources: Z5-3, Z8-3, R9-12.
- **A-M6. One description of the capture controls; a smaller `StepLogPolicy`.** Four controls with no combination rule; the policy carries tuning numbers that become permanent 1.7 contract; the namespace scope exists (`QuotaScope::Namespace`), a worker session has one namespace, so `enabled` is derived at registration; test a zero ceiling before the stream row is touched (no row, `STOP_CAPTURE`). Sources: Z5-4, Z5-9, C6-2, Z8-5.
- **A-M7. Move schema detail out of the frozen text, and make it complete where it goes.** Keep invariants in the Technical Plan; column lists, grants, metric names and Studio internals go to `specs/041-step-logs/data-model.md`, which must give the DDL: marker columns, closed sets for `kind`, `marker_reason`, `ceiling_reason`, `level`, `source` (and a `minLevel` order), types and keys of `step_log_streams` and `run_log_usage`, the creation path of the usage row, the owner of `namespaces.has_classified_schemas`, honest filter indexes, a stated key. Sources: Z5-5, Z8-12, R9-6, K7-22, K7-31, K7-34.
- **A-M8. Define the quota resources.** A table per resource (name, scope, kind, unit, range, default, ceiling, meaning of 0), the CHECK-constraint migration and the OpenAPI `Quota.resource` change in T041-4, the OpenAPI text that says lowering a limit does not erase content (it contradicts FR-041-020), an ingest step that reads the namespace policy, the purge's retention lookup; close the open question. Sources: R9-5.
- **A-M9. Put quota triggers on `step_log_lines` only, and name the owner paths.** The manifest requires triggers only on tables with `content` or `log` columns; triggers on the counter tables add `to_jsonb` on every update; entries need existing owner paths, but the adapter is created in T041-6. Sources: R9-7.
- **A-M10. Bound the purge pass.** Purge per namespace over `(tenant_id, namespace_id, last_at) WHERE purged_at IS NULL`; reject a second retention for stream rows and a stored `expires_at` (stage 8). Sources: K7-4, Z8-7.
- **A-M11. Rows with no bytes escape the tenant quota (owner).** Markers and empty lines cost 0; a per-row charge needs an ADR-0027 change and a record quota is another hot row; stage 8 recommends the owner's written acceptance with a stated bound after A-M5 and A-B3 remove most zero-byte rows, chunk rows if refused. Sources: K7-5, Z8-8.
- **A-M12. Make the classified-namespace flag fail closed.** Old replicas publishing after the backfill never set it, and it is never re-evaluated; the backfill cannot run the Rust walk in SQL. Name the mechanism (an idempotent pass re-run once no old replica remains, checked in T041-26), the publication call site, the legacy rule; refuse true to false in the database; keep the read routes off until it completes; an upgrade test. Sources: K7-6, Z8-9, R9-16.
- **A-M13. Signal the horizon lag.** Any long or prepared transaction freezes every tenant's new lines and tails; add a database-wide oldest-open-transaction age to the operations snapshot and state the PostgreSQL settings relied on; reject a "may be incomplete" `end` (stage 8). Sources: K7-7, Z8-10.
- **A-M14. Reorder the ingest's session and authority checks.** Session, then lease and `steps.activity`, then membership and binding in that namespace; check `expires_at` and the session's namespace; define the token after re-registration; name the `pub(crate)` exposures of `current_worker_authorized`. Sources: R9-13.
- **A-M15. One admission point and a transport-status table.** One `Class::Logs` with a per-tenant count, stated per replica, values and configuration keys; a gRPC-status-to-SDK-action table (`RESOURCE_EXHAUSTED`, `UNAVAILABLE`, `DEADLINE_EXCEEDED`, `INVALID_ARGUMENT`, `UNAUTHENTICATED`, `PERMISSION_DENIED`); name the `WorkerStore` proof method. Sources: R9-15, Z5-7, Z8-11.
- **A-M16. Define or drop the sealed cursor** (key source, algorithm, sharing, rotation, encoding, `Last-Event-ID`), or embed and compare the run id. Sources: R9-11, Z5-10.
- **A-M17. Design the throttled read audit**: reuse `collapses_repeats()`/`repeat_window()` or the denial-dedupe index under a keyed lock; keep the record independent of the filters; a race test. Sources: R9-10, Z5-8.
- **A-M18. Say how the Studio learns of a missing grant** without a request (a field on a named response, or a request mapped to a reason). Sources: R9-9.
- **A-M19. Tag the new operations `x-planned: '041'`** until their route tasks, add them to the battery's `body_of`, and state why T041-29's digest moves. Sources: R9-4.
- **A-M20. Make slice A build**: list every `WorkerService` implementation that must answer `UNIMPLEMENTED`, the regenerated Go and Python stubs and tool versions, and add `StepLogField` to `skip_debug`. Sources: R9-18.
- **A-M21. Name the public scrubber entry points to add** (`scrub_shapes` and `redact_credentials` are private; `scrub` cuts at 2,048) and the rule for non-string values under a sensitive key. Sources: R9-19.
- **A-M22. Extend the conformance harness in T041-14**: logging behaviours, a step op reading the logs route, a verdict-injecting fault, `mustNotAppear` plus a captured-output search, and an owner task for the server canary. Sources: R9-20.
- **A-M23. Move the SDK protocol constants and the negotiation golden, and record who holds minor 1.7** (drafts of features 020, 021, 025, 026 also claim it); move the §7.1 amendment to T041-7 or state the lag. Sources: R9-21, R9-30.
- **A-M24. Provide PostgreSQL 18 in tests** (the support pins `postgres:16-alpine`): name the override and the CI job. Sources: R9-22.
- **A-M25. Add a selector rule for the step-log code paths and create the `integration_step_logs` target** (and the module registrations) in T041-6. Sources: R9-23.
- **A-M26. Give each task its Create/Modify/Delete file list.** Sources: R9-17.
- **A-M27. Make the pull-request counts add up** (each task one pull request, 13 server tasks, ungrouped T041-21/26/27). Sources: R9-24.
- **A-M28. Name what the gates point at and make done conditions checkable**: the three checkers, both switches, the four lock files, the browser target, the validation-report path, the reviewers of T041-27, expected failures per scenario, and an exit for T041-1/T041-2. Sources: R9-25, R9-26, C6-16.
- **A-M29. Record the pending owner acceptance (owner).** "None open" while the revised ADR-0084 and ADR-0085 text awaits the owner through T041-2, whose gate cites OD-2026-11-13-14 (decided before the revision); the cited rows read "Decided; not built" / "Decided; not designed", which `citable()` refuses, so every gate on them fails R5 when ticked. Sources: C6-8, K7-12, K7-15.
- **A-M30. Replace the "this text answers them" claim with a finding-to-section trail**, say which first-round findings were lost, and move the review history out of the Overview. Sources: K7-14, K7-20.
- **A-M31. Align the requirement and goal text with the mechanism**: read order is transaction-id order (and in-batch order needs `ORDER BY sequence`); the Goals and Overview on completion waits; the secret claims limited to verbatim values of at least 6 bytes (G041-001); FR-041-011's undefined "horizon of FR-041-010". Sources: C6-1, C6-9, K7-8, K7-9, K7-18, K7-21.
- **A-M32. Define the project terms**: generation, instance path, storage- against data-classified, the store proofs behind 1.7 reachability, the namespace secrets lock and the two fences (and the step that takes each). Sources: C6-3, C6-4, C6-6, C6-7, K7-32.
- **A-M33. Rename before the contract freezes**: "stream" for the attempt log only, "tail" for the SSE connection (route, caps, metric names), `{runId}` throughout; one verb each for delete and purge (`run.logs_deleted`). Sources: C6-5, K7-16, K7-17.
- **A-M34. Complete Alternatives Considered** with a reason per rejected option, linking the ADRs' options rather than copying them (stage 8). Sources: K7-10, K7-11.
- **A-M35. Add the missing Risks rows** (default-on capture of customer text, six-language redaction drift, a hand-written client skipping redaction, the viewer break of `getScriptExecution.log`, a failed backfill, `ResolveDatabase` on ordinary 1.7 SDKs). Sources: K7-13.
- **A-M36. Split dense multi-obligation sentences and task blocks** (FR-041-007, FR-041-008, the negotiation paragraph, lines 459-466, T041-6, T041-8, T041-11). Sources: K7-19, R9-35.

## Nit

- **A-N1.** `log_truncated` still discloses a withheld script log's size class; the incident path does not cover `script_executions.log`. Sources: K7-25, K7-27.
- **A-N2.** Order T041-10 before T041-7 so `SAGAT_STEP_LOGS_ENABLED` keeps one default. Sources: Z5-13.
- **A-N3.** Tail re-authorization cadence: per poll costs membership locks; the graph stream rechecks every 30 s. Sources: Z5-11.
- **A-N4.** Say where "no log sent", "ended without its final flush" and the iteration pick come from; one flood figure in CUJ 3 and SC-041-002. Sources: C6-13, C6-14, K7-42.
- **A-N5.** Unexplained figures and vague quantities (50/1/11 against 47/1/9, "three constants", "four methods", "about a second", "about a tenth", the lag field, the typed error). Sources: C6-15, K7-41.
- **A-N6.** Link or gloss the features, ADRs, gaps (G69, G1618, G041-00n), roles, runners and admission terms. Sources: C6-10, C6-11, K7-35.
- **A-N7.** Translate the owner's quotes. Sources: C6-12, K7-39.
- **A-N8.** Formatting and traceability: wrapped code spans, NFR id scheme, the diagram, CUJ-to-story and goal-to-criterion references, repeated statements. Sources: K7-28, K7-29, K7-33, K7-36, K7-37.
- **A-N9.** Terminology drift and shorthand in both ranges (capture level, switch, kind, line against record; plan shorthand and ambiguous phrases; "move"). Sources: K7-38, K7-40, R9-36, R9-37, R9-38.
- **A-N10.** Constitution Check rows VII (dead letter) and II/VIII (planned verification written as done). Sources: K7-30, K7-43.
- **A-N11.** The stale migration anchor and the branch-check instruction belong in the migration task. Sources: K7-44, R9-34.
- **A-N12.** Stale line references (`scripts.rs:425-446`, `execution.rs:1002`), `Refs:` order, the Verification row that names a task. Sources: R9-27, R9-39, R9-40.
- **A-N13.** T041-8's restart outcome, the unowned "every stream's run exists" and canary tests, and the tests-first rule against T041-8's order. Sources: R9-28, R9-29.
- **A-N14.** Define the canonical JSON of `fields`. Sources: R9-31.
- **A-N15.** Name `RunLogsDelete`, say whether `follow()` is refactored or copied, and give the lock order of the delete route and the purge. Sources: R9-32, R9-33.

## Traceability matrix

| Action | Priority | Stage entries | Second-round repository findings on the same point |
| --- | --- | --- | --- |
| A-B1 | Blocker | R9-1 | security M6 (cursor content); architecture M4 (`ScriptExecution` schema) |
| A-B2 | Blocker | R9-2, K7-23, K7-24, Z8-13 | security r1-M4 (numeric form), N1 (Rust `tracing`); architecture N14 (capture floor) |
| A-B3 | Blocker | Z5-2, Z8-6, R9-3, R9-8, K7-26 | architecture N1, N9, N11; correctness M6, N6 |
| A-M1 | Major | Z5-1, Z8-10 | reopens first-round M1 of architecture and correctness (both marked it answered) |
| A-M2 | Major | K7-1, K7-2, Z5-6, Z8-1 | architecture M1; correctness r1-M3, M1, M2 |
| A-M3 | Major | Z8-2, R9-14 | architecture M1 (concurrent-ingest test) |
| A-M4 | Major | K7-3, Z5-12, Z8-4 | correctness M8; architecture N7 |
| A-M5 | Major | Z5-3, Z8-3, R9-12 | correctness r1-M5, M4, N4, N5, r1-N7 |
| A-M6 | Major | Z5-4, Z5-9, C6-2, Z8-5 | architecture N5 |
| A-M7 | Major | Z5-5, Z8-12, R9-6, K7-22, K7-31, K7-34 | correctness N7; architecture N11 |
| A-M8 | Major | R9-5 | architecture N5 |
| A-M9 | Major | R9-7 | architecture N3 |
| A-M10 | Major | K7-4, Z8-7 | architecture M2; correctness M5; security N5 |
| A-M11 | Major | K7-5, Z8-8 | security M2 |
| A-M12 | Major | K7-6, Z8-9, R9-16 | architecture M5; security M3 |
| A-M13 | Major | K7-7, Z8-10 | architecture N8 |
| A-M14 | Major | R9-13 | security r1-M1, N6, N9; correctness M3, N9; architecture N13 |
| A-M15 | Major | R9-15, Z5-7, Z8-11 | security M5; architecture N6 |
| A-M16 | Major | R9-11, Z5-10 | security M6 |
| A-M17 | Major | R9-10, Z5-8 | correctness N8 |
| A-M18 | Major | R9-9 | none |
| A-M19 | Major | R9-4 | architecture M4 |
| A-M20 | Major | R9-18 | security r1-N5 |
| A-M21 | Major | R9-19 | security N2 |
| A-M22 | Major | R9-20 | none |
| A-M23 | Major | R9-21, R9-30 | none |
| A-M24 | Major | R9-22 | none |
| A-M25 | Major | R9-23 | none |
| A-M26 | Major | R9-17 | none |
| A-M27 | Major | R9-24 | none |
| A-M28 | Major | R9-25, R9-26, C6-16 | none |
| A-M29 | Major | C6-8, K7-12, K7-15 | architecture M3; correctness r1-N13 |
| A-M30 | Major | K7-14, K7-20 | none |
| A-M31 | Major | C6-1, C6-9, K7-8, K7-9, K7-18, K7-21 | correctness N1 |
| A-M32 | Major | C6-3, C6-4, C6-6, C6-7, K7-32 | security N6 (binding lock) |
| A-M33 | Major | C6-5, K7-16, K7-17 | none |
| A-M34 | Major | K7-10, K7-11 | none |
| A-M35 | Major | K7-13 | none |
| A-M36 | Major | K7-19, R9-35 | none |
| A-N1 | Nit | K7-25, K7-27 | security M4 (raised there as Major) |
| A-N2 | Nit | Z5-13 | none |
| A-N3 | Nit | Z5-11 | architecture N7; security r1-N4 |
| A-N4 | Nit | C6-13, C6-14, K7-42 | none |
| A-N5 | Nit | C6-15, K7-41 | none |
| A-N6 | Nit | C6-10, C6-11, K7-35 | none |
| A-N7 | Nit | C6-12, K7-39 | none |
| A-N8 | Nit | K7-28, K7-29, K7-33, K7-36, K7-37 | none |
| A-N9 | Nit | K7-38, K7-40, R9-36, R9-37, R9-38 | none |
| A-N10 | Nit | K7-30, K7-43 | none |
| A-N11 | Nit | K7-44, R9-34 | architecture N4 |
| A-N12 | Nit | R9-27, R9-39, R9-40 | architecture N4 |
| A-N13 | Nit | R9-28, R9-29 | architecture N12 |
| A-N14 | Nit | R9-31 | security N3 |
| A-N15 | Nit | R9-32, R9-33 | security N4; architecture N10 |

Every entry of the five stage reports appears once in the Stage entries column (Z8-10 twice, A-M1 and A-M13, because it speaks to both).

## Items for the owner (product, scope, release)

1. Acceptance of the revised ADR-0084 and ADR-0085 text (A-M29; architecture M3), including the decisions A-M1, A-M2, A-M5 and A-M33 would rewrite (ADR-0084 decisions 3, 7, 9, 11; ADR-0085 decision 9's action name): one revision through T041-2.
2. The Status cells of OD-2026-11-13-9, -14 and -18 ("Decided; not built", "Decided; not designed"), which the checker refuses to cite (A-M29); editing the owner record is the owner's.
3. Whether step logs may fill the shared `tenant_log_bytes` and leave dispatch, connector and script errors stored empty (correctness M1; A-M2): a reserve, or a separate resource (an ADR-0027 amendment); it revisits O041-005.
4. Zero-byte rows that escape the tenant quota (A-M11; security M2): the constitution makes a quota bypass release-blocking, so either a fix or a written acceptance with a bound.
5. Conditional: if the script marker write is dropped (A-B3), whether the interleaved row in the whole-run tab is part of O041-009; if the spike favours chunk rows (A-M3), O041-004; if a wide fan-out fails (A-M4), an approximate run ceiling under O041-005; if the process secret set is not kept for the process's life, the accepted limit in ADR-0085 rule 12 and G041-001 (security M1, correctness M7); the allocation of protocol minor 1.7 among features 041, 020, 021, 025 and 026 if their release order is not already settled (A-M23).

## Second round of the repository reviewers

Reported separately in this directory: `architecture.md` (Blocker 0, Major 7, Nit 16), `security.md` (Blocker 0, Major 8, Nit 11), `correctness.md` (Blocker 0, Major 10, Nit 11). No first-round Blocker remains open as a Blocker: architecture B1 is partly answered and now a Major, security B1 is answered. Their overlaps with the actions above are in the matrix. The Majors with no EGM counterpart are architecture M6 (the ingest's rules placed in the adapter) and M7 (ADR-0084 rules without a check), and security M1 with correctness M7 (the same defect: the process secret set loses a value when its attempt ends). Correctness M1 (step logs fill the shared `tenant_log_bytes`) and M2 (the namespace secrets-lock queue) are listed beside A-M2 but name mechanisms the EGM stages did not.

Counts: Blocker 3, Major 36, Nit 15
