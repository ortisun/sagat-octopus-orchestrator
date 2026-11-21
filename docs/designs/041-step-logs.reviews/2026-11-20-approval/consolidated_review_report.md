# Approval run of design 041's revision: consolidated findings (first round)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: ff8033cd933d0c6adf972706eb1bb38407bdb7c6
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's consolidation of the first round of the approval run the owner chose on 2026-11-20 ("Full review, capped", OD-2026-11-20-17). It is a triage for the answers, not one of the four reports the approval block cites (those are written for the final commit after the answers and the short re-reads).

## The run

All five egm-review stages ran in order on `ff8033cd`, uninterrupted, each with fresh reviewers of the registered types, and
the three repository reviewers read the diff since the approved commit `f9d4a65f` at the same commit. Every reviewer
completed; none was retried.

| Report | Stage or lens | Blocker | Major | Nit |
| --- | --- | ---: | ---: | ---: |
| `zoom_out_report.md` | 5, zoom-out (two instances) | 0 | 6 | 3 |
| `comprehension_report.md` | 6, comprehension | 0 | 10 | 9 |
| `critic_report.md` | 7, design, structural, style | 1 | 11 | 21 |
| `intermediate_zoom_out_report.md` | 8, zoom-out (two instances) | 0 | 8 | 3 |
| `implementation_readiness_report.md` | 9, readiness, codebase details, style | 1 | 20 | 29 |
| `architecture.md` | architecture-reviewer | 1 | 6 | 8 |
| `security.md` | security-reviewer | 0 | 1 | 4 |
| `correctness.md` | correctness-reviewer | 0 | 6 | 9 |
| Total | | 3 | 68 | 86 |

Majors remain after the first round, so OD-2026-11-20-17's path applies: the findings marked "fix" are fixed (documents
first) and re-read briefly; the rest are accepted as recommended, one owner row each citing OD-2026-11-20-17; a Blocker is
never accepted. Every reviewer marked every Blocker and Major "fix" except Z-2 and D-12 ("accept").

## Classes of answer

- **Text:** a stale, contradictory or missing sentence, list entry, term or file; fixed in the design or its companions with
  no change of mechanism.
- **Delegate:** a design choice inside the accepted ADRs and owner decisions; decided by the coordinating session under
  OD-2026-11-20-6 and recorded as "delegate (OD-2026-11-20-6)" in research.md.
- **Owner:** the fix changes an accepted ADR's decision text, accepts a loss, or reopens an owner decision; it goes to the
  owner through the coordinating session (OD-2026-11-20-6 does not cover it).

## Findings that need the owner or the coordinating session first

