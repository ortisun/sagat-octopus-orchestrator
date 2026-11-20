# Resolution of the reviews of 4d74b27a (hot-row revision)

Four read-only reviews of the revision of 2026-11-20 (leases, ADR-0097; the stream written once, D041-111), saved verbatim
beside this file: `correctness-reviewer.md` (1 Blocker, 5 Major), `security-reviewer.md` (0 Blocker, 2 Major),
`architecture-reviewer.md` (1 Blocker, 9 Major), `egm-design.md` (2 Blocker, 7 Major). Every Blocker and Major is answered
below; the decision is D041-112 (`specs/041-step-logs/research.md`). Each behaviour fix of the prototype was written with
its test and seen failing under a hand mutation (validation-report.md, "Hand mutations of the lease prototype").

| Finding | Answer |
|---|---|
| Correctness B1, security M1, design B1: a limit lowered below S + G refuses every charge for good | A charge that finds S + G > L goes to the refill, which reclaims the idle leases and refuses only by I4; a change of the limit or ceiling, holding its fence exclusively, reclaims every lease of the tenant (ADR-0097 decisions 3, 4, 6). Test `a_limit_lowered_above_the_usage_still_admits_what_fits` (mutation R1) |
| Correctness M1, security M2, architecture M1, design M1, correctness M3: the release waits for every lease, "under the usage row", re-creating the hot row and risking a deadlock | The release takes no lease: under the usage row it reads Σ `used`, refuses an underflow of U and lowers S, which may be negative (ADR-0097 decision 5, I3 restated; T041-4's non-negative CHECK dropped before it merges). No transaction waits for a lease. Test `a_release_never_waits_for_a_lease_and_never_goes_below_zero` (mutations R2, R3) |
| Correctness M2, architecture M3: lease contention aborts reports, a refill waits under the run lock, the savepoint option is stale | The carry runs CC to CE in one `BEGIN … EXCEPTION` block: a refused charge records a mark-only stream in the same call, and its refill takes the usage row `NOWAIT`, a held row making it `contended` (ADR-0095 decisions 5 and 7 and option 6 revised). Tests `a_carry_refused_by_the_limit_records_its_mark_in_the_same_call` (R5) and `a_carry_never_waits_for_the_usage_row` (R4); the subtransaction's cost is in T041-35's third run |
| Correctness M4: T041-6 still holds ingests "after the quota flush" | The hooks become "hold after T8 (a lease held)" and "hold inside a refill"; SC-041-001, T041-6 and the verification row state the lease tests |
| Correctness M5, architecture M3: ADR-0095 still describes the commit-time charge | ADR-0095 decisions 5 and 7, options 6 and 7 and the Consequences revised |
| Architecture B1: ADR-0084 departed from without saying so | ADR-0097 names the superseded clauses of ADR-0084 (decision 3's usage-row clause, the ingest's reading of decision 11); a Note in ADR-0084; the README line |
| Architecture M2, design m7: two acceptance paths | ADR-0097 is accepted by the coordinating session under OD-2026-11-20-6 when the re-review closes and the third run is recorded, with O041-038 and the restated O041-040; README rule 4 is answered by the delegation (flagged to the coordinator) |
| Architecture M4, design M3, correctness m6: the evidence misquotes the slot figures | ADR-0097's Evidence quotes the paired figures exactly (0.988, p99 +3.4 ms), says the slots are a proxy, cites the 8 × 128 lock timeouts with their caveat, and leaves the lease charge's cost to the third run |
| Architecture M5, design M5, security m5, correctness m1: the usage readers read S only | `sagat_quota_step_log_usage` (S + Σ `used`, `EXECUTE` to `sagat_app`); T041-5 modifies `quotas.rs` and `guards.rs`; the prototype's carry reads S + Σ `used` |
| Architecture M6, correctness m7, design m8: a down that never runs | The down refuses while a line exists, otherwise drops the leases (U = 0) |
| Architecture M7, design M6: no telemetry, held refusals look like the limit | `step_log_quota_refills_total{kind}`, `step_log_quota_refusals_total{cause}`, the lease-row gauge, a constraint name of its own for a held-room refusal, an alert |
| Architecture M8: the lock-order fitness function would reject the order | A slot for L3 and one for the lease; tries exempt and proved never to wait; a refill case |
| Architecture M9, correctness m8, design m6: the stream run index dropped without evidence | Kept |
| Design B2: no case that the cut recovers the gates | T041-35's third run states, before any round, what each change is expected to do to each gate from the breakdown (it is not expected to pass at 1 × 16 unless the gated rounds reach the paired level) and what follows a third failure |
| Design M2: the regime at the limit | Stated in section 2b and ADR-0097's Consequences; a carry's refill never waits there; T041-9 measures it with a purge |
| Design M4: the charge and the storage change together | The (g)/`g-slot` service time from run 44730444900 is reported (median 2.0 ms apart); the preview of the third shape gives paired blocks; full attribution is not claimed |
| Design M7: the lease table bloats | Fill factor 50 and per-table autovacuum settings; the risk row and (d)'s record point at the lease table |
| The prototype's own defect, found by the preview (run 44753470534) | The lease is picked by an uncorrelated scalar subquery: the `UPDATE … FROM (… LIMIT 1 FOR UPDATE SKIP LOCKED)` form could rescan and update two leases |

Minor and Nit findings taken: the explicit revokes, `public.` names, the foreign key of the leases, the refill reading the
limit itself, the NFR-041-SEC and grants lists, "three functions", the enforcement switch, the purge's "today" and one
tenant per purge transaction, the probe order expression, the fence order comment, the fill factor's cost, the
Constitution Check and Complexity Tracking rows, the stale text listed, the HOT test pinned away from midnight, the
mutation record and the prototype tests' run command. Not taken: inlining the charge into the trigger function (the
revokes and the limit read inside the refill close the same risk); a test with 257 concurrent lease holders (the cap is a
constant of T041-5, which tests the direct charge with a lowered cap).

