# EGM stages 5 to 7 of the second approval run of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: fefcfafbb40d153ebd3e462c7ba2d94a7c0d9286
Reviewer: egm-review (stages 5, 6 and 7)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Note: fresh agents of the registered types, dispatched in order: stage 5 `egm:egm-zoom-out` (lines 1-1652), then stage 6 `egm:egm-comprehension` (FeatureRange 1-1184, DetailedRange 1185-1652), then stage 7 `egm:egm-design`, `egm:egm-structural` and `egm:egm-style` in parallel (lines 1-1184). Each was given the target, its persona and standard, and the reading list, not the author's conclusions. Recorded condensed; no severity was lowered.

## Stage 5, zoom-out (Blocker 0, Major 3, Nit 3)

- Z2-1 Major: the Technical Plan is at implementation level (a ten-step SQL pipeline, stub lists, over a hundred file:line pins), against D041-047 ("the design keeps invariants"); the completion-wait bound is restated six times. Move the pipeline and pins to data-model.md or the tasks; state the bound once.
- Z2-2 Major: the exact run ceiling (`run_log_usage`, its lock slot, G041-009, O041-020) costs more than the lossy design needs; the unlocked sum of stream counters overshoots by the batches in flight (about 2.3% at one replica). Make O041-020 (b) the recommended default.
- Z2-3 Major: T041-9 builds a second harness on `platform_with` (one `TestApi`, `tests/support/worker.rs:48-60`) that cannot run SC-041-007's "8 replicas" row; extend `scalability-ha` instead, or restate SC-041-007.
- Nits: "Owner decisions needed" repeats the record; T041-32 listed out of order and T041-2 still unticked; the spike's chunk variant measured when it cannot change the outcome.

## Stage 6, comprehension (Blocker 0, Major 2, Nit 10)

Paraphrase: present in the agent's result ("What this proposes", "How the existing system works, per this document"); the reader could paraphrase every section.

- C2-1 Major: T041-1 says "five stages … and the three repository reviewers" and "four reports"; a newcomer cannot tell what a complete run is, which reports count, or who the three reviewers are.
- C2-2 Major: research.md's prose still contradicts the design: "Review of 2026-11-15" says D041-030 removes the marker; "Owner answers of 2026-11-15" says the marker carries the runner's verified iteration (against D041-060); D041-006 orders by "row id" and is not marked superseded.
- Nits: two numbered lists in section 2 both cited as "step N" and the `AUTHORITY` claim does not cover the pre-checks; runner tokens refused `PERMISSION_DENIED` before the transaction yet tested as `AUTHORITY`; project terms undefined (activity binding, discovery, graph stream, runtime role, design 038, tickets) or used before the Terms block; markers' level unspecified; a deleted script stream's state; rounding of "half the cap" and D041-034 not superseded; CUJ 3 "first 10,000 lines"; `/logs/stream` against `/logs/streams`; T041-32 order and T041-2 tick; ADR-0084 and ADR-0085 texts still describe older mechanisms (spike at T041-6, `mustNotLeak`, classified fact "backfilled").

## Stage 7, design critic (Blocker 0, Major 4, Nit 8)

- K2-1 Major: ingest step 2 refuses every batch while `stop_reason` is set, but compensation and reconciliation steps (and a cancel acknowledgement) are leased and reported in a stopping run (`attempt_authority_fence`, `crates/application/src/worker.rs:2828-2831`; `crates/infra-postgres/src/workers.rs:98-153`); the logs an operator most needs are lost; Constitution VIII's row omits failed compensation and the timeout/signal race.
- K2-2 Major: the alternative that removes the main contention, a per-tenant step-log usage row separate from `tenant_retained_usage`, is not evaluated (ADR-0027 line 49; ADR-0084's original Consequences).
- K2-3 Major: no capacity estimate: 2 tickets per tenant per replica at ~923 batches/s leave ~2.2 ms per ingest at the "1 replica, 16 workers" row (`tests/performance/reports/scalability-ha.md:281`); final flushes would meet `RETRY_LATER`; derive defaults or reserve capacity for final batches, and state the service-time budget T041-31 must show.
- K2-4 Major: the read side has no per-request scan bound and no risk row: a non-matching `q` over a run of up to 1,000,000 rows can exceed the statement timeout every time, and `nextCursor` does not advance past scanned rows.
- Nits: the run ceiling never resets for a long-lived run; a full secret set stops capture for the process (no risk row, no drop reason); section 10 understates the down migration's refusal on `classified_namespaces`; the tail's per-window audit; CUJ 2 latency is not a bound; rollback on a direct `AUTHORITY` failure unstated; the ticket default and D041-034; research.md's stale marker sentence.

## Stage 7, structural (Blocker 0, Major 1, Nit 7)

- S2-1 Major: Complexity Tracking omits added moving parts: the separate quota and the ADR-0027 Amendment's machinery, the script-marker write path, `Class::Logs` with tickets, the read switch, the CI-gate amendment.
- Nits: long lines; the two numbered lists of section 2; NFR word suffixes; CUJ 5 tagged US01 but US01 does not list it; FR-041-007's unlabelled fifth item; "Owner decisions needed" without Open/Answered subheadings; "§10" after renumbering, and the second SDK table has no caption.

## Stage 7, style (Blocker 0, Major 8, Nit 27)

- T2-1 Major: Overview line 18 promises without condition that no secret can be read, against G041-001.
- T2-2 Major: FR-041-023's "a batch or a marker that commits after it is stored" contradicts FR-041-009 and section 4 (`REMOVED`) unless it means a new stream.
- T2-3 Major: "five quota resources" lists six items; line 1044 says "six resources".
- T2-4 Major: two different first failures stated for `step-logs-redaction` (lines 926-927 against 938-939).
- T2-5 Major: bare file names where several crates have one (`scripts.rs`, `execution.rs:1340`, `worker.rs:28`) and a glob (`ADR-0027-*.md:93`).
- T2-6 Major: RFC 2119 keywords used unevenly (MUST in FR-001 to -008, plain present after; "must never", "may" in a prohibition).
- T2-7 Major: ingest step 6 defines the tenant allowance in a five-line parenthetical; "that limit" points into it; broken wrapping.
- T2-8 Major: SC-041-001 names "measuring windows" defined nowhere, and "shown by the queue, not by a time" is a remark, not a criterion.
- Nits (27): quantity without a noun; constitution versions; fragmentary comparisons; "full keyboard operation"; "superset"; saturation wording; first Goal is a cost and "stored" for "sent"; the owner-decisions heading; unverifiable reasons; "the three numbers"; jargon ("Minors are a line", "store proofs", "held to commit", "errs closed", "stays green"); clipped clauses; `StepLogTx` "only reads and writes"; repetition; "prove"; marker exclusion in the counting rule; a double negative; a passive deployment requirement; the "916" quote; "one absence of `used`"; mixed number style; "growing for an hour"; the rollout parenthetical; "Keeping the convention"; CUJ 2 bound; "decision 11 without an exception"; long lines.

Counts: Blocker 0, Major 18, Nit 55