| Finding | Severity | Why it is not a text fix | Proposed answer |
| --- | --- | --- | --- |
| A-1 (architecture) | Blocker | The kept room per backend cannot be read from a pooled sqlx 0.9 connection without a statement (`process_id` is private); ADR-0097 decision 3b and the warm equality assume it can | Owner (changes accepted ADR-0097 decision 3b's mechanism): top up the lease after a carrying report commits, on the same connection, from the room that carry returned (no pid needed), the first carry of a backend falling back; or a counted `SELECT pg_backend_pid()` once per acquired connection; restate the warm and cold equalities and add a test that a warm carrying report runs nothing outside its budget |
| Z-1 (zoom-out) | Major | Proposes dropping the ensure and keeping the bounded fallback alone; reverses ADR-0097 decision 3b | Owner, together with A-1 (the post-commit top-up of A-1 is the reviewer's middle ground) |
| C-6 (correctness) | Major | ADR-0095 decision 4 (a MUST of an accepted ADR) says capture never makes a report late; the ensure can, up to the statement timeout | Owner: a Note on ADR-0095 decision 4 naming the ensure's lateness, plus the ensure's own `statement_timeout` (delegate the value, for example 2 × *b* plus a margin) and a test with a lease that expires during a held ensure |
| A-2 (architecture), D-4 (critic) | Major | G041-018 (a holder of `run:logs:read` without a namespace's PII grant can watch `tenant_step_log_bytes` usage move) is a principle V condition still open; the design says none is open | Owner question: show `used` only with the grant in every namespace, or accept the residual in writing |
| D-1 (critic) | Blocker | SC-041-007 records the accepted 1 × 16 cost but keeps 90% and +10% as the gate T041-9 checks, so T041-9 fails by construction | Delegate, reported to the owner as an application of OD-2026-11-20-13: at 8 × 128 the original gates; at 1 × 16 a regression floor at the fourth run's figures with a declared tolerance (fixed before any T041-9 round); the Goals and T041-9's done rule restated. Same answer closes A-3, C-1, K-3, Z-5, R-16 |
| D-3 (critic) | Major | Capture on by default (O041-001) was decided before its cost was known | Text (a risk row weighing it, the runbook's per-namespace switch) unless the coordinating session judges the owner should be asked again; the cost was put to the owner with OD-2026-11-20-13 |
| D-2 (critic), K-3 part | Major | The ticket and cap defaults stay placeholders, since T041-35 sets them only "if it passes" | Delegate: set the defaults from the fourth run (7 tickets used at 1 × 16, 5 at 8 × 128) by the rule of D041-072, recorded as a decision |
| S-1 (security) | Major | One tenant's ensure and fallback waits hold shared pool connections; no measurement with a second tenant as the victim | Delegate: a two-tenant case in T041-9's near-limit and pooler regimes with its criterion fixed before the run, and the named remedy (a per-tenant, per-replica permit tried before a carry expected to need an ensure or a fallback) |
| I-5 (intermediate zoom-out) | Major | The busy loss and the held refusal are drops the Requirements never permit, reported in a field named `ceiling` | Text for the Requirements (the owner accepted the loss, OD-2026-11-20-14); delegate for renaming the unreleased field |
| I-7 (intermediate zoom-out) | Major | The pooler fix (session mode) removes the reason `docs/runbooks/scaling.md` gives for a pooler | Delegate: state the trade in the runbook and T041-9; reopen ADR-0097 option 1c by its declared threshold |
| I-3 (intermediate zoom-out) | Major | The store forgets its kept room after every batch, so the ensure runs before nearly every carrying report (the rejected option 1h) | Goes with A-1 |
| C-3 (correctness) | Major | The ensure's `limit` or `held` answer pre-empts `plan_batch`, so stored counts and reasons differ from the rule | Delegate: pass the ensure's answer as an input that only forbids the fallback refill, the function planning by its own allowance read; property test over the ensure's answers |
| C-4 (correctness), A-6 (architecture) | Major | Counters not derivable: fallbacks inferred from a null room, the drop-reason sets differ, two producers of the refusal metric | Delegate: an explicit `path` and a "not measured" room, one domain enum of drop reasons (with `failed`), one producer per metric |
| R-1 (readiness) | Blocker | T041-5 and T041-6 test a carry that only T041-36 builds | Text: move those tests to T041-36 or move the carry's function to T041-5 (task order) |

## Findings answered by text fixes

Stale cost and wait text (A-5, K-5, K-6, D-7, R-15, C-2, K-4, Z-3, C-7, C-8); terms and their senses (K-1, K-2, D-10, R-20,
K-10, K-12); the contended conditions (K-7); the third budget measure (K-8, D-9); the diagram (K-9); the revision note's
claim (D-8); the latency paragraph (D-11); the fallback's measured frequency and longest carry (D-6); the fitness row
(A-7); the compose smoke check (A-4, R-13); the Verification Plan's coverage of ADR-0097 and the gap list (I-1, K-15);
T041-9's place, environment and consequences (I-2, I-6, R-10); the order guard's rejected alternative (I-4, Z-4: the
duplicated write rules recorded in Complexity Tracking, or one write path); the Owner-gate lines (I-8); task boundaries,
files and seams (R-2 to R-9, R-11, R-12, R-14, R-17 to R-19); T041-1's tick against this revision's review (R-21); the
finished spike protocols (Z-6, D-12 accepted as recommended); the Complexity Tracking row of the rejected simpler
alternative (Z-2 accepted as recommended, its row corrected); the kept-room tests (C-5). Each answer cites its finding in
`resolution.md` of this folder.

## Accepted as recommended

Z-2 and D-12, each by its own owner row citing OD-2026-11-20-17, if they are still open after the fixes.
