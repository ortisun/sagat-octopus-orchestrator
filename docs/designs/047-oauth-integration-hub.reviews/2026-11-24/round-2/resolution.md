# Round 2 of 2026-11-24: what answers each finding

Round 2 read `e1d438b0ff4e17994feb29517a4d0cd2582b1349`. Six reports came back (security, architecture, correctness,
the zoom-out, the comprehension test and the design critique): 3 Blockers and 43 Majors. The structural, style,
implementation-readiness and codebase stages of `egm-review` were not run on this text (the agent slots of the session
were exhausted); a third round runs all of them on the revised text. TEXT: the design, a contract, the data model, an
architecture decision or feature 021's contracts now say it. OPEN: not this design's to settle. Decisions 047-039 to
047-060 and Gap 047-008 record the choices, under Owner decision 2026-11-24-4.

| Report | Finding | Answer |
|---|---|---|
| correctness | Blocker 1 (disconnect between answer and store) | TEXT: store first once answered; a holder that loses to a disconnect revokes; the pass revokes pending revisions (Technical Plan 4; Decision 047-040) |
| security | Blocker 1 (client credentials on a shared client) | TEXT: refused by a check constraint (Requirement 047-002; Decision 047-048) |
| architecture | Blocker 1 (the Principle V wording) | TEXT: the full wording with the binding per executor and the replacement sentence (Technical Plan 12; Decision 047-051). OPEN: it must be merged into the constitution; feature 021 records the dependency (its Decision 021-022) |
| correctness | 2, 3, 4 (vault mark, adoption data, row bound) | TEXT: a written mark and a probe; origin, epoch, expiry and scopes on pending rows; retirement and the sweeper (Decisions 047-041, 047-042) |
| correctness | 5 (clock and timeout) | TEXT: `CLOCK_BOOTTIME` before the acquire; timeout stated (Decision 047-043; Gap 047-007) |
| correctness | 6, 7 (lock order, fence writers) | TEXT: the credential row first; state predicates; terminal refusals; the grant epoch (Decisions 047-044, 047-045) |
| correctness | 8 (delivery counting) | TEXT: one per-attempt row (Decision 047-057's companion in data-model.md) |
| correctness, security, design | delivery checks (correctness 9, security 2, design 1) | TEXT: feature 021's checks, `resource:effect` for declarations (Decision 047-046) |
| correctness, design, comprehension | `invalid_client` and rotation (correctness 10, design 5, comprehension 3) | TEXT: `BROKEN` after three answers, rotation in place (Decision 047-047) |
| correctness, architecture, security | keys and imports (correctness 11, architecture 5 to 7, security 8) | TEXT: deployment tables apart, tenant keys, `NULLS NOT DISTINCT`, feature 028's routes and permissions (Decision 047-055) |
| security | 3 (wording) | TEXT, as architecture Blocker 1 |
| security | 4 (credential to a foreign host) | TEXT: binding only to the provider's API hosts (Requirement 047-018; Decision 047-050) |
| security | 5 (disconnect race) | TEXT, as correctness Blocker 1 |
| security, architecture, design | rate limits (security 6, architecture 9, design 4) | TEXT: windows in PostgreSQL per registration and per tenant (Decision 047-048) |
| security, zoom-out | the feed (security 7, zoom-out 1) | TEXT: revocations travel in feature 028's indexes; a revocations-only index; no feed (Decision 047-049) |
| architecture | 2 (clause 11(e)) | TEXT: Architecture decision 0122 widens it (Decision 047-052) |
| architecture | 3 (operator routes) | TEXT: the platform-operator surface, schema-owner commands until then, a definer function (Decision 047-053) |
| architecture | 4 (rollback) | TEXT: remove the requirement first; `UNIMPLEMENTED` retryable (Decision 047-054) |
| architecture, design, comprehension | codes (architecture 8, design 6, comprehension 4) | TEXT: reasons under existing codes; three rows in feature 021's list; the error catalogue (Decision 047-056) |
| architecture | 10 (timing in the application) | TEXT: the `Waiter` port; passes in `apps/control-plane` (Decision 047-057) |
| architecture | 11 (checks that do not fail) | TEXT: the string-literal scan, the allow-list, the `pg_stat_activity` test (Decision 047-058) |
| architecture | 12 (stale ADRs) | TEXT: Architecture decisions 0122 and 0123 rewritten |
| design | 2 (raw tokens outside presets) | TEXT: a risk row and the proxy alternative (Gap 047-008) |
| design | 3 (tokens without expiry) | TEXT: a default lifetime; `401` feedback; GitHub App user tokens (Decision 047-059) |
| design | 7 (security revocation and delivery) | TEXT: delivery refused too (Requirement 047-023) |
| design | 8 (offers of client credentials) | TEXT, as security Blocker 1 |
| comprehension | 1 (phases) | TEXT: one phase list; early work allowed, acceptance per phase |
| comprehension | 5 (publishing imported providers) | TEXT: `hub:publish` for imported, `oauth:client:manage` for authored |
| comprehension | 6 (the incomplete record) | TEXT: the hourly pass writes it |
| zoom-out | Nits 2 to 6 | TEXT: statements moved to data-model.md, timings to timings.md; `SKIP LOCKED` instead of a lease row; the phase table. CONTEST in part on Nit 6: the per-attempt row stays, because a worker is outside the platform and its re-asks must be bounded where they are served |
| the others | Nits | TEXT |
