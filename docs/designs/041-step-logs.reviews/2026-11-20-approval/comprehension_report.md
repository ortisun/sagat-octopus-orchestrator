Doc: docs/designs/041-step-logs.md
Reviewed-commit: ff8033cd933d0c6adf972706eb1bb38407bdb7c6
Reviewer: egm-comprehension (stage 6 of egm-review)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the complete result of a fresh `egm:egm-comprehension` reviewer, dispatched directly, on the owner's approval path "Full review, capped" (OD-2026-11-20-17). Saved verbatim.

## Comprehension Test (stage 6): docs/designs/041-step-logs.md at ff8033cd

I read the persona, the whole target (lines 1-3214) and the hunk map of the supplied diff. No document standard applies. The review is complete.

### What this proposes

The SDKs of six languages capture the lines an activity writes to its per-attempt logger. Each SDK removes the secret values its process resolved and sends the lines to the platform over protocol minor 1.7. A long step streams its earlier records through a new unary RPC, `AppendStepLogs`, which runs in its own transaction and never takes a run or step row lock. Every attempt's last records ride on its `CompleteStep` or `FailStep` as a "carry": at most 64 records and 16 KiB, plus the highest sequence. One plpgsql function call writes the carry inside the report's own transaction and only tries its step-log locks. When a lock is held, the carry writes nothing (`contended`). When the quota refuses it, or the quota row is busy past 200 ms, the carry records a "mark-only" final mark. When the call itself fails, the report is re-run with the carry downgraded, so logging never changes a step's outcome.

Lines live in two PostgreSQL tables beside the journal. They have per-attempt ceilings, a separate tenant quota, retention with a purge pass, and an admin-only delete route. The quota is charged to per-(tenant, database backend) "leases" of room instead of the tenant's usage row at commit. Before a carrying report, a separate "ensure" transaction tops that lease up, and a bounded "fallback refill" inside the report covers the rest. People read the lines through a list route, a streams route and a server-sent event tail, under a new `run:logs:read` permission plus the PII reveal grant, and in a Studio Logs tab.

### How the existing system works, per this document

Today a worker's log stays on its own machine. The run's events, attempts and telemetry routes are rebuilt from `run_events`, and only script steps have a stored log (`script_executions.log`, under `tenant_log_bytes`). The engine locks the run row `FOR UPDATE`, and `work_budget.rs` holds leases and completions to exact statement, transaction and row counts. Retained bytes are charged by a per-row trigger and a deferred flush that locks the tenant's `tenant_retained_usage` row at commit. A report is fenced by its lease token, the activity binding and the membership. Nothing deletes a run.

### Findings

**K-1. Major: "lease" means two different things, and Terms defines only the newer one.** Lines 652-653, 129, 172-177, 338, 401, 823-828, 1184, 1213-1226.
- Terms defines *Lease* as a row of `step_log_quota_leases`.
- The document still uses "lease" for a step's lease on almost every page: "A `steps` row holds a lease" (129), "T2. The lease, read without a lock: the `steps` row" (823), "A lease and a heartbeat pay nothing" (1184).
- FR-041-005 uses both senses in one requirement: "its lease's room" (172) and "close to its lease's expiry" (177). SC-041-001 line 338 does the same: "held after T8 (its charge holds a lease)".
- "Reclaim" is overloaded the same way. It means a step-lease reclaim ("expired or was reclaimed", 637-638, 1135) and a quota-lease reclaim (2b).
- A new reader cannot tell which lease a sentence means without reconstructing it from context.

Recommended disposition: fix. Renaming the quota sense (for example "quota lease" everywhere, and the same for "reclaim") removes an ambiguity that runs through the FRs, SCs and tests.

