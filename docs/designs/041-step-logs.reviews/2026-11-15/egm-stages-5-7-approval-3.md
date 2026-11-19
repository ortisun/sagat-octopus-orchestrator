# EGM stages 5 to 7 of the third approval run of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 6dceb5482d1a5a27b22be4018ed2df83e106137b
Reviewer: egm-review (stages 5, 6 and 7)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Note: fresh agents of the registered types, dispatched in order: stage 5 `egm:egm-zoom-out` (lines 1-1818), then stage 6 `egm:egm-comprehension` (FeatureRange 1-1269, DetailedRange 1270-1818), then stage 7 `egm:egm-design`, `egm:egm-structural` and `egm:egm-style` in parallel (lines 1-1269). Each was given the target, its persona and standard, and the reading list, not the author's conclusions. Recorded condensed; no severity was lowered.

## Stage 5, zoom-out (Blocker 0, Major 4, Nit 5)

- Z3-1 Major: four namespace-tunable ceilings (and `step_log_attempt_records = 0` as the capture switch) that no user story asks for; they cost resources, fences on every batch and a validation exception. Make the attempt and run ceilings constants, keep the retention, give the namespace a plain capture switch; the owner if O041-005 decided tunability.
- Z3-2 Major: the exact run ceiling brings `run_log_usage`, a serialising lock held across the authority reads, the purge give-back and two owner questions; ask the owner up front to drop the run ceiling or take the unlocked sum (which also gives O041-027 (a) for free).
- Z3-3 Major: the script marker inside `record_execution`'s transaction ties feature 005's report to step-log quota locks; a short second transaction after the commit, keyed by the execution id, would leave `record_execution` and `take_log_prefix` unchanged (a crash between the two loses one marker).
- Z3-4 Major: the Technical Plan is still at statement level and repeats data-model.md and contracts.md, and the copies disagree (Constitution Check V still says "trigger").
- Nits: Constitution V "trigger" and VIII "G041-016"; the "Answered" history; T041-2 unticked; `isolated_server` duplicates `start_server`'s start.

## Stage 6, comprehension (Blocker 0, Major 2, Nit 12)

Paraphrase: present in the agent's result ("What this proposes", "How the existing system works, per this document"); every section could be paraphrased.

- C3-1 Major: ADR-0085, linked as a decision of record, describes a trigger, a backfill, the read switch and `mustNotLeak`; the only path to correct it is O041-025's Note, which is about other things; Constitution V repeats "trigger".
- C3-2 Major: the state of a `source = script` item is undefined when the marker stream does not exist (skipped) and after a delete, while `getScriptExecution` still returns the log.
- Nits: terms used before the Terms block with no pointer; "usage row" for two locks; "held after T7" and the digest remark in SC-041-001; lake runner, database runner and managed executor undefined; FR-041-002's identity against script markers; ADR-0027 never linked; VIII's gap range; the capacity rate against the spike's two batches a step; T041-28 listed before T041-11; T041-2 unticked and the header date; "five stages"; O041-026 missing from T041-31's "Then".

## Stage 7, design critic (Blocker 0, Major 8, Nit 9)

- K3-1 Major: ingest holding the namespace secrets lock shared delays leases, heartbeats and managed-database admission too (`crates/infra-postgres/src/execution.rs:1023-1027`; `execution/database.rs:96, 106-114`), not only completions; restate the bound and measure it, or drop the lock from ingest.
- K3-2 Major: one tenant can fill the whole `Class::Logs` cap, since the class slot is held while a final batch waits for a tenant ticket (`crates/api-grpc/src/limits.rs:675-696`).
- K3-3 Major: synchronous redaction on the activity's thread against FR-041-005's timing rule; check the buffer and ceiling state before formatting and redacting, or redact on the sender.
- K3-4 Major: ingest overload slows execution (final flush waits up to 2 s per step), with no SDK-side breaker.
- K3-5 Major: with the defaults the tenant step-log quota fills in minutes to hours and drops the newest lines for up to 14 days, with no tenant signal, no sizing guidance and no risk row; the benchmarks must set the quota.
- K3-6 Major: O041-025 (a) accepts more than reports do in a stopping run (derived nodes, not cancelled, `crates/application/src/worker.rs:2824-2831`), and its framing misstates the code.
- K3-7 Major: ADR-0085's text contradicts the design and its correction depends on an unrelated owner answer; Constitution V still says "trigger".
- K3-8 Major: reads filtered to one step or stream scan the whole run (the read index has no `stream_id`), and the Studio's paging through empty pages is unspecified.
- Nits: VIII gap range and G041-017 not a condition of V; re-enabling capture waits for re-registration; the lock-order wording for `tenant_step_log_bytes`; whether SDKs implement the credential shapes of `redact`; a process serving two tenants makes G041-017 cross-tenant; shared tail caps; rollback with a ceiling row; stream-row churn and non-HOT updates; the audit per poll.

## Stage 7, structural (Blocker 0, Major 2, Nit 8)

- S3-1 Major: Constitution Check row V names the rejected trigger.
- S3-2 Major: G041-017 (open, principle V) is missing from row V's conditions, row VIII's range and Complexity Tracking.
- Nits: long lines; O041-026 grouping; the Terms block's place; label collisions (T1 to T10, tenant T, task ids, lock-order numbers); run-in anchors and punctuation; NFR suffixes; links (ADR-0027 glob); operation ids not introduced.

## Stage 7, style (Blocker 0, Major 6, Nit 25)

- T3-1 Major: section 10's sentence on `SAGAT_STEP_LOGS_ENABLED` never says "when false".
- T3-2 Major: Constitution V "trigger".
- T3-3 Major: Constitution VIII's gap range.
- T3-4 Major: O041-025 does not say what the ADR text says now and what each option changes.
- T3-5 Major: FR-041-006 packs ceilings, truncation, markers, a pending decision and the accounting rule into one paragraph.
- T3-6 Major: the Overview's last sentence holds mechanisms and a schedule; a garden-path phrase.
- Nits (25): MUST used unevenly; "whatever"; "exactly"; stacked clauses; "stored" for "sent"; "one contingency, one insertion point"; "unless that result"; O041-026 grouping; unsupported reasons in answered options; unmeasurable words in CUJs; CUJ 3 and the byte ceiling; "graph stream" twice; ambiguous pronouns; "the connections lease calls need"; vague gain; verbless openers; meta-commentary; a passive requirement; the "916" quote; units; constitution versions; vague rationale cells; "say it first"; "a tail's included"; long lines.

Counts: Blocker 0, Major 22, Nit 59
