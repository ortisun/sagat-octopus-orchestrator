# Resolution of the reviews of 2026-11-15 (design 041)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: b1b2c249 (the reviewed text is that of 879c550b1726870fa56392b1cff20272f114a5ed)
Reviewer: egm-resolve, by the author
Status: resolved; one re-audit run (section 5); the approval run answered in section 9

Every finding of `consolidated.md` (54 actions from the five EGM stages) and of the second round of the repository
reviewers (`architecture.md`, `security.md`, `correctness.md`) has one answer below. Answers:

- **FIX**: the critique holds; the change is made in the place named.
- **FIX + OWNER**: the critique holds and is product, scope or release; the design carries the recommended option, marked
  pending, and the decision is an owner question (O041-…, design "Owner decisions needed", research.md).
- **CONTEST**: the critique, or the part named, does not hold; the reason cites what a reader can check.

Every claim about the code in the revision was read in the tree at `b1b2c249`. New registers: D041-028 to D041-052,
G041-008 to G041-014, O041-014 to O041-023 (`specs/041-step-logs/research.md`). New files:
`specs/041-step-logs/contracts.md`, `specs/041-step-logs/data-model.md`. The ADR text that changes is in a
"Revision (2026-11-15), proposed" section of ADR-0084 and ADR-0085, awaiting O041-014; their Status lines are unchanged.
Owner record: the Status cells of OD-2026-11-13-9, -14 and -18 in `docs/owner-decisions.md` now read `Decided`, as
`citable()` of `scripts/check-design-docs.py` requires; what is built is the task list's to say. OD-2026-11-13-9's
Covers no longer says "(to be written)" and OD-2026-11-13-14's no longer lists T041-2, whose gate is now the owner's
answer to O041-014. No decision of the owner was changed.

## 1. Blockers

| Id | Answer | What changed, where |
|---|---|---|
| A-B1 | FIX | `contracts.md` §1 (every proto message, field number and enum value, including `retry_after_ms` and the one-field `StepLogPolicy`), §2 (the four operations, item schemas, stream states, `heldBack`, SSE events and `Last-Event-ID`, parameter formats, the delete response, the `ScriptExecution` and `Quota` changes), the error table with existing codes only (`FORBIDDEN`, `PII_REVEAL_NOT_GRANTED`, `NOT_FOUND`, `BAD_REQUEST` as every cursor, `VALIDATION_FAILED`, `RATE_LIMITED` with the `STREAMS_*` reasons of `execution.rs:1264-1269`) and one new reason, `STEP_LOGS_NOT_AVAILABLE`, added the way reasons are added (the `Error.details` description and the remediation table); `FEATURE_DISABLED` is gone. §3 the end rule of the poller (`complete`). Design sections 1, 5; T041-3 copies it (D041-047) |
| A-B2 | FIX | Design section 6: two tables, per SDK, of the logger today (path:line) and what each task adds (Java default methods for fields and `Throwable`, `trace` in TypeScript, Go and Python, the Rust `StepLogger` signature), the level mapping, the capture point against the host's enabled check, exception text, context as fields, identifiers not captured, option names; the retry spacing, `RETRY_LATER` not counted, re-enabling after `STOP_CAPTURE`, the process bound of 16 MiB; the algorithm left to each SDK, equal to `redact` by goldens, cost measured in T041-9. FR-041-001, -003, -007, -015 corrected; a line gains an `exception` text (D041-042) |
| A-B3 | FIX | The write is dropped (D041-030): no step-log row for a script step; `listRunLogStreams` with `source = script` lists the run's `script_executions`, paged on its own (FR-041-011, FR-041-016, section 7, T041-13). The nullable key, the lock-order exception and the barrier test go. Whether the whole-run tab should still interleave a script row is O041-019 |

## 2. Majors