**K-2. Major: the vocabulary of the quota-lease mechanism is used from the Overview on but defined only in section 2b.** Lines 24-30, 171-174, 210-214, 635.
- Terms says it covers terms "used from the Overview on" (635). It has no entry for *ensure*, *writer*, *refill* or *fallback refill*, *fold*, *grant*, *room* or *b*.
- The Overview's key sentence (24-30) depends on all of them: "the ensure that makes its writer's step-log lease's room before the report ... a fallback refill of that lease under the run lock".
- A reader reaching FR-041-008's "its writer's own step-log quota lease" will take "writer" to mean the worker. It actually means a server database backend (`pg_backend_pid()`, 1213-1214), and that only becomes clear about 1,000 lines later.

Recommended disposition: fix. A few Terms entries make the Overview and the FRs readable on first pass.

**K-3. Major: what SC-041-007 now requires, and whether T041-35 passed, is ambiguous.** Lines 374-386, 405-408, 438-452, 777-778, 2189-2191, 2248-2251, 2438-2441, 2700, 3071-3072.
- SC-041-007 states the 90% rate and 10% (or 2 ms) p99 criteria. It then says they are "Settled by the measured cost", with 1 × 16 failing (82.0%, +8.6 ms).
- The Goals still say the effect "must" keep those figures (405-408).
- T041-9 says "a failure of SC-041-007 stops the release" (3071-3072), and the Open Questions say the same (450-451). Measured against the original thresholds, 1 × 16 already fails, so the reader cannot tell what T041-9 compares against: the original thresholds, the accepted figures, or only the 8 × 128 row.
- T041-35 gates other tasks "until it passes" (2190-2191). Its evidence says "this task is ticked" (2440), but its box is unticked (2189).
- The ticket and cap defaults are "placeholders until T041-35 ... records the defaults it sets" (777-778). T041-35 sets them only "If it passes" (2248). T041-7 builds "the defaults T041-35 set" (2700). The reader cannot find which defaults T041-7 builds.

Recommended disposition: fix. One sentence each in SC-041-007 and T041-9, stating the criterion that now applies, plus the chosen defaults, closes this.

**K-4. Major: SC-041-001 and the Verification Plan contradict section 2a/2b on a quota-refused carry.** Lines 348-350, 3167 against 1105-1112, 1124, 1357-1360, 2820-2825.
- SC-041-001 says a carry "refused at the tenant's step-log limit" is "recorded by a downgrade re-run". The Verification Plan row "A carry never changes a report" says the same.
- Section 2a says the function's own block catches the refusal, writes a mark-only final mark, and "The report commits in its first transaction". It adds that "The quota's own refusals never reach this path".
- Section 2b says "no report is re-run for a quota refusal", and T041-36's test expects "no re-run".

Recommended disposition: fix. The success criterion and the plan would test the opposite of the design.

**K-5. Major: several cost claims are left over from the earlier transport.** Lines 401-403, 448, 1184-1185, 1885 against 1145-1147.
- The Goals say "six [statements] for a long step". Section 2a's table says any carry costs "+ 1" statement.
- The Open Questions say "the round trips (five for a short step)". The design now makes one call, in one round trip.
- The Goals, section 2a (1184-1185) and the Risks (1885) say a short step adds no transaction. The table charges the ensure "+ 1" transaction whenever the kept room is short, unknown or stale, which includes every cold start.

Recommended disposition: fix. These are the headline cost claims a reader takes away first.

**K-6. Major: the Verification Plan describes the withdrawn `NOWAIT` refill.** Line 3168 against 1284-1303, 1070-1071.
- The row "What a report waits for" says "a carry's refill answers `contended` at once".
- Under D041-119, a carry's fallback refill waits up to *b* = 200 ms for each lock acquisition, then answers `tenant_quota_busy` (mark-only). It never answers `contended`.

Recommended disposition: fix. An implementer writing that test would build the wrong expectation.

**K-7. Major: the conditions for `contended` contradict each other.** Line 1112 against 1079-1082, 351-353, 2812-2814.
- Line 1112 says "The function returns `contended` only when the existing stream's row is held."
- CA returns `contended` when any of the six policy fences, the stream lock or the three class fences is refused.
- SC-041-001 and T041-36 test `contended` for an ingest held after T3 and for a held resource fence.

Recommended disposition: fix. Scoping line 1112 to the charge paragraph is a one-line edit.

