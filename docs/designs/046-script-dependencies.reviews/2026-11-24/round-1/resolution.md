# Round 1 of 2026-11-24: what answers each finding

The complete run (the five stages of `egm-review` with direct dispatch, and the three repository reviewers) read
`cfb22df54df5b084f748531dbdb9b3546c01f60d`. It ended with 7 Blockers and 82 Majors, so the design was rewritten and a
second round reads the new text. TEXT: the design, a contract, the data model or an architecture decision now says it.
CONTEST: the evidence says otherwise, with the reason. Decisions 046-021 to 046-034 and Gaps 046-007 and 046-008 of
`specs/046-script-dependencies/research.md` record the choices. Settled under Owner decision 2026-11-24-4.

## The changes that answer most findings

| Change | Findings it answers |
|---|---|
| The resolve container has no network; HTTP over the supervisor channel to the runner, which forwards only the gateway's package routes with the token (Technical Plan 4, `contracts/runner-protocol.md`, Decision 046-021) | design 2 and 3, security 5 and 15, architecture 8 and 13, correctness 16, comprehension Nit 8 |
| No platform storage of layers; each runner materializes from the package store and checks the signed digest (Technical Plans 8 and 11, Decision 046-022) | zoom-out 4, design 8, codebase 6 and 8, architecture 4 (layers), correctness 8, security 12 |
| `script_dependency_jobs`, fenced phase transactions, resolution never cached, build key version 2 after the lock, evaluation on every publication (Technical Plan 1, Decision 046-023) | comprehension 1, design 1, security 3, architecture 2, codebase 2, correctness 1, 3 and 4 |
| Report, policy and snapshot outside the bundle (`contracts/bundle-v5.md`, Decision 046-024) | architecture 2, security 3, correctness 1 |
| `script_artifact_blocks` with scope; durable re-evaluation jobs with a cursor; malware blocks in the import transaction; older or shrinking snapshots refused (Technical Plans 6, 7 and 12, Decision 046-025, Architecture decision 0121) | comprehension 3, zoom-out 2, codebase 1, architecture 1, security 1, 2 and 4, correctness 5, 6 and 7 |
| Finishing transaction reads the snapshot row `FOR SHARE` (Technical Plan 9, Decision 046-026) | correctness 2 |
| Tenant-wide reservations, normalised names, uploads tenant-wide, reservation re-evaluation (Technical Plan 10, Decision 046-027) | comprehension 5, design 7, security 7, correctness 13, style 7 |
| A package store of its own; collection; fetch budget; uploads in content bytes (Technical Plan 3, Decision 046-028) | architecture 4 and 5, codebase 6 |
| Required targets, per-target install jobs, capability and target declared per claim, target-aware activities (Technical Plans 8 and 11, Decision 046-029) | comprehension 2, architecture 9 and 10, codebase 10, correctness 11 |
| Version math behind an application port (Technical Plan 5, Decision 046-030) | architecture 6, codebase 15 |
| Contracts and data model written at the reviewed text (`contracts/lock-formats.md`, `bundle-v5.md`, `package-gateway.md`, `runner-protocol.md`, `errors.md`, `supply-chain-policy.schema.json`, `acceptance-packages.md`, `data-model.md`) | implementation readiness 1 (the Blocker), 3, 8, codebase 13 |
| Tasks with files, tests and an ordered table by builder and phase (Detailed Implementation Plan, Technical Plan 18) | zoom-out 1, comprehension 7, implementation readiness 2, 4, 9 and 10, codebase 14 |

## The other findings

| Report | Finding | Answer |
|---|---|---|
| comprehension | 4 (OSV egress) | TEXT: the control plane downloads from the configured source through the streaming send (Technical Plan 7) |
| comprehension | 6 (undefined terms, unlinked references) | TEXT: two term lists and links |
| comprehension | 9 to 12 | TEXT |
| zoom-out | 1 (scope) | TEXT in part: five delivery phases with their own acceptance (Technical Plan 18). CONTEST in part: the design is not split, because the phases share one lock, one bundle and one policy table, and splitting them would repeat the contracts in two documents |
| zoom-out | 2 (speculative options) | TEXT for `blockNewAtLease` (defined in Technical Plan 6). CONTEST for `blockUnrated` and licences: each is one rule over data the evaluation already has, and both are common asks of security owners |
| zoom-out | 3 (fallbacks with 023) | TEXT: Decision 046-033; feature 023's Decision 1627 agrees |
| zoom-out | 5 (existing lockfile gate) | TEXT: `lockfile.rs::accept` stays the single runner gate (Technical Plan 2) |
| zoom-out | 6 to 9 | TEXT: the gateway uses the domain rules; EdDSA tokens with their own key; details moved to contracts; the proposal endpoint dropped (Decision 046-032) |
| structural | 1 and 5 (identifier forms) | CONTEST: constitution 4.7.0 ("Identifiers", "Words in full") and `docs/governance/identifier-scheme.md` require "Requirement 046-001" and "Owner decision 2026-11-24-4" in new prose; the template's comment predates them; the checker does not parse requirement identifiers; task and gate lines keep the old forms as the scheme says |
| structural | 2 (security exception not listed) | TEXT: Owner decisions needed names Decision 046-009 and the partial supersession |
| structural | 3 (missing risks) | TEXT: rows for the gateway down, compromised or poisoned, and the re-evaluation cost |
| structural | 4, 6 to 13 | TEXT |
| style | 1 to 6, 8 to 15 | TEXT |
| style | 7 (absolute claim) | TEXT: the claim is narrowed to reserved names |
| implementation readiness | 5, 6, 7, 11 | TEXT: one owner per shared piece; pinned versions and the probe's acceptance and fallback; the test-run path (Requirement 046-016); fixture paths |
| codebase | 3 (port taken, network name) | TEXT: port 18095; no network needed now |
| codebase | 4, 5, 7, 9, 11, 12 | TEXT |
| security | 6 (server-side request forgery) | TEXT: https upstreams and file hosts only, no redirects, validated names, runner URLs built from its base and the hash |
| security | 8, 9, 10, 11 | TEXT: identical `404`s and the accepted public-store residual (Gap 046-007); the permission table (Technical Plan 14); the marker and the hold; `python -I` and paths written twice |
| security | 13, 14, 16 to 19 | TEXT |
| architecture | 3 (ADR-0013 and ADR-0057) | TEXT: Architecture decision 0120 now supersedes ADR-0013 in part and narrows ADR-0057 decision 4 |
| architecture | 7 (checks that check nothing) | TEXT: three concrete rules with negative tests (Technical Plan 13) |
| architecture | 11 (another feature's work) | TEXT: Decision 046-033 with feature 023's Decision 1627 |
| architecture | 12, 14 to 17 | TEXT |
| correctness | 9, 10, 12, 14, 15 | TEXT |
