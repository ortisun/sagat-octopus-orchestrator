# Round 3 of 2026-11-24: what answers each finding

Round 3 ran the three repository reviewers (architecture, security, correctness) on
`4c35bed4ed55c5aa10b08e2dd1749ffbb9be3dd1`: no Blocker, 22 Majors. The `egm-review` stages were not run again in this
round (the agent slots of the session were exhausted); the round-2 `egm-review` reports stand for this text with their
resolution. Every finding below is answered in the text; a fourth round must read the result before the design can be
approved. Decisions 046-051 to 046-064 and Gap 046-010 record the choices, under Owner decision 2026-11-24-4.

| Report | Finding | Answer |
|---|---|---|
| architecture | 1 (lease placement) | TEXT: a typed admission on both lease paths (Technical Plan 12; Decision 046-052); feature 023's plan and Decision 1628 aligned |
| architecture | 2 (backfill) | TEXT: the `BACKFILL` phase signs records without touching the version (Technical Plan 9; Decision 046-059) |
| architecture | 3 (silent wait) | TEXT: failure reports, a gauge, `LAYER_UNAVAILABLE` after three failures or ten minutes (Technical Plan 11; Decision 046-053) |
| architecture | 4 (ADRs lag) | TEXT: ADR-0120 and ADR-0121 rewritten in their Refines, Enforced by and decisions |
| architecture | 5 (gateway isolation unenforced) | TEXT: a dependency allow-list rule (Technical Plan 13) |
| architecture | 6 (switch matching) | TEXT: precomputed pairs, ranges only for later rows (Technical Plan 7; Decision 046-055) |
| architecture | Nits 7 to 11 | TEXT: grant claims for targets and interpreter; two more ports; the import role; the activation test; MUST NOT |
| security | 1 (relabelled files) | TEXT: registry hashes per file; the unpacker's identity check (Technical Plans 3, 5, 8; Decision 046-060) |
| security | 2 (bytecode outside the digest) | TEXT: no bytecode in layers; the compile container removed (Decision 046-061; Gap 046-010) |
| security | 3 (reserved names on metadata and file requests; npm scopes) | TEXT: three forwarded route kinds with defined name extraction; private metadata and file routes (Technical Plan 4, `runner-protocol.md`; Decision 046-062) |
| security | 4 (npm times) | TEXT: full documents read with a capped streaming parse; versions without a time dropped (Technical Plan 3; Decision 046-063) |
| security | Nits 5 to 9 | TEXT: forwarded bytes charged; tenant keys and attempt-derived namespaces; the import role stated; upload bounds; the hold refuses matching finishes |
| correctness | 1, 2 (stuck build job, deadlock) | TEXT: no sibling writes; claims and quota count only `BUILDING` versions (Technical Plan 1; Decision 046-051) |
| correctness | 3 (boolean admission) | TEXT: typed admission (Decision 046-052) |
| correctness | 4 (blocked step never leased) | TEXT: the block pass (Technical Plan 6; Decision 046-053) |
| correctness | 5 (page removes a fresher block) | TEXT: reservation revision held; revisions recorded on blocks; tenant ordering (Decision 046-054) |
| correctness | 6, 7 (older import, hold delays malware) | TEXT: recheck under the lock; a held import still refuses (Decision 046-055) |
| correctness | 8, 10 (waiver expiry, evaluation placement) | TEXT: earliest waiver expiry; `StaleInputs` from the store (Technical Plan 1; Decision 046-056) |
| correctness | 9 (test-run start check) | TEXT: runnable per attempt or test run (Technical Plan 11; Decision 046-057) |
| correctness | 11, 12 (collected files, install fetch failures) | TEXT: name-carrying routes; `PACKAGE_FILE_UNAVAILABLE` (Technical Plan 8; Decision 046-058) |
| correctness | Nits 13 to 15 | TEXT: the route path; deadlines that grow with the layer; test wording |
