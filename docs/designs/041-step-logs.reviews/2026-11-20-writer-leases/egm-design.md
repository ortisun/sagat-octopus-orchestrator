# Design critique of design 041's writer-owned leases (D041-117, D041-118)

Reviewer: `egm:egm-design` subagent, read-only, on `ba47aa81` and the merge `5458820b`. Saved verbatim.

I read every file the task named. No target standard was supplied, so I judged the documents against the questions in the task.

**Overall.** A lease per backend is the right fix for what the evidence shows: the shared pool collided, and the `NOWAIT` refill lost carries. It does not create a new hot spot; the hot spot that remains is the content class on the usage row (G041-024), for which every carrying report already waited at its commit under the run lock before the leases. So against ADR-0027 as built, waits under the run lock are fewer; against D041-112, the design accepts delay instead of losing records. The problems are around the mechanism: the fourth run's expectation does not follow from the mechanism as declared; the refill bound is labelled "derived" but is a heuristic built on the wrong inputs; the new failure path contradicts the design's own requirements; the "held" refusal still drops a carry's records while the tenant has room; and two rejected alternatives were rejected for reasons that do not hold.

---

### Blocker 1. The fourth run's expectation ignores the first refill of every new backend, which the protocol triggers in every row
**Where:** `docs/designs/041-step-logs.md:2158-2175` (Fourth run), `:2132-2133` (pools filled and closed per row, inherited from the third run), `:1178-1187` (the bound).

Leases are keyed by `pg_backend_pid()`. Every row opens new pools and closes them at its end, so every backend is new and its first charge is a refill: at 8 × 128 about 160 refills (8 replicas × 20 connections) per 9,000 carries, ≈ 1.8%. The expectation names this term ("plus one per backend per row at most") but never sizes it; it is about 6× the 0.3% expected from the bytes and the same order as the shared pool's 1.9%–3.3% slow path (`validation-report.md:731-732`). With ≈ 2% of carries on the slow path, they still sit in the top 1%, so "the carry's p99 [is] back near its fast path's" does not follow. The warm-up is also a herd the bound does not cover: the bound allows one refill holder ahead; at a row's start up to 159 first refills queue on the usage row, each holding it to its report's commit (slow-path p50 at 8 × 128 is 10.7 ms, `validation-report.md:752`); at ≈ 102 completions a second with all backends new at once the queue can reach seconds, and a refill past *b* voids the round (`:2166-2167`) — the run can be void for a reason predictable now. Production has the same pattern: after a deploy, or as pools recycle connections, every active tenant's first carry on each new backend refills at once.

**Fix:** declare lease warm-up as part of the pool-fill step (a protocol change: say so), or restate the expectation with this term — slow-path share ≈ 0.3% + backends ÷ carries per row, the expected warm-up queue on the usage row, and the expected number of refills past *b* with its basis.

---

### Major 1. The refill wait under the run lock contradicts ADR-0097's own stated force, and no risk row covers it
**Where:** `docs/adr/ADR-0097-…md:60` vs decision 4 (`:93-107`); `docs/designs/041-step-logs.md:1739`, `:892-895`, `:1011-1020`.

The Context states "a report must never wait under its run lock for another tenant's, replica's or attempt's commit"; decision 4 has a carry wait up to *b* = 2 s under the run row for commits of other replicas and attempts of the same tenant (the content class already breaks the same force, G041-024). The risk row at `:1739` says the report "never waits" for step-log locks; no row covers the refill wait. Nobody says who queues behind a run row held for 2 s (sibling reports of fan-out and collection runs; any heartbeat or lease path that takes the run row), and each queued report holds a pool connection; the benchmark's replicas acquire connections with a 2 s timeout (`validation-report.md:523-525`), equal to *b*. Fan-out is not measured (G041-019).

**Fix:** restate the force as what is guaranteed (rare, bounded, counted); add a risk row for the wait under the run lock; state what waits on the run row, with which timeouts, while a refill waits; measure a fan-out run during a refill wait in T041-9.

---

### Major 2. The refill bound *b* is not a sound derivation, and tying it to `max_connections` causes an operational hazard
**Where:** `docs/designs/041-step-logs.md:1178-1194`; `specs/041-step-logs/data-model.md:246-253`; D041-118 (`research.md:129`).