| Id | Answer | What changed, where |
|---|---|---|
| A-M1 | FIX | The end rule is D1958's: `end = pg_current_xact_id()` taken by the first read that sees the run finished, carried in the cursor, `xmin > end` (FR-041-012, section 5 "End of a read", contracts.md §3, D041-031, ADR-0084 Revision decision 9, T041-12 with the equal-id case) |
| A-M2 | FIX + OWNER | No locking path and no band: the tenant row is never taken before commit; over the allowance (unlocked read or the finalizer's raise) nothing is stored, `tenant_quota` is recorded, `FINAL_ONLY` (FR-041-008, section 2 steps 6 and 10, D041-029, ADR-0084 Revision decision 3). The wait bound is stated as the commits queued ahead (4 × R) and the secrets-lock queue (section 2 "What a completion can wait for"). Step logs filling the shared quota: O041-015 |
| A-M3 | FIX | T041-31: the spike runs on a scratch schema before T041-5, with workloads, the harness change (`scalability-ha` completes in process with `{}`), the pass rule, the comparison with chunk rows, and one contingency (stop, change, review again). The grouped commit is no longer the named fallback (D041-043; Open Questions; Alternatives) |
| A-M4 | FIX + OWNER | `run_log_usage FOR UPDATE` kept; the fan-out case (200 attempts, 64 at a time) and the membership MultiXact rate in T041-31; Risks rows; G041-009; the approximate run ceiling is O041-020, asked only if the fan-out fails |
| A-M5 | FIX | `DROPPED` removed; the server counts gaps; the final batch carries `highest_sequence`; a ceiling answers `FINAL_ONLY`, which still allows the empty final batch; `ceiling_sequence` splits worker and server drops; empty-buffer, field-map overflow and oversized-batch rules stated (FR-041-004, -006, section 1, section 2 steps 4-5 and 9, section 6, data-model.md §5, D041-028) |
| A-M6 | FIX | One combination rule (FR-041-020); `StepLogPolicy` has only `enabled`, derived at registration; the SDK numbers are constants of `sdks/SPEC.md`; `step_log_attempt_records = 0` turns capture off and is tested before the stream row (section 2 step 3; D041-033; CUJ 8) |
| A-M7 | FIX | `data-model.md`: DDL with types, keys, nullability, closed sets and `minLevel` order, the `run_log_usage` creation path, the classified-namespace table and its owner (T041-5, T041-28), indexes stated honestly (section 5 "Filters"). The design keeps invariants (D041-047) |
| A-M8 | FIX | `data-model.md` §6: five resources (name, scope, kind, unit, range, default, meaning of 0); T041-4 restates both CHECK lists and the OpenAPI enums; the `Quota` description's exception for retention (contracts.md §2); ingest step 3 reads the namespace policy; the purge resolves retention in Rust (section 4); the open question is closed |
| A-M9 | FIX | Triggers on `step_log_lines` only; owner path the migration in T041-5, the adapter in T041-6 (data-model.md §3, section 3) |
| A-M10 | FIX | Purge per namespace over `(tenant_id, namespace_id, last_at) WHERE purged_at IS NULL` and a deleted-streams index; no stream-row retention and no stored `expires_at` (stage 8's rejections followed); 500 lines per transaction (section 4, D041-038) |
| A-M11 | FIX + OWNER | O041-016: the fixed per-row charge (`fixed_octets`, an ADR-0027 amendment) is recommended, because a quota bypass is release-blocking (constitution V); data-model.md §3 carries it as pending; Constitution Check V names it as a release-blocking condition until answered. With `DROPPED` and the script marker gone, the only zero-byte rows left are empty lines and one ceiling marker per attempt |
| A-M12 | FIX | Insert-only `classified_namespaces`, both publication sites named (`definitions.rs:547`, `execution/ad_hoc.rs:34`), the capability `classified-namespaces-v1` of the instance registry, a resumable backfill pass that walks legacy versions too, fail closed until it completes; no true-to-false path exists (no `UPDATE` grant) (FR-041-021, section 3, D041-037, G041-010, T041-28, T041-26) |
| A-M13 | FIX | `database_oldest_transaction_age_seconds` and `SagatDatabaseOldTransaction`; the PostgreSQL settings in section 5 and the runbook; `heldBack` on the page; the bounded "may be incomplete" end rejected as stage 8 advised (G041-012) |
| A-M14 | FIX | Order: id, lease (namespace, `steps.activity`), session with namespace and expiry, policy, stream, plan, allowances, authority last; replaced-token rule (SDK resends with its current token); `current_worker_role` named as the `pub(crate)` exposure (section 2, D041-032, contracts.md §4) |
| A-M15 | FIX, partly CONTEST | One `Class::Logs` outside `Shared.in_flight` with `SAGAT_GRPC_MAX_LOG_CALLS`; the gRPC-status-to-SDK-action table (contracts.md §4); `supports_step_log_protocol` named (section 1, D041-034). Contested part: the per-tenant count cannot live in the admission point, which runs before authentication (`limits.rs:548-563`); it is a ticket set in the handler built as `TaskStreams` (`limits.rs:47-129`), taken without waiting |
| A-M16 | FIX | Sealing dropped: a plain cursor with the run id compared, as every other cursor (`primitives.rs:193-209`); public stream id a UUID (section 5, contracts.md §3, D041-035; G041-008 for what the position still shows) |
| A-M17 | FIX | `insert_audit` with `collapses_repeats` and `repeat_window` (300 s) under its advisory lock; the record holds no filter; a race test in T041-11 (section 5 "Audit", D041-036) |
| A-M18 | FIX | The missing permission from the session's permissions (no request); the missing grant from the first request's `403 PII_REVEAL_NOT_GRANTED` (CUJ 5, section 8, T041-22) |
| A-M19 | FIX | `x-planned: '041'` until served; `body_of` gains each served operation; the digest approves both tags; T041-29 states why the digest moves (descriptions only) (section 5 "Contract", D041-044) |
| A-M20 | FIX | The seven `WorkerService` implementations, the regenerated Go and Python stubs with their recorded tool versions, `StepLogField` and its `oneof` in `skip_debug` (section 1, T041-3) |
| A-M21 | FIX | `scrub_shapes_with(text, placeholder)` and a public `redact_credentials`; non-string values under a sensitive name become `[REDACTED]` (FR-041-007 (c), section 2, D041-045) |
| A-M22 | FIX | Behaviours, the `stepLogs` op, the `verdict` fault in each SDK's fault plan, `mustNotStore` over stored lines, routes and the server's output; the server canary in T041-8 (section 6 "Conformance") |
| A-M23 | FIX + OWNER | Each SDK task moves its constant (paths in T041-15 to T041-20); the negotiation golden and §7.1 move in T041-7. The holder of 1.7 is O041-018 |
| A-M24 | FIX | `SAGAT_TEST_POSTGRES_TAG` override in `tests/support/mod.rs` and `bounded.rs`, and a CI step with `18-alpine` (T041-11, D041-048) |
| A-M25 | FIX | Selector rule and the `integration_step_logs` target in T041-6 (D041-048) |
| A-M26 | FIX | A `Files:` line in every task that changes code (29 of 31) |
| A-M27 | FIX | Each task one pull request except T041-2; 31 tasks, 30 pull requests; the Estimate's parts add up (14 + 8 + 3 + 5) |
| A-M28 | FIX | The checkers, the two switches and the four lock files named; the browser target `e2e_workflow_studio`; the validation-report path; T041-27's reviewers and output folder; the expected failure of each scenario; T041-1 and T041-2 have done conditions |
| A-M29 | FIX + OWNER | "Owner decisions needed" lists O041-014 to O041-023; T041-2 no longer cites OD-2026-11-13-14; the OD Status cells fixed as above. Acceptance of the ADR revision is O041-014 |
| A-M30 | FIX | This file is the finding-to-section trail; the review history left the Overview, which now points here; the first-round findings whose text was lost are listed in section 4 |
| A-M31 | FIX | FR-041-002 (transaction-id order; `ORDER BY sequence` in step 8); Overview and Goals restate the wait bound; the secret claims limited to verbatim values of at least 6 bytes with G041-001 (Goals, CUJ 1, FR-041-007); FR-041-010 defines the horizon that FR-041-011 cites |
| A-M32 | FIX | "Terms" at the start of the Technical Plan: attempt and generation, instance path, stream and tail, storage against data classification, store proofs, the namespace secrets lock, the content and log fences; each lock in the lock order is taken by a numbered step |
| A-M33 | FIX | "Stream" is the attempt log, "tail" the SSE connection (metrics `step_log_tails_*`); `{runId}` throughout; `run.logs_deleted` for the delete and "purge" for the removal (FR-041-023, ADR-0085 Revision decision 9) |
| A-M34 | FIX | Alternatives Considered links the ADRs' options and lists, with a reason each, the options this design weighed on its own; the owner's rejected options carry a reason (stage 8's advice not to copy every ADR option followed) |
| A-M35 | FIX | Risks rows for default-on capture, redaction drift, a hand-written client, the viewer break, a failed backfill, `ResolveDatabase` on ordinary SDKs, fan-out, zero-byte rows, the shared quota |
| A-M36 | FIX | FR-041-007 lettered; FR-041-008 cut to its obligations; the negotiation paragraph split; "Before the transaction" numbered; T041-6, T041-8 and T041-11 grouped by suite and case |

## 3. Nits

| Id | Answer | What changed, where |
|---|---|---|
| A-N1 | FIX | `logTruncated` false when withheld, on the get and the list (FR-041-022, D041-040); the incident path names `script_executions.log` (FR-041-023, CUJ 7, section 9) |
| A-N2 | FIX | T041-10 is before T041-7, so `SAGAT_STEP_LOGS_ENABLED` has one default (section 10) |
| A-N3 | FIX | The tail re-checks every `StreamLimits::recheck` (30 s), stated in FR-041-012 (D041-041) |
| A-N4 | FIX | "No log" comes from the streams route; "ended without its final flush" is defined (FR-041-017); the iteration is picked from the streams list (CUJ 6); one flood figure (1,000,000) |
| A-N5 | FIX | The budget figures explained (The Problem fact 3); vague quantities replaced (CUJ 2, the lag field `heldBack`, the typed error of CUJ 5) |
| A-N6 | FIX | Links to research.md, the ADRs and the contracts in the Overview; G69 and G1618 qualified; every gap listed in the Verification Plan |
| A-N7 | FIX | Owner quotes translated |
| A-N8 | FIX | Wrapped code spans joined; NFR titles; the diagram shows the purge and the script read; CUJs cite their stories, Goals their criteria; CUJ 8 covers the rest of US03 |
| A-N9 | FIX | One term each: capture level, deployment switch against namespace resource, record against line |
| A-N10 | FIX | Constitution Check VII says why no dead-letter rule applies; II and VIII say "to be verified" |
| A-N11 | FIX | The migration anchor rule is in data-model.md §2 |
| A-N12 | FIX | `scripts.rs:426-448`, `execution.rs:1017`, `dto_scripts.rs:582`; `Refs:` sorted; the Verification row for reset and rerun names its method |
| A-N13 | FIX | T041-8's restart outcome; "every stream's run exists" in T041-26; the tests-first rule stated for the plan |
| A-N14 | FIX | Canonical JSON and the number form in data-model.md §5 (D041-046) |
| A-N15 | FIX | `ApiOperation::RunLogsDelete`; `follow` refactored to a shared loop; lock order of delete and purge (sections 2, 4, 5) |

## 4. The second round of the repository reviewers

Findings that `consolidated.md` maps to an action are answered with it; the others are answered here.

| Report, id | Answer | Where |
|---|---|---|
| architecture M1 | FIX | A-M2 (bound restated; K held ingests in T041-6 and T041-31) |
| architecture M2 | FIX | A-M10 (500 lines per purge transaction; completion p99 during a purge in T041-9) |
| architecture M3 | FIX + OWNER | A-M29; O041-014. The Status lines stay as the user asked; the Revision section is marked "proposed" |
| architecture M4 | FIX | `log` stays required (D041-040); T041-29 says no check covers component schemas |
| architecture M5 | FIX | A-M12 |
| architecture M6 | FIX | `plan_batch` and `validate_batch` in the domain with property tests; `StepLogTx` only reads and writes; `first_uncovered` and the role rule reused (section 2, D041-039) |
| architecture M7 | FIX | Source-text test (T041-5), graph-stream contract test (T041-12), ADR-0084 "Enforced by" in its Revision |
| architecture N1, N9, N11 | FIX | Moot: no script write (A-B3) |
| architecture N2 | FIX | SC-041-001 says the ingest runs between the measuring windows |
| architecture N3 | FIX | A-M9 |
| architecture N4 | FIX | A-N11, A-N12; the upgrade-boundary oracle in T041-5 |
| architecture N5 | FIX | A-M8 (the switch is the zero of a `Policy` resource with a stated exception) |
| architecture N6 | FIX | A-M15 |
| architecture N7 | FIX | A-N3 |
| architecture N8 | FIX | "any open transaction of the cluster, prepared ones included" (section 5, G041-012) |
| architecture N10 | FIX | A-N15 |
| architecture N12 | FIX | A-N13 |
| architecture N13 | FIX | A-M14 |
| architecture N14 | FIX | FR-041-001 and section 6 (capture independent of the host's enabled check; `step-logs-capture` sets the host level above the capture level) |
| architecture N15 | FIX | Metric enums in `crates/telemetry/src/metrics.rs` (data-model.md §7); T041-25 names `docs/architecture/overview.md` |
| architecture N16 | FIX | FR-041-022 depends only on the reader's right, not on the read switch (section 5) |
| security r1-M1 | FIX | A-M14 |
| security r1-M4 | FIX | D041-046 (ECMAScript form); scenario numbers 12345678 and 1e15 |
| security M1 | FIX + OWNER | O041-017 (the process-lifetime set, bounded, fail closed, recommended); `step-logs-redaction` has an attempt after another ended; ADR-0085 Revision decisions 1 and 12 |
| security M2 | FIX + OWNER | A-M11, O041-016 |
| security M3 | FIX | A-M12 (insert-only table; no grant on `namespaces`) |
| security M4 | FIX | A-N1 |
| security M5 | FIX | A-M15 (own counter; tickets taken without waiting) |
| security M6 | FIX, partly CONTEST | Public stream id is a UUID. Contested: an encrypted cursor; no cursor of the API is encrypted (`primitives.rs:193-209`), and the position must order by `stored_xid`; what it shows is recorded as G041-008 and as a condition on principle V |
| security r1-N4 | FIX | A-N3 (the window is stated) |
| security r1-N5 | FIX | A-M20 |
| security N1 | FIX | Only `StepLogger` calls are captured; no `tracing` layer (FR-041-001, section 6) |
| security N2 | FIX | A-M21 |
| security N3 | FIX | Collision rule `~2`, `~3` (data-model.md §5) |
| security N4 | FIX | `RunLogsDelete`; a `stream` of another run is 404 (contracts.md §2) |
| security N5 | FIX | The purge does not skip a leased stream (section 4) |
| security N6 | FIX | The binding lock is in the lock order and taken by step 7 |
| security N7 | FIX | Section 1 corrected for admin; `step-logs-negotiation` asserts both tokens |
| security N8 | FIX | `event: progress` carries the scanned position (section 5, contracts.md §2) |
| security N9 | FIX | `current_worker_role` refuses runner in the locked read |
| correctness r1-M3 | FIX | A-M2 |
| correctness r1-M5 | FIX | `GREATEST` and the late-batch test (D041-028, T041-6) |
| correctness M1 | FIX + OWNER | O041-015 (reserve recommended); G041-011 |
| correctness M2 | FIX | Authority last before the insert; a barrier test with a `lock_secrets` waiter; the wait stated (section 2) |
| correctness M3 | FIX | A-M14; a re-registration case in `step-logs-transport-faults` |
| correctness M4 | FIX | A-M5 (`FINAL_ONLY`; drop counts no longer clamped at the ceiling) |
| correctness M5 | FIX | Step 4 refuses a removed stream under the purge's row lock; T041-10 test |
| correctness M6 | FIX | Moot: no script write (A-B3) |
| correctness M7 | FIX + OWNER | Security M1, O041-017 |
| correctness M8 | FIX, partly CONTEST | A-M4 (measured, Risks, O041-020). Contested: making the run-usage update the last statement; the row is updated in the same transaction and so locked to commit either way, as stage 8 noted |
| correctness r1-N7 | FIX | A-M5 |
| correctness r1-N13 | FIX | The Revision sections say "proposed, awaiting the owner" (the Status line is unchanged, as asked) |
| correctness N1 | FIX | A-M31 |
| correctness N2 | FIX | A resent final batch gets `FINAL_RECORDED`, which the SDK takes as success (FR-041-004) |
| correctness N3 | FIX | The duplicate test asserts `RETRY_LATER` then a resend that stores nothing |
| correctness N4 | FIX | `StepLogStopReason` (contracts.md §1) |
| correctness N5 | FIX | Moot: no `DROPPED` record |
| correctness N6 | FIX | Moot: no script stream; the stream key has no nullable column |
| correctness N7 | FIX | `INSERT … ON CONFLICT DO UPDATE … RETURNING` for `run_log_usage` |
| correctness N8 | FIX | A-M17 |
| correctness N9 | FIX | Security N9 |

**First-round findings whose text was lost in the relay** (2026-11-13 reports, "not described"): architecture M8, N2, N3,
N4, N5, N7, N11, N13; security M5, M6; correctness N2, N5, N9, N14. Their text is not in the record and cannot be
recovered from it. They are not claimed answered. The second round and this re-audit read the whole design afresh, which
is the only way left to catch what they said; nothing they raise is known to be open.

## 5. Re-audit (one round, the EGM cap)

Five fresh agents read the revision (design, contracts.md, data-model.md, research.md, the ADRs) and their earlier
reports, not this file: `implementation-readiness-reaudit.md` (Blocker 0, Major 14, Nit 10), `design-reaudit.md`
(Blocker 0, Major 8, Nit 10), `architecture-reaudit.md` (Blocker 0, Major 1, Nit 10), `security-reaudit.md` (Blocker 0,
Major 2, Nit 6), `correctness-reaudit.md` (Blocker 0, Major 5, Nit 8). No Blocker. Every finding was checked against the
code or the text before it was answered; each answer below is in the design, contracts.md, data-model.md, research.md
(D041-049 to D041-052, G041-013, G041-014, O041-022, O041-023) or the ADRs' Revision sections. The EGM cap allows one
re-audit, so the revision was not reviewed again; what is not fixed is recorded in section 6.

| Report, finding | Answer | Where |
|---|---|---|
| architecture M1, security M2, design D, IR 16, architecture N7 | FIX | The classified record is an `AFTER INSERT` trigger on `workflow_versions` with a superset test (security's smallest fix), and the migration records existing versions; no backfill, no registry capability, no fail-closed window (FR-041-021, section 3, data-model.md §2, D041-037, G041-010 rewritten, T041-5, T041-28) |
| security M1 | FIX | Step 2 matches run, node and instance path as `lease_unexpired_instance` does, and later steps take them from the row; a T041-6 case (D041-032) |
| design C | FIX | Retention pinned per stream at creation, as ADR-0027 requires; `expires_at` stored; CUJ 8, FR-041-009, FR-041-020, section 4 and the `Quota` text corrected; ADR-0084 Revision decision 8 (D041-049) |
| design A, correctness M2, architecture N10 | FIX | `end` taken in a short transaction of its own after the read saw the run finished; a forged `end` for a running run ignored; T041-12 tests (D041-051) |
| correctness M1, design N5 | FIX | The second transaction re-plans under the stream lock with a tenant allowance of 0 and sets `ceiling_sequence`; both tenant branches set it; property test (D041-050) |
| correctness M3 | FIX | The purge marks (`removed_at`) first, then deletes 500 lines per transaction; readers and the ingest honour the mark (section 4, data-model.md, D041-038) |
| correctness M4 | FIX | SC-041-001 and the Verification row assert, through `pg_locks`, that the completion queues behind K committed ingests, with the test's lock timeout raised, within the bound T041-31 measures |
| correctness M5 (r1-M8) | FIX, partly CONTEST | FR-041-008 states the per-run wait. Contested: moving the run-usage row after the authority locks, which would make every batch hold the namespace-wide secrets lock while it waits for a per-run row (section 2, lock order) |
| design B | FIX | The Goal and SC-041-003 are narrowed to attempts that have a stream; a full secret set still sends the empty final batch; G041-014 |
| design E, architecture N9 | FIX | Defaults stated (cap a quarter of the pool, tickets half the cap); the cap is what keeps lease calls' connections; K defined; a lease-p99 test (D041-052) |
| IR 4 | FIX | T041-3 adds the operations to `not_built`; T041-11, T041-12, T041-30 remove them |
| IR 5, architecture N3 | FIX | A `minimum()`, a named zero exception for `step_log_attempt_records`, and `quota_limits_tenant_scoped` in T041-4 (data-model.md §6) |
| IR 14 | FIX | T041-9 is a new `perf_step_logs` target that starts the platform as `sdk_conformance` does |
| IR 20 | FIX | `stepLogs` is a plain HTTP request each runner makes; the runner files are in T041-15 to T041-20; the first failure is "unknown op" (section 6, T041-14) |
| IR 23, architecture N8 | FIX + OWNER | O041-022 (the constitution's CI gate allows only the suites it names); T041-6 builds the rule only on answer (a) |
| IR 26, IR 30, IR 35, IR 37 | FIX | One failure rule for all five scenarios; the minor register in SPEC §9 and the two "10" headings renumbered; T041-6 split into Builds and Tests; the release-note wording |
| IR N1 | FIX | `docs/reference/api.md` regenerated in T041-3, T041-11, T041-12, T041-29, T041-30 |
| IR N2, architecture N4 | FIX | The source-text test forbids the table names and `StepLogTx` in the production code of the named lease and completion files |
| IR N3, correctness N11 | FIX | `current_worker_authorized` gains a refused-roles argument; `share_secrets_lock` keeps its one caller and the test passes unmodified |
| IR N4 | FIX | The read switch in the control plane's configuration, CLI and test (T041-11) |
| IR N5 | FIX | Crash points through a `StepLogTx` wrapper of the test support; lost responses through the conformance proxy (T041-8) |
| IR N6 | FIX | `step_logs::may_read`, built in T041-11, used by T041-13 and T041-29 |
| IR N7, architecture N2 | FIX | "Waits for" on every task that builds a revised decision or a decision still open (O041-014, O041-018, O041-019, ...); O041-014 covers both revisions and cites `docs/adr/README.md` rule 2 |
| IR N8 | FIX | T041-6's fixtures insert `worker_sessions` rows at minor 7 |
| IR N9 | FIX | T041-31's driver runs only with `SAGAT_PERF_STEP_LOG_SPIKE=1` and makes its scratch tables in the bench's own database |
| IR N10 | FIX | `docs/releases/unreleased.md` |
| IR N11, security N4 (Rust part) | FIX | `skip_debug` in `sdks/rust/build.rs` and its conformance copy (T041-3) |
| security N4 (other languages) | CONTEST | A check that the worker's own log holds no canary cannot hold: the host log keeps receiving the activity's own lines, unredacted, as it does today (FR-041-001); `mustNotLeak` already excludes it for that reason (`harness.rs:959-960`) |
| IR N12, N13, N14 | FIX | Anchors and Files lines; T041-3 alone owns the OpenAPI enums; `task_id` stated as carried, the step found by run, step and instance |
| architecture N5 | FIX | Horizon tests in a disposable container of their own |
| architecture N6 | FIX | FR-041-022 holds whatever the read switch says |
| architecture N11, design N7 | FIX | research.md's review note on G041-003 and G041-007; ADR-0084 Revision replaces "Enforced by", corrects Consequences and Evidence; step 3 reads five values |
| security N3 | FIX | Answers decided before step 7 are given only after it passes; the second transaction repeats the checks (D041-050) |
| security N5 | FIX | `store_error_without_text` on every statement that binds line text or `q`; a forced error in T041-8 |
| security N6 | FIX + OWNER | No row identity on lines; the cursor and the index use (`stored_xid`, `sequence`); the xid part is O041-023 |
| security N7 | FIX | ADR-0085 Revision rule 6 scopes "how large" to runs and streams; the tenant total stays readable under `quota:read` |
| security N8 | FIX | The cross-tenant audit goes through `record_denial`, outside the ingest transaction |
| design 3, 4, 9, 24, 27, N8 | FIX | Risks rows (membership MultiXact, `quota_transaction_deltas` bloat, stream and usage row growth); T041-31 records MultiXact and the held-transaction case; Constitution Check V qualified; a 1 ms p99 criterion for redaction in T041-9; G041-013 for script logs |
| design N1 | FIX | The session is read without a lock; the policy reads take no fence, and why |
| design N2 | FIX | `fixed_octets` on `level`, a `NOT NULL` text column the manifest test compares |
| design N3, correctness N6 | FIX | `records` counts worker lines only; the rule says "lines stored (the marker excluded)" |
| design N4 | FIX | `last_at` is the server's time of every recorded batch; `expires_at` follows it |
| design N6 | FIX | Fact 3's labels |
| correctness N7 | FIX | The end test restated as the largest ingest id below `end` |
| correctness N8, N9, N10, N12 | FIX | `FINAL_ONLY` to a final batch is its success; `highest_sequence` bounded at 2^62-1; the tenant limit worked out as the finalizer does, with `log_enforcement_enabled`; the lock-order row |
| correctness N13 | CONTEST | Line 3 (Status) of both ADRs is not changed, as the request for this work required; each Revision (2026-11-15) section is headed "proposed: awaiting the owner's acceptance (O041-014)" and says the decisions above stay in force until then |

## 6. Residual findings, accepted as trade-offs

The one re-audit is spent. These remain, each a trade-off recorded rather than a defect left unexamined:

1. **The trigger's superset test** (G041-010; Nit). A document that quotes `"x-sagat-classification"` outside an
   annotation makes its namespace need the PII grant. It errs closed by design; no reviewer has read the trigger itself.
2. **The run-usage row before the authority locks** (correctness M5, part contested; Major, answered by measurement).
   A run's batches wait for each other; T041-31's fan-out case decides, and O041-020 is the owner's if it fails.
3. **The transaction counter in the cursor** (G041-008, O041-023; Nit). The owner decides.
4. **The worker's own host log can hold the canary** (security N4, contested). By FR-041-001 the host log is unchanged.
5. **ADR Status lines unchanged** (correctness N13, contested). By instruction; the Revision headings carry the state.
6. **Fixes of the re-audit not reviewed again.** The changes of section 5 (the trigger, pinned retention, the end
   transaction, the mark-then-delete purge, the re-plan after the raise) were made after the re-audit and have had no
   independent read; T041-27 and the per-slice reviewers are the next check.

## Counts

Consolidated actions: 54 (Blocker 3, Major 36, Nit 15): FIX 48 (3 Blockers, 30 Majors, 15 Nits), FIX + OWNER 5 (A-M2,
A-M4, A-M11, A-M23, A-M29), FIX partly CONTEST 1 (A-M15), CONTEST 0. Second-round repository findings: architecture 23,
security 19, correctness 21, each answered on its own or through an action; partly contested 2 (security M6, correctness
M8). Re-audit: 30 Majors and 44 Nits raised; Majors fixed 28, partly contested 1 (correctness M5), owner 1 (IR 23, with
O041-022); Nits fixed 41, contested 2 (security N4 in part, correctness N13), owner 1 (security N6 in part, O041-023).
Open after the re-audit: Blocker 0, Major 0 without an answer.

T041-1 stays unticked although its done condition holds: `scripts/check-design-docs.py` refuses a ticked task in a doc
that is not `Approved` (R9, "T041-1 is newly ticked but the doc is not Approved", seen on 2026-11-15), and the design is
`Draft` until the owner answers O041-014 to O041-023 and the approval fields are written.

## 7. The owner's answers and the final review (2026-11-15)

The owner answered O041-014 to O041-019, O041-022 and O041-023 (OD-2026-11-15-4 to -11; two against the recommendation:
the separate quota and the script marker). Commit `35070fe9` built them in: the ADR revisions marked accepted; an
Amendment of ADR-0027 for the separate quota and the 128-byte row charge, its text for the owner's acceptance (O041-024);
the script marker specified (design section 7); constitution 4.5.0 with its Sync Impact Report and task T041-32; the
register of minors in `sdks/SPEC.md` §9 and a note in the research of drafts 020, 021, 025 and 026. Three fresh
reviewers then read the diffs `b1b2c249..69e0f828` and `69e0f828..35070fe9` and their re-audit reports:
`architecture-final.md` (Blocker 0, Major 3, Nit 8), `security-final.md` (Blocker 0, Major 2, Nit 3),
`correctness-final.md` (Blocker 0, Major 4, Nit 8). Answers, in the commit that adds this section:

| Finding | Answer | Where |
|---|---|---|
| architecture M1 | FIX + OWNER | The Amendment gains considered options and consequences (2 GiB of diagnostic text per tenant by default) and the rule-2 trade-off; O041-024 gains option (c), a new ADR refining ADR-0027. The text stays in ADR-0027, as the owner asked |
| architecture M2, correctness N5, N7 | FIX | Both 2026-11-15 Revision headings say "accepted 2026-11-15 (OD-2026-11-15-4)"; the "not in force" sentence is gone; ADR-0084's note gives the readings of decisions 8 (`step_log`) and 11 (the fences of the classes charged) and the end test that follow from the owner's answers |
| architecture M3, correctness M1 | FIX | Section 7: one record and one marker per (run, node, attempt), as `script_executions_step_uniq` keeps; G041-007 restated and owned by feature 005; a two-iteration test |
| security A, architecture N6 | FIX | The reported path and generation must match a `step.leased` event of the run, or no marker (`skipped_identity`); a test |
| security B, correctness M2, architecture N2 | FIX | The Amendment names the routing branch, the `add_delta` argument with a default, the loop filter, the raise on an unknown class, and two `SECURITY DEFINER` accessors (`sagat_quota_step_log_allowance`, `sagat_quota_lock_step_log_usage`); data-model.md §4 corrected; a runtime-role run in T041-13 |
| correctness M3 | FIX | The attempt and run byte ceilings count text only; the 128 bytes are the tenant quota's (FR-041-006, section 2 step 6, data-model.md) |
| correctness M4 | FIX | The barrier ingests are held after `SET CONSTRAINTS ALL IMMEDIATE`, so their flush holds the usage row (SC-041-001, T041-6, Verification) |
| architecture N1, security C | FIX | The trigger is `ENABLE ALWAYS`, with a fixed `search_path` and qualified names; trigger and backfill in one transaction; T041-28 tests both |
| architecture N3 | FIX | `identity.rs` (`classify`), `quotas.rs` and `content_usage.rs` in T041-5's Files |
| architecture N4 | FIX | OD-2026-11-13-18's Covers names T041-5; OD-2026-11-15-9's names T041-22, which cites it; T041-3 and T041-13 wait for O041-024 |
| architecture N5 | FIX | Clause 4.5.0 and its report name the suites; T041-32 carves `worker.proto` out of `SHARED_INPUTS`; the report states the cost for later protocol changes |
| architecture N7 | FIX | `script_marker_decision`, a pure function beside `plan_batch` |
| architecture N8 | FIX | "six resources" |
| security D | Residual | G041-015: the step-log quota's usage under `quota:read` is a step-log-only signal; ADR-0085 decision 6 (accepted) names only the tenant total. For the owner if it matters |
| security E | CONTEST | `pg_snapshot_xmax` is the end horizon D1958 removed: a snapshot's `xmax` is the last completed id plus one, so an ingest with a higher id can still be open when `xmin` passes it. The cost is one transaction id per read of a finished run whose cursor lacks `end`, bounded by the API rate limit |
| correctness N6 | FIX | The Amendment: a transaction charging several classes takes their fences in the order content, log, step-log |
| correctness N7 (skipped marker) | FIX | A skipped marker is counted by `step_log_script_markers_total` and the Amendment says so |
| correctness N8 | FIX | The marker stream's `expires_at` is never later than the execution's |
| correctness N9 | FIX | FR-041-023: the delete removes what was committed when it ran |
| correctness N10 | FIX | The count rule is stated for highest sequences below 2^53 |
| correctness N11 | FIX | The marker reads usage through `sagat_quota_lock_step_log_usage` and sets `last_at` and `expires_at` |

Residual after the final review: Blocker 0; Major 0 without an answer (architecture M1 is answered and its choice is the
owner's, O041-024); Nits: security D (G041-015) and security E (contested). The fixes of this section had no further
independent read: the EGM cap of one re-audit is spent, and T041-27 and the per-slice reviewers are the next check.

## 8. The approval run (2026-11-15) and why the design stays Draft

The owner accepted the ADR-0027 Amendment in place (OD-2026-11-15-16) and the design with constitution 4.5.0
(OD-2026-11-15-17), recorded in commit `882e56fc`. Approval (`docs/designs/constitution-4.0.0-egm.md` section 3) needs a
complete run of all five egm-review stages and the three repository reviewers on a committed text after the last fix,
ending with no Blocker and no Major open, a Major counting as settled only when the owner accepts that finding in
writing, one owner-decision row per finding. That run was made on `882e56fc`:

| Report | Blocker | Major | Nit |
| --- | ---: | ---: | ---: |
| `egm-review-approval.md` (stages in `egm-stages-5-7-approval.md`, `egm-stage-8-approval.md`, `egm-stage-9-approval.md`) | 0 | 49 | 80 |
| `architecture-approval.md` | 0 | 0 | 5 |
| `security-approval.md` | 0 | 0 | 3 |
| `correctness-approval.md` | 0 | 1 | 3 |

The repository reviewers are clean but for one Major (the barrier test of SC-041-001). The egm stages raise 49 Majors,
not deduplicated, mostly of the task plan (Files, task order, test facilities the plan names that do not exist, the
control plane's copy of `SAGAT_STEP_LOGS_ENABLED`, the `recordScriptExecution` contract owner) and of wording (the
completion bound, the step-2 control flow, "no `SECURITY DEFINER`"), and stage 8 proposes two lighter mechanisms (a
script marker keyed on the execution; `RETRY_LATER` instead of a second transaction after the finalizer's raise).
OD-2026-11-15-17 accepts the design as a whole; it is not a written acceptance of these findings, so the record cannot
cite it in `Accepted`. The design therefore stays `Draft`, T041-1 and T041-2 stay unticked (the checker's R9 refuses a
tick in a doc that is not `Approved`), and no approval block was written. The findings are open: none was fixed after
this run, so that its reports describe the committed text. Approval needs either a revision that answers them and a new
complete run, or the owner's written acceptance of each remaining Major.

## 9. Answers to the approval run on `882e56fc`

Every finding of the approval reports, deduplicated, with one answer. FIX: the change is made where named. CONTEST: the
finding, or the part named, does not hold, for the reason given. Each claim about the code was read in the tree at
`65c59a4f`. New registers: D041-059 to D041-066 and G041-016; G041-005, G041-007 and G041-008 restated; G041-015 closed
(`specs/041-step-logs/research.md`). No owner question is raised: none of the answers decides product, scope or release,
and O041-020 and O041-021 stay conditional on T041-31. Ids: Z, C, K, S, T are stages 5 to 7
(`egm-stages-5-7-approval.md`), "8-…" stage 8 (`egm-stage-8-approval.md`), IR, CD, ST stage 9
(`egm-stage-9-approval.md`), AR, SE, CO the repository reviewers (`*-approval.md`).

### Majors

| Findings (merged) | Answer | What changed, where |
| --- | --- | --- |
| CO-1 (the barrier test cannot be built) | FIX, D041-059 | One ingest held after its flush, K − 1 seen blocked behind it with a raised lock timeout, then the completion; released one at a time, the completion is blocked only by ingest backends (`pg_blocking_pids`, `tests/support/lock_order.rs:322`) and commits after the K-th. SC-041-001, T041-6 (the `StepLogTx` wrapper's "hold after flush" hook, moved from T041-8 to T041-6), Verification Plan |
| C-1, T-3, ST-2, 8-merge C-1/T-3, IR nit "within the bound" | FIX, D041-059 | "The bound" is two things, named apart: in structure, the ingests queued ahead (barrier test); in time, SC-041-007's completion p99 within 10%. Overview, Goals, Open Questions, section 2, Risks; T041-31 says the per-line variant must pass and the chunk variant only informs O041-021 |
| K-1, 8-merge K-1 | FIX | Goals state the final-flush wait of every logging step and that it meets `RETRY_LATER` with no reserved capacity; section 1 says so; T041-31 models admission with a ticket semaphore and records the `RETRY_LATER` share of final batches and what the final flush adds |
| T-1, 8-merge T-1 | FIX | Section 2 lists the control flow: `LEASE` ends at once; `SESSION`, `CAPTURE_OFF`, `REMOVED`, `FINAL_RECORDED` are noted, skip to step 7 and roll back after it; `FINAL_ONLY` goes on. CONTEST of stage 8's proposal to short-circuit `SESSION` and `CAPTURE_OFF` too: D041-050 defers them so a revoked caller learns only `AUTHORITY`, and keeping that costs nothing |
| T-2, 8-merge T-2, IR-2 (second half), AR-1, SE-2, CO-3 | FIX | "No `SECURITY DEFINER` function beyond the two accessors of ADR-0027's Amendment" in section 3 and NFR-041-SEC |
| 8-missed (marker keyed on the execution), Z-2, CD-4, AR-4 | FIX, D041-060 | Section 7 rewritten: one `source = script` stream per inserted execution, public id = execution id, no step, generation or path; runner fields, resend fallback, verification and `skipped_identity` removed; `recordScriptExecution` unchanged (contracts.md §2), so CD-4 and AR-4 have nothing left to own. OD-2026-11-15-9 holds: the whole-run tab still shows the marker row. data-model.md §1, §2, §7 |
| Z-1 (unlocked run ceiling now) | CONTEST | Exact or approximate is O041-020, which the owner left conditional on T041-31's fan-out case; choosing (b) now pre-empts it, as stage 8 also rejects |
| Z-3 (code-level detail in the Technical Plan) | FIX in part, CONTEST in part | Review tags removed from the text; tool versions moved to T041-3. The file:line anchors stay: stage 9 needs them, and data-model.md says what detail a task may correct without a new review (stage 8 merges it as a Nit) |
| C-2 | FIX | Section 5, "A stream's state": `ended_without_final` from the stream row and the `steps` row of its `step_id`, read without a lock; data-model.md §1 |
| C-3 | FIX | Terms define stream, attempt log, script stream, the `stream` parameter and tail; the script stream's public id is the execution id, so `listRunLogs?stream=` finds a marker; FR-041-010, contracts.md §2 |
| C-4 | FIX | contracts.md `expiresAt`: last batch plus the retention the stream took at creation |
| K-2 (downgraded by stage 8) | FIX, D041-064 | `DELETE` answers `204` with no body; FR-041-023, section 4, CUJ 7, contracts.md |
| S-1 | FIX | Rows II and VIII read `Pass`, with the checks that enforce them as evidence |
| S-2 | FIX | Complexity Tracking row for G041-008; G041-015 closed by D041-066 (SE-1), so principle V and the Risks name it as closed; Risks rows for G041-008, G041-015, G041-016 |
| IR-1 | FIX | T041-4 adds `tenant_retained_usage.step_log_bytes` and the exhaustive arms of `crates/infra-postgres/src/quotas.rs:1211-1287` and `tests/integration/security-multitenancy/quotas.rs:507-598` |
| IR-2 (first half) | FIX | T041-5 names the routing, the `add_delta` argument (drop and recreate, AR-2), the loop filter and the two accessors; data-model.md §3 "Who adds what" |
| IR-3 | FIX | T041-31 defines the prototype (section 2's SQL against a `spike_041` schema), the chunk-row table, and its accounting (the ADR-0027 triggers on the scratch tables, charging the existing `log` class, `durable_quotas.up.sql:444-558`) |
| IR-4, CD-2 | FIX, D041-065 | `isolated_server(tag)` and `fresh_db_on` in `tests/support/mod.rs` (the shared server is one per process, `:100-176`); `bounded.rs:81` is the managed-connection target and leaves T041-11's Files |
| IR-5, CD-1 | FIX | The lost-answer proxy is `tests/integration/database-runner/proxy.rs`, moved by T041-8 to `tests/support/worker_proxy.rs` with a `lose_append_ack` fault; the conformance proxy of `sdks/rust/conformance` is not used |
| IR-6, ST-9 | FIX | The logger-call micro-benchmark and the matcher fallback belong to each SDK task (section 6, Slice D); T041-9's baseline is the same workload with `SAGAT_STEP_LOGS_ENABLED=false`, as SC-041-007 |
| IR-7, ST-7, AR-3 | FIX, D041-063 | Step-log handlers in `crates/api-http/src/routes/step_logs.rs` and `crates/api-grpc/src/step_logs.rs`; T041-32 matches `crates/[a-z-]+/src/(.*/)?step_logs(\.rs$|/)`, `limits.rs`, `worker.proto` and the six named SDK capture files, and lands after T041-6 and before T041-7 |
| IR-8, ST-10 | FIX | T041-26: target `e2e_step_logs` with its expected outcomes, and the older replica through a scenario of `scripts/test-release-upgrade-profile.py` (the v0.2.0 tree at `RELEASE`) |
| IR-9, CD-3 | FIX, D041-062 | The control plane reads `SAGAT_STEP_LOGS_ENABLED`, as it reads `SAGAT_SCHEDULES_ENABLED` (`apps/control-plane/src/cli.rs:190`); FR-041-020, section 7, section 10, T041-13 |
| CD-5 | FIX | The gauge is T041-11's (`operations.rs`, `metrics.rs`), its visibility limit G041-016; purge metrics in T041-10's Files, marker metric in T041-13's |
| CD-6 | FIX | Added to Files: `sdks/rust/src/lib.rs` and `sdks/rust/conformance/src/faults/grpc.rs` (Debug impls, T041-3); `crates/application/src/quotas.rs` (`RetainedUsage`, T041-5); `durable_quotas.rs` and `runtime_privileges.rs` (T041-5); `execution/path.rs` (T041-6, the only other caller of the free `current_worker_authorized`; the trait method keeps its signature); `worker/activity.py` (T041-17) |
| CD-7 | FIX | T041-5's source-text test covers `crates/application/src/worker/` |
| CD-8 | FIX | T041-9 drives the workers of `tests/support/worker.rs` (`platform_with`, `Platform::worker`), taught to send batches by section 6's rules; the private `harness.rs:328` is not used |
| ST-1 | FIX | The plan's rule: every test is seen failing on its own assertion, not a compile error or a missing fixture, and the pull request names the assertion that failed first |
| ST-3 | FIX | T041-11 serves `source = worker` and builds the end rule for `complete`; `source = script` is T041-13's; T041-12 uses T041-11's rule |
| ST-4 | FIX | T041-12 and T041-30 switch their operations to `x-implemented` and leave `not_built` |
| ST-5 | FIX | The lock-order test grows per task: T041-6 (ingest, report, binding), T041-10 (purge), T041-13 (marker), T041-30 (delete); section 2, Verification Plan |
| ST-6 | FIX | T041-10 creates `docs/runbooks/step-logs.md`; T041-30 and T041-25 modify it |
| ST-8 | FIX | Slice D states each SDK task's runner work and done condition |
| Z-4 (Nit), 8-merge Z-4 (the second simplification) | FIX, D041-061 | After the finalizer's raise the answer is `RETRY_LATER`; the resend's plan counts `tenant_quota`; a never-stored final batch leaves "ended without its final flush" (G041-002). ADR-0084 decision 3 holds (no line stored, counted on its stream). Section 2 step 10, T041-6, Verification Plan, Alternatives |

### Nits

| Findings | Answer |
| --- | --- |
| CO-2 (which bytes the stream holds) | FIX: text bytes everywhere; step 8 returns text sizes; FR-041-008, data-model.md, contracts.md `bytes` |
| CO-4 (a stream removed while its lease lives) | FIX: SC-041-003 |
| AR-2 (`add_delta` overload) | FIX: drop and recreate, grants restated, down restores (data-model.md §3) |
| AR-5 (older replica fails quota listings) | FIX: section 10 and the runbook; asserted by T041-26 (b) |
| SE-1 (viewers read step-log usage) | FIX, D041-066: `used` absent without `run:logs:read`; G041-015 closed; T041-4 |
| SE-3 (script log not audited; `may_read` transaction) | FIX: `may_read` runs in the route's read transaction, and T041-29 writes the throttled `run.logs_read` before a log is returned (section 5) |
| Stage 6: Draft beside OD-2026-11-15-17; D1958 not introduced; G041-005 "below 1.6"; gap range; research G041-008 "row id"; "go on to step 7"; rollout wording; CUJ 2 poll attribution; "negotiation is unchanged" | FIX: Overview, FR-041-012, G041-005 and G041-008 restated, VIII and the gap list to G041-016, section 2's control flow, section 10, CUJ 2, section 1 |
| Stage 6: "five quota resources"; undefined project terms; terms used before definition; the section 10 reference | CONTEST: five namespace resources plus one tenant resource is what FR-041-020 and data-model.md §6 say; the Terms paragraph defines the project terms the design uses before section 1; no undefined reference to section 10 was found |
| Stage 7 design nits: the runtime role holds `SELECT` on `tenant_retained_usage`; "goes on to step 7"; K against tickets × R | FIX: step 6 now says the accessor exists for `quota_accounting_control` and for one limit rule (`durable_quotas.up.sql:561-572`); control flow; SC-041-001 uses K on one replica, section 2 tickets × R |
| Stage 7 design nits: the single-caller test covers only some files; records above `ceiling_sequence`; per-page scan bound | CONTEST: the test guards `share_secrets_lock`, which the design keeps at one caller; records after a met ceiling fail the attempt allowance of step 6 and are counted as dropped by the server; a read scans at most the run ceiling's rows under the route's statement timeout (section 5) |
| Stage 7 design nit: `safelog.go` path | FIX: `sdks/go/internal/safelog/safelog.go` |
| Stage 7 structural nits: status; Goals miss the Studio; gap range; resolution.md not a link; CUJ 5 without a story | FIX |
| Stage 7 structural nits: answered decisions under "needed"; Alternatives format; no CUJ for the marker; NFR id style; long lines | CONTEST: the heading is cited by research.md and the ADRs; the format is the template's; the marker is a row of CUJ 1's tab, specified in section 7 and tested in T041-13; `NFR-041-xxx` is the id form of `AGENTS.md`; long lines are kept where a path or a table row needs them |
| Stage 7 style nits ("stream" for three things, poll attribution, review tags, K against R, "remaining calls", rollout sentence) | FIX |
| Stage 7 style nits ("MUST be sent", markers "hold no text" against labels, five resources, forward references, saturation, "high mark", reason against verdict, `SESSION` resend clause, two "fixed" quantities, jargon, long lines) | CONTEST: each reads as intended in context (a marker stores no text; the Studio renders a label for it), and none changes a rule |
| Stage 8 nit: `SAGAT_STEP_LOGS_READ_ENABLED` asked by no owner decision | CONTEST: it is an operational switch to shed read load during rollout (NFR-041-OPS), not product; it never opens a script log FR-041-022 closes |
| Stage 9 IR nits: Rust SDK refuses accepted > 6; callers of `current_worker_authorized`; `take_log_prefix` argument and callers; lock-order test needs later tasks; "within the bound"; T041-22 and T041-25 Files; T041-32 placement | FIX: T041-20 (`worker.rs:468`); T041-6; a `record_execution`-only variant (T041-13); ST-5; D041-059; full paths; ordering |
| Stage 9 IR nits: `supports_step_log_protocol` form; Owner-gate on T041-23 and T041-26; T041-13 test location | CONTEST: the proof is the form of `supports_scoped_worker_protocol`; the owner rows gate the server tasks that build the decisions those two tasks only exercise; `integration_inline_code_sandbox` is the existing target of the script runner's tests |
| Stage 9 CD nits: `negotiate_protocol_details` wording; inline-code-sandbox target; `step_log_isolation.rs` in T041-6; T041-32 placement | FIX, except the target, which exists (`tests/Cargo.toml:233`) |
| Stage 9 ST nits: T041-2's condition; "starts" against "merges"; T041-31's held-transaction workload; "four messages"; "deferred answers"; T041-28's "use"; first-failure wording; partial paths; T041-25 quickstart; T041-27 done condition; gap range | FIX |
| Stage 9 ST nits: T041-8 Refs; "the row of section 6"; formatting | CONTEST: T041-8's Refs name what it proves; "the row of section 6" is defined by Slice D's introduction; formatting is the checker's, which passes |

The approval run of this revision follows in `*-approval-2.md`.
