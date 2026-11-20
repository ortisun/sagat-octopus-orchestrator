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
