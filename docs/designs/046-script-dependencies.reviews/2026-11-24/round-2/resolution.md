# Round 2 of 2026-11-24: what answers each finding

Round 2 (the five `egm-review` stages and the three repository reviewers) read
`c00d7999f24de5eef7cbe2e2a95d1059bf27f03e`: 4 Blockers (implementation readiness 1, codebase 3) and 61 Majors. The
design was revised; a third round reads the new text. TEXT: the design, a contract, the data model or an architecture
decision now says it. CONTEST: the evidence says otherwise, with the reason. Decisions 046-035 to 046-050 and Gap 046-009
record the choices; feature 023's Decision 1628 records the matching corrections there. Settled under Owner decision
2026-11-24-4.

| Change | Findings it answers |
|---|---|
| The gateway applies no policy and never calls the control plane: the job grant carries the quarantine cut; reserved names go to the control plane's private routes (Technical Plan 3, `package-gateway.md`; Decision 046-037) | implementation readiness Blocker 1, zoom-out 1, architecture 1, security 1, correctness 6, comprehension 2, codebase 12, design Nit 9 |
| Private packages in the tenant's payload store, served by the control plane (Technical Plan 10; Decision 046-036) | security 2, implementation readiness 4, design 6 |
| OSV archives through the gateway; staged imports; refusals and the hold; a short switch (Technical Plan 7; Decision 046-039) | zoom-out 2, architecture 2 and 5, security 4, design 4 |
| Blocks checked at lease, materialization and start (Technical Plans 11 and 12; Decision 046-040) | security 3 |
| Materialization before the lease (Technical Plan 11; Decision 046-041) | design 1 |
| A fresh verify container, checked-hash bytecode, a seal step (Technical Plan 8; Decision 046-042) | design 2, correctness 8, codebase 8 |
| Layer records separate from the bundle; the backfill pass (Technical Plan 9, `bundle-v5.md`; Decision 046-043) | architecture 7 |
| Image retention (Technical Plan 11; Decision 046-044) | design 3 |
| The finish on both paths with the three rows `FOR SHARE` (Technical Plans 1 and 9; Decision 046-045) | correctness 1, 2 and 10, codebase 4 |
| Re-evaluation per namespace with rows held; malware release route and trigger (Technical Plans 6 and 7; Decision 046-046) | correctness 3, comprehension 4, security 6, architecture Nit 11 |
| The lease check in `script_execution_admitted_instance` with `reject_input` (Technical Plan 12; Decision 046-047) | codebase Blocker 1 |
| The capability constraints by their current names with the full set (Task 046-2, `data-model.md`) | codebase Blocker 2 |
| `SCRIPT_SANDBOX_UNAVAILABLE` for an old runner (Technical Plan 17) | codebase Blocker 3 |
| `fetch` reuse, 160 KiB chunks, budgets, transport failures retryable (Technical Plan 4, `runner-protocol.md`; Decision 046-048) | codebase 7, security 7, correctness 5 |
| Publication deduplication, `:resolve-again`, the key at claim (Technical Plan 1; Decision 046-049) | codebase 4 and 5 |
| `Expected`, `Bundles::check` and `BuildAssignment` extended (Technical Plan 9) | codebase 6 |
| Install completion under the version row; siblings; `NULLS NOT DISTINCT` (Technical Plan 1) | correctness 4 |
| Collection with marks, pins and a grace period (Technical Plan 3, `data-model.md`) | design 5, correctness 7 |
| The test-run path with its fields (Requirement 046-016, `runner-protocol.md`, Task 046-14) | comprehension 1, implementation readiness 6, correctness 9 |
| The author-lock path (Technical Plan 1) | comprehension 3 |
| The fetch budget charged at resolve completion (Technical Plan 3) | comprehension 7, implementation readiness 5, design Nit 9 |
| Phase A with the lease check and the battery's part; activation refused before it (Technical Plan 18; Decision 046-050) | architecture 6, comprehension 5, design Nit 10, implementation readiness 7 |
| The REST contract (`rest-api.md`), the telemetry split (`telemetry.md`, Tasks 046-21 and 046-25), task owners | implementation readiness 2 and 8, comprehension 6 and 12 |
| Claim fields `cut` and `maxLayerBytes` (`runner-protocol.md`) | implementation readiness 3 |
| `script_build_provenance` created here, envelope by digests (Technical Plan 9; feature 023 Decision 1628) | architecture 4, codebase 11 |
| ADR-0057's amendment, feature 023's plan 8a and Decisions 1237 and 1625 corrected | architecture 3 and Nit 12 |
| The `script_jobs` quota, the job states (`data-model.md`) | codebase 10 |
| Reuse of the lake profile's mount builder (Technical Plan 8) | codebase 9 |
| The placement table completed | codebase 13 and Nits 14 and 15 |
| Principle II "Pass with conditions"; one constitution baseline; refs and named tests | structural 1 and 2 and Nits 3 and 4 |
| Step labels in words; long sentences split; terms written out | style 1 and 2 and Nits |

| Report | Finding | Answer |
|---|---|---|
| security | 5 (runner disk exhaustion) | TEXT in part: the unpacker's bound, the volume filesystem in the runbook. CONTEST in part: engine volumes have no quota, and a compromised unpacker's worst case is a full layer disk on that runner, a denial of service with no disclosure; accepted and stated as Gap 046-009 |
| security | 4 (parse in a sandbox) | CONTEST in part (Decision 046-039): bounded streaming parsing in the control plane, with the older, shrinking and hold rules limiting what a poisoned archive can do |
| zoom-out | 4 (bytecode and materialize-ahead are unmeasured) | CONTEST: bytecode compilation is now also what keeps imports working on a read-only layer without writing bytecode at run time; materializing ahead is the mechanism that keeps materialization out of the attempt timeout (Decision 046-041), not an optimisation |
| structural | 5 (the "(CUJs)" heading) | CONTEST: it is the template's section name; the subheadings are written in words; a sentence under the heading says so |
| style | Nit 14 (spelling variety) | CONTEST: the repository's documents use both; changing words across files is the sweep's business |