## Re-reads of 3f1f0aba

Saved verbatim as `rereads-3f1f0aba-{correctness,security,architecture,egm-design}.md`. Every earlier Blocker and Major
was found closed except architecture M2 (as R1). New Majors, answered by D041-113:

| Finding | Answer |
|---|---|
| Architecture R1 (M2 still open): one acceptance route, O041-040 restated | ADR-0097, ADR-0095 and O041-040 share one route: the coordinating session under OD-2026-11-20-6 once the re-review has no Blocker or Major and the third run is recorded (ADR-0097 whatever its result, ADR-0095 with a pass); O041-040 (a) restated to the measurement and the per-row release, its flush replaced by ADR-0097 (design and research.md) |
| Architecture N1: an `INSERT … ON CONFLICT` before the `NOWAIT` select waits for a holder that updated the usage row | The refill locks the row first and inserts it only when absent; the test's holder now updates the row as real holders do (mutation R7) |
| Correctness N1, architecture N5, design critic N3: near the limit a held usage row loses carries uncounted (`contended`) | A held row on a refill now records the mark-only final mark (`tenant_quota`, cause `busy`); `contended` only when the existing stream's own row is held (test `a_carry_never_waits_for_the_usage_row`) |
| Design critic N1, correctness N3, architecture N2, security Nit 2: the handler catches all of SQLSTATE 54, loses a long step's mark, and §8a does not describe it | The handler matches the two constraint names and `55P03`, re-raises the rest, and takes an existing stream again `SKIP LOCKED`; data-model §8a describes it (test `a_long_steps_carry_refused_by_the_limit_records_its_mark_on_its_stream`, mutation R9) |
| Design critic N2, correctness N8, architecture N6: a subtransaction on every carrying report | A carry first reserves a lease with room for its planned charge; with one it writes with no subtransaction (test `only_a_carry_that_needs_a_refill_opens_a_subtransaction`, mutation R8); the block is kept for carries that need a refill; a risk row and T041-9's measurement cover the rest |
| Security new Major, design critic N4, correctness N6, architecture N4: the limit-change reclaim relies on its caller's fence and cannot be called by the runtime role | An `ENABLE ALWAYS` `SECURITY DEFINER` trigger on `quota_limits` and `quota_ceilings` that takes the fence itself, then the leases `NOWAIT`, then the usage row; `quotas.rs` calls nothing (mutation R6 still fails its test) |

Minors taken: T041-4's non-negative CHECK removed from its migrations now (the T041-4 migration test passes); the down
folds Σ `used` into S; a test of the S + G > L path with the reclaim disabled; the script marker's held refusal counted
`skipped_quota`; the enforcement switch in the charge; telemetry counted by sequences read through a definer accessor,
the carry's cause returned, `contended` in the carries' outcomes, an underflow counted; the stale rows of the Risks and
the Constitution Check, ADR-0027's Note, the architecture principle's row, ADR-0095's Enforced by, G041-023; the probe
expression in ADR-0097; the setting's description against ADR-0027's rule on session flags.

## Second re-reads of 35ec06b1

Saved verbatim as `rereads-35ec06b1-{correctness,security,architecture,egm-design}.md`. **No Blocker and no Major remains
from any of the four reviewers.** Their new Minors and Nits are answered by D041-114 (the reservation takes its own fence
and refuses a non-positive amount; the reservation's amount is an upper bound of the measured charge, with a property test
in T041-36; the handler's retake rechecks removal and expiry; the refill raises when the usage row is missing; the counter
sequences revoked and published by one replica; the stale acceptance and lock-order text; the busy case stated in
G041-023) or recorded there as Minors left open (a reason of its own for the busy case, the refill's `NOWAIT` under a
constraint name of its own, preferring the reserved lease).