**K-8. Major: the third budget measure is garbled, and its pinned value is unclear.** Lines 1159-1169, 1383-1390.
- The sentence says the test "sets `pg_stat_statements.track = 'all'` for nothing", and then says it changes no server setting. Its parentheses are unbalanced.
- The equality "26" is derived on the prototype. That count includes an `application_name` statement "which the built function does not set", and "the built function's own statements are recorded by T041-36 the same way".
- Section 2b then states "26 nested statements" as the budget line for the built carry.
- The reader cannot tell whether the built equality is 26, 25, or a number still to be recorded.

Recommended disposition: fix. The text calls this an equality gate, so its value must be unambiguous.

**K-9. Major: the architecture diagram contradicts the waits the Overview highlights.** Lines 690-699.
- The carry box says "each lock tried, never waited for". It shows no ensure transaction before the report and no bounded fallback wait for the usage row under the run lock. Those are the two waits the Overview names.
- The ingest box omits the attempt's stream lock that T3 waits for.
- A reader who builds a mental model from the diagram gets the central latency property wrong.

Recommended disposition: fix. Adding the ensure box and qualifying "never waited for" is small.

**K-10. Major: "K" is never defined.** Lines 338, 2630, 3168.
- SC-041-001 says "K − 1 more ingests of T and a carrying report of T each commit while it is held", and T041-6 repeats it. Nothing says what K is (tickets, leases, backends or writers).
- Without K, the success criterion cannot be reproduced as a test.

Recommended disposition: fix. One clause defines it.

**K-11. Nit: symbols are reused with different meanings.** Lines 666, 1216, 2156; 960, 1215; 889-904, 1224.
- *R* is the number of API replicas (Terms, T041-31) and also the leased room Σ(granted − used) (2b).
- *S* is a batch's service time (Capacity) and also the folded usage (2b).
- *L* is the limit, and also the prefix of the lock levels L0-L4.
- *T* is a tenant, beside the steps T1-T10 ("tenant T is held after T8").

**K-12. Nit: "step-log lock" is used as a closed set but never defined.** Lines 28-29, 351, 937, 1056.
- It is unclear whether the usage row (L4) counts. L4 is listed in "One lock order for everything that writes step logs", yet "a carrying report never waits for a step-log lock" while it does wait for L4 in the fallback.

**K-13. Nit: some status phrases are stale.**
- ADR-0097 and ADR-0095 are called "Proposed" (50-51, 63), but are accepted (84).
- "superseded by ADR-0095 decision 2 when accepted" (1140).
- "T041-35's third run, which states what follows a third failure" (1412-1413), although a fourth run happened.
- The header Date is 2026-11-19, but the revisions are dated 2026-11-20 (5).
- "Before this revision" (73) does not say which of the four revisions it means.

**K-14. Nit: the "Open" heading holds a settled item.** Lines 458-464.
- It says "None is open", then lists O041-026 under it, marked settled.

**K-15. Nit: the gap summary is incomplete.** Lines 3190-3200 against 1936.
- It lists G041-001 to G041-017 only. It omits G041-018 to G041-027, which the revision depends on (for example G041-021, -023, -026 and -027), and the closure of G041-025.
- The Constitution Check cites "G041-001 to G041-027".

**K-16. Nit: the Risks row gives the wrong number of re-runs.** Line 1889 against 1116-1119.
- It says a failed carry "re-runs the report once". Section 2a allows up to four transactions.

**K-17. Nit: the renumbering of `sdks/SPEC.md` is unclear.** Lines 1594-1596.
- The sentence does not say which of the two sections numbered 10 is "the one on failure isolation".

**K-18. Nit: one sentence does not parse.** Lines 1185-1186.
- "and the batches that what does not fit its carry needs".

**K-19. Nit: an allowlisted function is unnamed.** Lines 1435, 2044.
- The `SECURITY DEFINER` allowlist names every function except "the statement-level quota trigger function of its Amendment of the Amendment of 2026-11-20". An allowlist should name each entry.

Counts: Blocker 0, Major 10, Nit 9