- A refill is by definition the slow path, but *r* = 32.7 ms is the fast path's p99; the written-by-a-refill p99 at 8 × 128 was 97.6 ms, 116 ms cold (`validation-report.md:752-753`).
- The formula allows exactly one refill holder ahead; warm-up (Blocker 1) and the near-limit regime ("about once per charge", `:1236-1239`) both put many refill holders ahead, each holding to its commit.
- *h* = 4.5 ms was measured at 1 × 16. At 8 × 128 the usage row already made 1,075 ingest commits wait > 200 ms (`validation-report.md:557-562`), the third run's ingest refills hit their 200 ms timeout 30 times (`:671-673`), and the split diagnosis found the row held at 55%–78% of slow-path attempts at 8 × 128 (`:731-732`); none of this enters *b*.
- (N − 1) × p99 is neither a worst case nor a percentile; `lock_timeout` applies per acquisition (tuple lock, then the holder's xid, and under READ COMMITTED possibly more waits along the update chain), so real waits can exceed *b*; PostgreSQL row locks are not strictly FIFO, so "at most every other backend ahead" is not guaranteed.
- A larger `max_connections` lengthens how long a report may hold its run row; managed PostgreSQL defaults often run to thousands of connections; at ≈ 1,700 the computed value passes the 5 s statement timeout and the startup check refuses to start, coupling availability to an unrelated server setting.

**Fix:** treat *b* as a latency budget, not a derived worst case (exceeding it is not a correctness failure; the fail-and-resend path covers it); make it a small configured value (hundreds of ms) sized from the measured distribution of usage-row waits at 8 × 128 from a diagnosis run (the split data already gives the busy fraction); drop the `max_connections` derivation and the startup refusal, or base it on the platform's own sum of pools; say that the wait can exceed *b* when it spans several acquisitions.

---

### Major 3. Options 1c and 1e are rejected for reasons that do not hold, and a cheaper option is missing
**Where:** `docs/adr/ADR-0097-…md:151-158`; D041-117 "Rejected" (`research.md:128`).

- **1c, a lease per (replica, connection slot).** "A replica's lapse would need a reclaim of its own" — the backend key needs one too (decision 5b, the idle reclaim); "a slot token from Rust" costs one `set_config` per connection. The real reason to prefer the backend id is unstated: PostgreSQL guarantees it unique, and the deadlock argument depends on that ("only a reclaim can hold my lease", so the refill's `FOR UPDATE` of its own lease under the usage row never waits on a peer); slot keys lose that if two backends ever share a slot (e.g. a rolling-deploy overlap). The backend key's costs are unstated: a first refill per (tenant, backend) after every pool recycle, and lease rows growing as tenants × backends; slot keys stay stable across recycles.
- **1e, a background refiller.** Rejected as "not needed for exactness or for a carry to write", which does not answer the force it would serve: no wait under the run lock.
- **Not considered: a top-up on the same backend, outside the report transaction.** The carry function returns its lease's remaining room; when under a low-water mark, the store runs a refill transaction on the same connection after `COMMIT`, before releasing it — the run lock is no longer held, and the report's answer can already have gone out. The in-transaction refill stays as the rare fallback. This removes most refill waits from under the run lock at no cost to exactness.

**Fix:** restate 1c's rejection with the uniqueness argument and the backend key's real costs; add the top-up option and adopt or reject it on its merits; re-argue 1e against the run-lock force.

---

### Major 4. Failing the report past *b* contradicts FR-041-005 and section 2a, and the error mapping is unspecified
**Where:** `docs/designs/041-step-logs.md:149-155` (FR-041-005), `:950`, `:1057-1061`, `:1083-1086`; `specs/041-step-logs/data-model.md:471`, `:541-542`; `docs/designs/041-step-logs.md:1075`.

FR-041-005 lets capture change a report's timing only through "the carry's own statements and … its downgrade re-run"; section 2a says "A carry never makes a report fail" and lists the only cases where capture changes a report's answer. The *b* path sends `UNAVAILABLE` to the worker, whose resend is a new report that can be refused if the lease expired in between — a residual not in G041-021. This is a real trade between the owner's "no loss while there is room" (G041-025) and FR-041-005, and no decision records it. The mapping can also lose the records it means to save: carry errors map to `StoreError::StepLogCarry` (data-model :471), which re-runs the report with no carry; and `store_error_without_text` keeps only the SQLSTATE (`:1075`), so the store cannot see the `tenant_step_log_refill_wait` constraint name it is supposed to map to `Unavailable`.

**Fix:** specify the mapping order (SQLSTATE `55P03` with that constraint name first, kept by name); prefer a server-side re-run of the same full carry once, through the existing downgrade re-run wrapper (fenced as any re-run), before sending `UNAVAILABLE`; amend FR-041-005 and G041-021; record the trade as an owner question.

---

### Major 5. A "held" refusal still drops a carry's records while the tenant has room, and the held-room bound is misstated
**Where:** `docs/designs/041-step-logs.md:1053-1057`, `:1207-1212`, `:1226-1230`; `docs/adr/ADR-0097-…md:113-115`, `:189-191`; G041-023 (`research.md:157`).

A carry refused as `tenant_step_log_quota_held` becomes mark-only and its records are lost for good — the defect class the owner would not accept (G041-025), with a different cause. Ingests get `RETRY_LATER` in the same case; carries do not. The gate cannot show it (the tenant has the catalogue's maximum). The bound "at least L · C(2k,k)/4^k" is true only when S + G = 0; in general it is F₀ · C(2k,k)/4^k with F₀ the free room when the grants began, and the test (k = 8, L = 1 MiB, 205,920) checks only S + G = 0. The stated inequality runs the wrong way: C(2k,k)/4^k < 1/√(πk) for every k (k = 8: 0.19638 vs 0.19947); the valid lower bound is 1/√(π(k + ½)). For a small quota this bound is not the one that matters: with k up to 128 open writers only ≈ F₀/20 is guaranteed (up to ≈ 95% of the free room can be held); what matters is the share of charges that refill (grants of F/(2(A + 1)) with A up to the number of backends; the reclaim threshold makes every refill reclaim when F < 1 MiB) and the rate of held refusals.

**Fix:** make a carry's held refusal behave like the *b* path (a retryable re-run), not mark-only; correct the bound to F₀ · C(2k,k)/4^k ≥ F₀/√(π(k + ½)) in the ADR, section 2b and G041-023; in T041-9, measure a small quota with many backends (refills per charge, held refusals, usage-row waits).

---

### Major 6. The fourth run cannot attribute a fail, and a fail's meaning is not declared in advance
**Where:** `docs/designs/041-step-logs.md:2169-2178`; `validation-report.md:476-477`, `:680`, `:702-706`, `:762-763`.

- **1 × 16.** The declaration already expects (1), (2b) and (3) to fail there. D041-106 showed why: capture with no quota charge already costs ≈ 9% of the rate on that runner (`f-none`/off-{} 0.911). The paired diagnosis gives (a)/off-1K 0.89–0.90, but the gated rounds give 82.8%: the protocol's own error is ≈ 7 points, larger than anything D041-117 changes at 1 × 16. A fail there says nothing about the leases; the environment question to the owner can be asked now.
- **8 × 128, (2b).** The expected +56 ms sits against an allowance near 61 ms, and (a) − off-1K p99 has ranged over [−221, +712] and [−7, +354] ms across rounds: a fail is a coin flip unless decomposed. The +56 ms baseline is from the second run, whose carry had no leases and no refill waits.
- **No rule for interpreting a fail**, so a fail triggers the same "take it to the owner" whatever the cause.
- **The void rule treats a design defect as noise:** a refill past *b* voids a round like drift, so that defect would surface only as "not decided".
- **The counter mixes ingest and carry timeouts:** `step_log_quota_refill_wait_timeouts_total` also counts ingest refills cut off at their 200 ms timeout (data-model :248-253); the third run had 30 at 8 × 128; the alert, and possibly the void rule, cannot tell them apart.

**Fix:** before any round, declare: (i) the validity counts (zero `mark_only:*`, zero carry refills past *b*) close G041-025 independent of the gates; (ii) a refill past *b* is recorded as a design outcome, not a void; (iii) a (2b) fail at 8 × 128 is attributed by the share of `written:refill` carries in the slowest 1% of completions — above a stated threshold the lever is the top-up or 1e, below it the lines' p99 (chunk rows); (iv) a 1 × 16 fail on (1) or (3) is the known cost of the carry on that runner. Label the timeout counter by caller (`carry`/`ingest`) and alert only on carries.

---

### Major 7. "About one carry in 300" holds only for a high-volume tenant far from its limit, so the subtransaction cost rejected in option 10 comes back
**Where:** `docs/adr/ADR-0097-…md:142-144`, `:180-183`, `:197-199`; `docs/designs/041-step-logs.md:1017`, `:1755`; `specs/041-step-logs/data-model.md:252`.

A grant reaches 1 MiB only when F ≥ 2(A + 1) MiB, and a lease survives only if its backend charges for that tenant within 10 minutes. For a tenant with less than ≈ 1 MiB of carries per backend per 10 minutes (most tenants of a multi-tenant deployment with ≈ 160 backends), and for any small quota, nearly every carry is a refill, each taking the usage row under the run lock and opening subtransactions. Each slow-path carry opens two (the carry's block, and the refill's own `BEGIN … EXCEPTION` around its lock, used to count the timeout, data-model :252), while the ADR says one; in aggregate this can approach the XID and `pg_subtrans` pressure option 10 was rejected for.

**Fix:** model the refill share as a function of tenant volume, backends, F and A, and qualify every "1 in 300"; count the timeout in Rust from the mapped error, dropping the refill's subtransaction; add a many-tenants, low-volume scenario to T041-9 with the XID rate and `pg_stat_slru`.

---

### Minor 1. The grant and fold rule is written three different ways
ADR decision 4 (`:109-111`) and design `:1175`: fold `used` only, grant max(n − room, …). data-model `:255-259`: fold the whole lease, grant max(bytes, …). Fourth run `:2161`: max(n, …). Mutation W8 (`validation-report.md:785`) shows the prototype folds the whole lease. D041-117 (`research.md:128`) gives the order "fence, writer's lease, usage row"; ADR decision 4 and design `:1195` give "fence, usage row, own lease". **Fix:** make the ADR and section 2b match data-model §3b and the prototype, and state both orders (the charge tries the lease first; the refill takes the usage row, then the lease).

### Minor 2. The lock-order text contradicts the refill and the planned test
`docs/designs/041-step-logs.md:870-871`, `:874`, `:883-886`. "Only a holder of L4 could hold [its own lease]" is false as worded: the limit-change trigger holds leases before it takes L4; the argument holds only because that trigger has the step-log fence exclusively, as section 2b says. The planned test is to check that "after L4 no writer takes a lock on an existing row", but the refill locks its own existing lease with a waiting `FOR UPDATE` (data-model :255), which is not "only tried" and so not exempt. Line 874 says the purge takes L3b; decision 5 says a release takes no lease. **Fix:** write the exception for the refill's own lease into the order and into the test.

### Minor 3. The startup check and the bound use different numbers
`docs/designs/041-step-logs.md:1188-1189`; `specs/041-step-logs/data-model.md:246-247`. The check uses the unrounded value but *b* is rounded up: at 1,103 connections *b* is 5 s, equal to the statement timeout, so the timeout fires first (`57014`, not counted, not mapped). The statement timeout covers the whole carry call, so the check must be *b* plus the carry's time before the refill, below 5 s. The 4.5 ms and 32.7 ms are constants hard-coded in SQL, taken from a hosted runner.

### Minor 4. The reservation's size is stated two ways
`specs/041-step-logs/data-model.md:536` vs `:268-271` and ADR `:91-92`. §8a reserves "the kept rows' text bytes plus 128 a row" after the plan and without the marker; §3b and the ADR say every sent record plus 128 for a possible marker, computed before CC. Under §8a's version, a ceiling marker's charge can exceed the reserved room; it then refills on the fast path, which has no block, so a refusal there fails the report.

### Nit
- G041-023 (`research.md:157`) still says "at most 1/64 of the free room".
- The risk row at `docs/designs/041-step-logs.md:1754` says "at most 256 lease rows".
- The risk row at `:1735` says every carry opens one subtransaction.
- The nested-statement count is 23 at `:1111` and 26 at `:1251`.
- The ADR's acceptance trigger (`ADR-0097:6-9`) still names the third run.
- The ADR's Evidence has no entry for the cost of the first refill per backend.
