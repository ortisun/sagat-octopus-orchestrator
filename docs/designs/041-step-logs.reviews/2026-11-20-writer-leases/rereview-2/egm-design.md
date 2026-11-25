# Design re-read of decision D041-120 (the best-effort ensure, the bounded fallback, the fourth run's two setup steps)

Reviewer: `egm:egm-design` subagent, read-only, on the snapshot of `9361a112`. Saved verbatim.

**Scope and limits.** I read files only from the snapshot at `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/snapshot`. I had no shell, so I could not check that its commit is `9361a112`. The documents I read:
- the diff since `fc4adff7`;
- my previous re-read;
- the constitution, the design, ADR-0097, ADR-0095, research.md, data-model.md and validation-report.md;
- the prototype files `step_log_spike.rs`, `step_log_leases_tests.rs` and `main.rs`;
- `tests/support/engine.rs` and `docs/runbooks/scaling.md`, to check two claims.

No target standard was supplied beyond the constitution. I judged the documents against the questions in your task and against constitution principle VIII.

Short names for paths below the snapshot root:

| Short name | Path |
|---|---|
| design | `docs/designs/041-step-logs.md` |
| ADR97 | `docs/adr/ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md` |
| ADR95 | `docs/adr/ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md` |
| research | `specs/041-step-logs/research.md` |
| dm | `specs/041-step-logs/data-model.md` |
| spike | `tests/performance/scalability-ha/step_log_spike.rs` |
| main | `tests/performance/scalability-ha/main.rs` |

## Overall

**Most of the earlier findings are fixed:**
- The ensure now has a budget, a list of answers and a rule that it never fails the report.
- The room cache sees every charge.
- G041-025 is closed for its real cause, the collisions. The remaining loss has its own gap (G041-026) and its own owner question (O041-041).
- The held-room bound is restated correctly.
- The fourth run's setup now spreads the leases' fill levels. I checked the arithmetic of the expected refill shares and it holds: 0.31% for (a), about 2.0% for (h), at both sizes.

**What still blocks:**
- **Blocker:** the prototype's new setup cannot finish a single row.
- **Major 1:** the ensure's `busy` answer moves the wait back under the run lock, in the cold-start herd the ensure exists for.
- **Major 2:** option 1c is rejected on an argument that does not hold, and its advantage for the pooled topology the runbook advises is never weighed.
- **Major 3:** several normative texts were not updated for D041-120.
- **Major 4:** the fourth run's last rule changes, without any decision recording it, what goes to the owner.
- **Major 5:** the reading rule for gate (2b) gives the same answer whatever the round shows.

**On proportionality.** Leases owned by their writer (D041-117) solve the measured problem, the collisions, on their own. The ensure targets a different problem that has not been measured: refills under the run lock, mostly at a cold start. It brings most of the new complexity and two of the three new gaps, and the fourth run does not measure it. That is acceptable only if the owner is told so plainly; ADR97:8-9 now does tell them.

## Status of the previous findings (re-read of `fc4adff7`)

| Previous finding | Status |
|---|---|
| Major 1 (the fourth run's expectation did not follow from its setup) | Closed in the design: the share setup step and its arithmetic (design:2264-2275), and the ADR trigger reworded (ADR97:8-9). The arithmetic is right (below). But the prototype of that setup cannot run (Blocker 1), and the shares have small defects (minor 2). |
| Major 2 (the cache saw only carries; poolers) | Closed: every call that charges returns its pid and room, the cache is kept per (pid, tenant) for 9 minutes, session affinity is required, G041-027 records the pooler case, and T041-9 has a pooler scenario. Remaining: the pooler is the topology the runbook advises at scale, and option 1c was not weighed against it (Major 2). How the store learns a connection's pid is not stated (minor 8). |
| Major 3 (reclaims near the limit take the ensure's room) | Mostly closed: "rare" is qualified in section 2b, option 1g is recorded, and T041-9 measures the share. "Rare" is still unqualified in five places (Major 3). The reason given for rejecting 1g is thin (minor 5). |
| Major 4 ("at most 200 ms" stated as a hard bound) | Open in part: correct in section 2b, in "What a report can wait for", in ADR97 decision 4 and its Consequences, and in the fitness row; still stale in eight places (Major 3). |
| Major 5 (the ensure's wait had no bound and no failure path) | Closed in the design, the data model, the prototype and its tests. ADR97:145-146 still says the ensure "waits without that budget" (Major 3). The ensure's new `busy` path is Major 1. |
| Major 6 (closing G041-025 hid a kept loss) | Closed: G041-026 and O041-041 with three options, and G041-023 corrected. Leftover conflation: minor 3. |
| Major 7 (the cold-start criterion could not see the herd) | Closed: the herd is judged directly, a steady window is required, and a 100-tenant case is added. What a breach leads to is not stated (minor 10). |
| minor 1 (the held-room bound) | Closed (design:1256-1266, ADR97:156-166). G041-023 keeps one stale clause (Nit 2). |
| minor 2 (no answer for held refusals) | Closed (the reader text at design:1756-1757). |
| minor 3 (gaps in the reading rules) | Gate (4) and (2b) at 1 × 16 are now covered. Attribution by completions is still missing, and the arithmetic now shows the (2b) rule is predetermined, so it is raised to Major 5. |
| minor 4 (T041-9 criteria not usable) | Mostly closed: numbers are stated and F and the held room are recorded. One figure is unsupported, and no consequence of a breach is stated (minor 10). |
| minor 5 (the fan-out scenario had no cause for its waits) | Closed (design:2953-2955). |
| minor 6 (the busy alert would fire constantly) | Closed in the design (design:1771-1774: a ratio, and a warning naming the tenant). ADR97:185-187 still says "any increase over 5 minutes" (Major 3). |
| Nits | "the tenant's log quota was busy": closed, but the new wording claims more than the server knows (minor 6). G041-023: mostly closed (Nit 2). The source-text check: closed (ADR97:38-39). The idle reclaim described twice: closed (decision 5b removed). |

## Blocker

### Blocker 1. The fourth run's setup cannot complete a row: the reclaim runs on a pool whose connections are all held

**Where:** spike:1756-1774 (`prewarm`), tests/support/engine.rs:764, spike:1830 and :1898 (the two places `prewarm` is called), main:2183-2187 (`PREWARM_LEASE` is set before any row).

`prewarm` first acquires every connection of the pool into `held` (`while held.len() < max`). It then runs:

```rust
sqlx::query("SELECT spike_041.step_log_reclaim_idle($1)").bind(tenant).execute(pool)
```

`execute(pool)` has to acquire one more connection from that pool. All `max` connections are in `held`, so the acquire waits for the pool's 2 s acquire timeout and fails (engine.rs:764). The `unwrap_or_else` then panics with "fold the leases of ended backends". This happens for every pool:
- the four starting pools of `start()` (spike:1830);
- every replica's pool in `run_row` (spike:1898).

So with `SAGAT_PERF_STEP_LOG_CARRY_RUN=3` every round stops on its first row, and under the declared validity rule a round that stops is void (design:2278-2279). Nothing exercises this path:
- the lease tests never call `prewarm`;
- the benchmark is one of the ignored tests in "the whole performance test binary passes" (validation-report.md:804).

**Fix:** run the reclaim on one of the held connections (`&mut *held[0]`), or before the fill loop. Add a test that is not ignored and calls `prewarm` with `PREWARM_LEASE` set on a small pool, then asserts that every lease's room is 1 MiB minus its share.

## Major

### Major 1. After an ensure answers `busy`, the carry waits for the usage row again, under the run lock, so the herd the ensure exists for moves back under the lock

**Where:** ADR97:121-125, design:1203-1207, dm:266-272 and :512-513, ADR95:99-100, design:2694 (the T041-36 test), design:2932-2938 (cold start).

The store passes only `limit` or `held` to the carry. After `busy` (or after `made` whose room was then reclaimed), the report opens, its reservation finds no room, and the fallback refill waits again for the same usage row, up to *b* per lock acquisition, holding the run row. The claim "so a report waits on the usage row at most once" (ADR97:124-125, design:1206-1207) is therefore true only for `limit` and `held`.

This matters most in the case the ensure was added for. At a cold start, 160 backends queue on one tenant's row. With holds of about 1 to 4.5 ms, the ensures beyond roughly the first 40 to 200 in the queue answer `busy`. Their carries then fall back under their run locks into the same queue, where a second 200 ms wait is the likely outcome. The T041-36 test "an ensure whose usage row is held past its budget answers `busy` and the report lands" (design:2694) passes while the report waited 200 ms outside the lock and up to 200 ms more inside it.

**Fix:** choose one of these and state it:
- (a) Pass `busy` to the carry like `limit` and `held`: either a fallback that tries the usage row `NOWAIT` once, or an immediate `tenant_quota_busy`.
- (b) Give the ensure a longer budget than the in-lock fallback. It holds no run lock and only the connection the report would hold anyway; anything up to the 2 s acquire timeout is defensible.

Correct "at most once". Assert in T041-36 that after an ensure answers `busy` the report does not wait for the usage row under the run row (`pg_locks`).

### Major 2. Option 1c is rejected on an argument that does not hold, and its advantage in the topology the runbook advises at scale is never weighed

**Where:** ADR97:205-212 (option 1c), ADR97:132-134 and design:1244-1245 (the refill takes its own lease `NOWAIT`), research:163 (G041-027: "the Helm chart and `docs/runbooks/scaling.md` advise a pooler"), docs/runbooks/scaling.md:36 ("Put a pooler in front of the server before the product passes it"), research:131 (D041-120 point 2).

**The reason given does not hold.** 1c is rejected because a key per (replica, slot) "is not unique by construction", so a refill could meet a peer on "its own" lease, "which the deadlock argument of decision 4 rules out by uniqueness". But decision 4 never waits for the writer's own lease: it takes it `NOWAIT` after the usage row, and a charge or a reservation takes it `SKIP LOCKED`. With a key that two backends share during a deploy's overlap, the second one fails fast with `55P03`. The carry's block answers that as `tenant_quota_busy`, and a batch as `RETRY_LATER`. No transaction waits, so no deadlock can form. Uniqueness only guarantees that `NOWAIT` never fails; the cost of losing it is a rare, counted busy answer during an overlap, not a deadlock.

**What the rejection leaves out:**
- **Poolers.** Behind a transaction pooler, a slot is kept by the client connection. The ensure and the report run on the same slot, so G041-027 would not arise. The runbook tells deployments to add a pooler once they reach scale, which is when 8 × 128 matters. A pooler that saves server connections is usually in transaction mode.
- **Cold starts.** A slot survives pool recycles and deploys (ADR97:205-206), so most of the cold-start herd that motivates the ensure would not arise either.
- **What 1c costs.** A slot token for every writer (a `SET LOCAL` or a parameter per transaction, which counts against the budgets).

With pid keys and a transaction pooler, the store's cache keyed by pid is also meaningless, and the ensures spread room over backends the next report may not use, which inflates G near the limit.

**Fix:** restate 1c's rejection without the deadlock argument, and weigh it against G041-027 and the cold-start herd at its real cost. Or keep pid keys and say in ADR97 and in O041-042 that decision 3b has no effect in the advised pooled topology, so that "rare" fallbacks there depend on T041-9's pooler scenario. In either case, add G041-027 to O041-042's text next to G041-026.

### Major 3. The normative text is not consistent after D041-120: the old bounds, "rare", the ensure's budget and the telemetry are still stated as before

D041-120 point 4 says "waits are stated as they are" and "rare is qualified". These places still contradict it:

| Where | Stale text | Contradicts |
|---|---|---|
| ADR97:145-146 | "The ensure (decision 3b) waits without that budget" | ADR97:121 ("under the same budget as the fallback") and ADR97:267 ("waits at most the same budget") |
| ADR97:182-187 (decision 8) | `kind="ensure"`; no `step_log_quota_ensures_total`; no `kind="ended"`; no `caller="marker"`; the alert "fires on any increase … over 5 minutes" | design:1311-1315 and dm:312-316 (the metrics); design:1771-1774 (the alert: above 0.1% of carries for ten minutes) |
| ADR97:69 and :144-145 | "one bounded, counted wait of the rare fallback refill, *b* = 200 ms"; "rare (decision 3b), bounded" | ADR97:135-141 and :268-276 (per acquisition, the statement timeout as the hard bound, rare only far from the limit) |
| design:25 (Overview) | "bounded by 200 ms and rare" | design:1223-1231 |
| design:162 (FR-041-005, binding) | "its fallback refill's wait of at most 200 ms"; nothing bounds the ensure | the same; FR-041-005 is the requirement the ensure's delay must satisfy |
| design:878, :889 (lock order L4) | "waiting at most *b* = 200 ms", "waits for L4 at most *b*" | the same |
| design:1029, :1033-1036 (section 2a heading and text) | "at most 200 ms"; "when the previous carry's returned room was short"; "(a reclaim took the room in between, rare)" | design:1196-1216 (the cache per pid and tenant, older than 9 minutes, the cache being wrong, near the limit) |
| design:1826 (risk row) | "Rare … bounded at 200 ms" | design:1300-1310 and its own neighbour, the risk row at design:1843, which is qualified |
| ADR95:150 | "waited for at most 200 ms" | ADR95:78-79 and :115 (per acquisition) |

An owner reading ADR97 alone, which is what O041-042 asks them to accept, gets two contradictory statements about the ensure's bound and a telemetry and alert design that the design no longer has.

**Fix:** correct each of these cells, and grep for "at most 200 ms", "at most *b*", "rare" and `kind="ensure"` before the next re-read.

### Major 4. The fourth run's last rule puts ADR-0095 and O041-038 to the owner "whatever the result", against O041-038, D041-115, the task's own "Then" step and ADR-0097's acceptance clause

**Where:** design:2308-2309; against research:211 (O041-038: "Asked once T041-35 passes … Recommended (a) if T041-35 passes"), research:126 (D041-115: "and, if the gate passes, ADR-0095"), design:2185-2186 ("If it passes: … O041-038 (ADR-0095) is put to the owner"), design:2301 (a gate failing at 8 × 128 means "a design change, reviewed again before task T041-5 starts"), and ADR97:6-11.

The rule says the coordinating session "puts ADR-0097 …, ADR-0095 and questions O041-038 and O041-040 to the owner … as ADR-0097's acceptance clause states". The clause at ADR97:6-11 covers only ADR-0097 and O041-040. ADR-0095, the carry, is the decision the gates test. Putting it to the owner whatever the result (including after a gate failure that calls for a design change) reverses D041-115 and O041-038, and D041-120 does not record that change (research:131 point 6 covers only O041-042).

**Fix:** either limit design:2308-2309 to ADR-0097, O041-040, O041-041 and O041-042, keeping ADR-0095 and O041-038 behind a pass; or record the change in a decision, update O041-038 and D041-115, and say what the recommendation is when the gates fail.

### Major 5. The reading rule for gate (2b) gives the same answer however the round turns out, and it leaves out the most likely cause

**Where:** design:2297-2300, design:2287-2290, main:2272 and `over_p99_by_path` (main, the diff's new function), validation-report.md:752-760.

**The arithmetic.** A row of (a) has 9,000 carries, so 90 of them are slower than the row's carry p99.
- The declared share gives 28.2 refills per row; the per-pool shares give about 32 at 8 × 128 (minor 2).
- Even if every refill is slower than the carry p99, refills are at most 32 of 90, about 36%, against the rule's one-half threshold.
- The refill branch can be chosen only with at least 45 refills above the p99, 1.6 times the expected share. In practice the rule always answers "the lines' p99".

**The cause it leaves out.** The gate is the completion p99 of (a) minus off-1K's, not the carry's p99. The expectation (design:2289-2290) puts that difference near +56 ms at 8 × 128, against a carry p99 near its fast path's 31.7 to 32.7 ms. Most of the excess is therefore not the carry's own time. Two causes are more likely:
- each (a) completion's own wait for the content class's usage row (G041-024, which the leases do not touch);
- the load the carries add on a 4-vCPU runner.

A fallback refill also holds the usage row to the end of its report, which delays other completions; that never shows in its own carry time. A rule that sends a (2b) failure to chunk rows, which D041-111 rejected for the read paths, can name the wrong lever.

**Fix:** attribute a (2b) failure three ways, each with a threshold fixed now:
- (i) the completions above the completion p99 whose carry took a refill or was answered mark-only;
- (ii) the sampled commit waits behind a carrying report, against the excess (the harness already samples them: main:2278-2279, `carry_waits`);
- (iii) what remains, attributed to the lines' p99 or to the environment.

State the outcome you expect from that rule. The more likely reading of a (2b) failure at 8 × 128 is the environment and the content queue, not the lines.

## Minor

### minor 1. The arithmetic of the expected refill share: checked, and it holds
**Where:** design:2269-2275.

For (a), each carry charges 10 × 328 = 3,280 bytes. A 1 MiB grant serves 1 + ⌊(1,048,576 − 3,280) / 3,280⌋ = 319 charges, which is 0.313% of carries. A row charges 29,520,000 bytes, so 29,520,000 / 1,048,576 = 28.15 refills. For (h), 64 × 328 = 20,992 bytes a carry, 49 a grant, 2.04%, and 180.2 refills a row.

I checked the shares at both sizes:
- At 1 × 16, with 20 connections each charging about 1.41 MiB, the formula's first 20 shares give 20 + 8 = 28 refills.
- At 8 × 128, with 160 connections each charging about 0.176 MiB, 4 of every 20 shares exceed 1 − 0.176 MiB, so 32 refills.

Both are within twice 28.2. No change needed. Recorded as a finding because you asked for the check.

### minor 2. The setup shares repeat across replicas and are not the low-discrepancy sequence the formula suggests; some declared expectations are missing
**Where:** spike:1775-1786, design:2265-2268, :2273, :2287; the workloads at design:2151-2160.

- *i* restarts at 0 in every replica's pool, so at 8 × 128 the 160 backends start at only 20 distinct fill levels, each shared by 8 backends. If the replicas get the same load, the leases run dry in groups of 8. Refills are then spread over the row less than "spread over the rows" says (8 at once still queue well under *b*).
- 2,654,435,761 modulo 2^20 is 489,905, about 0.4672 MiB, close to 7/15. The sequence is close to a 15-point lattice, so pairs *i* and *i* + 15 differ by only about 8.5 KB. Knuth's golden-ratio spreading holds modulo 2^32, not modulo 2^20.
- (h) runs only at 1 × 16, so "2.0% of (h)'s at both sizes" overstates.
- G041-025's criterion applies to "every round used", which includes (f), (g) and (d), but no expected share is declared for them. For (g), batches and carries drain the same lease, and the harness does not count batch refills.

**Fix:** index the share over the whole row (replica × slot), or use an odd multiplier near 2^20/φ such as 648,055. Declare the expected share for (f), (d) and (g)'s carries, or limit the criterion to (a) and (h).

### minor 3. No consequence is declared when G041-025 stays open in a valid run, and its condition again mixes in G041-026
**Where:** design:2303-2307, research:161, design:1827.

Two outcomes have no declared consequence:
- busy answers under the 0.1% validity line, so the round is valid but G041-025 stays open;
- refills above twice the expected share, which signals collisions.

The closing condition "no carry answered `mark_only:*`" also counts the busy loss that G041-026 keeps on purpose, so G041-025 cannot close in a run that measures G041-026.

**Fix:** close G041-025 on the refill share and on `mark_only:quota` and `mark_only:held` alone. Declare what refills above twice the share lead to: the collision cause is back, a design defect.

### minor 4. G041-021 and a risk row still say no contention causes a report's first transaction to fail; D041-120 adds such a path
**Where:** research:157 ("no contention causes the first failure (D041-097)"), design:1831; against design:1228-1231 and ADR97:137-138 (past the statement timeout the carry call fails, `57014`, and the report is re-run, a re-run that can be refused).

The path is rare: it takes several lock acquisitions on one row. But it is the outcome change D041-118 was rejected for, and the documents deny that it exists. Constitution VIII requires the guarantee to be stated as it is.

**Fix:** update G041-021 and the risk row. Or enforce a real deadline (`NOWAIT` in a loop until the start time plus *b*), so that the statement timeout becomes a defect guard again.

### minor 5. Option 1g's rejection treats held refusals as worse, but near the limit both options lose records
**Where:** ADR97:223-226, design:1305-1307.

Near the limit both choices lose records. With 1g:
- the loss is labelled correctly (`tenant_quota`);
- the losers are mostly ensures, which answer `held` under no run lock;
- the protected room is bounded by the grants' shares, for a short window.

Without 1g, the ensure's room is taken again and again by other refills, so near the limit about one carry per charge falls back under its run lock, and some are labelled `tenant_quota_busy`. That is the regime the design calls normal for a full quota (design:1851).

**Fix:** weigh both losses in 1g's entry, and tie T041-9's measured share of reclaimed ensures (design:2939-2942) to a decision rule, for example: above 10%, adopt 1g with a window of one statement timeout.

### minor 6. Studio and the contract tell the reader "this is not your quota", which the server cannot know
**Where:** design:1234, :1757, contracts.md:233, ADR97:271.

A busy answer means the refill could not get the usage row within its budget. Near the limit, those waits come from the quota's own regime: a refill on almost every charge, each holding the row to its report's commit. Constitution VIII says a guarantee that is not proven must not be faked.

**Fix:** "the log service was busy, and these lines were not checked against your quota".

### minor 7. The ensure's `invalid` threshold is the attempt's ceiling, not a carry's
**Where:** ADR97:118 and design:1201 ("larger than any valid carry's"), against dm:266 and spike:387 (16 MiB + 128 × 100,001, about 29.6 MB) and design:148 (a carry is at most 64 records and 16 KiB encoded, so at most about 24.6 KB planned).

The threshold is about 1,000 times looser than the largest valid carry. The held-room bound assumes the ensure and the reserving carry use the same planned charge (design:1256-1259). A planned value that differs (through a store defect) leaves up to about 29.6 MB of grant beyond the share. That room is held whenever the backend's next report reserves its lease.

**Fix:** bound the threshold by the carry's encoded cap: 16 KiB plus 128 times 65.

### minor 8. How the store finds the pid of the connection it is about to use is not stated, and an ensure answering `room` does not refresh `charged_at`
**Where:** design:1198-1200, :1126-1127 (the warm equality: no extra statement), :1271-1278.

The store must know the backend of a pooled connection before its first statement in the report. If that takes a `SELECT pg_backend_pid()`, the warm equality of 51 statements no longer holds.

Separately, the cache refreshes its time on a `room` answer, but `charged_at` stays the time of the last charge. Suppose a backend charges at t, its cache entry expires at t + 9 minutes, and an ensure answers `room` at t + 9.5 minutes. The cache then counts as fresh until t + 18.5 minutes, while the idle reclaim takes the lease at t + 10 minutes. If that report's carry did not charge, the next carry falls back.

**Fix:** state where the pid comes from: a value kept per connection, set from the call's returned `writer` and not read by a query. Have the ensure return `charged_at` and age the cache entry from it, or have the ensure touch `charged_at`.

### minor 9. A limit change empties every lease of the tenant at once, so every backend's next carry falls back together
**Where:** ADR97:173-177, design:1283-1285, design:1214-1216.

After a change of the limit, every backend's cache is wrong at once. The next carry on each backend falls back under its run lock into one usage row: a cold-start herd with no ensure to absorb it, after every quota edit. T041-9 has no scenario for this (design:2930-2955).

**Fix:** add a T041-9 scenario "change the limit under load at 8 × 128", or return a per-tenant generation number with the room, so that a stale cache sends the store to the ensure.

### minor 10. T041-9's lease criteria have no consequence when breached, and one figure is unsupported
**Where:** design:2928-2955.

"Done when the figures are recorded"; only a failure of SC-041-007 stops the release. The cold-start, low-volume and fan-out criteria have no stated consequence.

"Under 1,000 a second (the rate below which `pg_subtrans` stays in its default buffers)" has no source. The pages `pg_subtrans` needs grow with the transaction ids between the oldest snapshot and the newest, not with a rate. At about 1,500 ids a second, 32 buffers of 2,048 ids cover about 44 seconds of horizon.

**Fix:** declare a consequence for each criterion (a design change reviewed again, or a gap for the owner). Restate the subtransaction criterion as `pg_stat_slru` misses for `subtransaction` under a stated horizon.

### minor 11. Complexity Tracking does not list the store-side room cache or the pid passed through every charging call
**Where:** design:1881.

The row lists functions, metrics and alerts. It omits the store's cache per (pid, tenant) with its 9-minute expiry, the pid and room returned by four kinds of call, and the alternatives considered for the ensure (1c, 1h, no ensure at all).

**Fix:** add them, with 1c and "no ensure, a bounded fallback only" as the simpler alternatives considered.

## Nit

1. design:1870 (Constitution Check, principle VIII): "gaps G041-001 to G041-024". The gaps now run to G041-027. The same cell lacks a comma before "one whose fallback refill".
2. research:159 (G041-023): "grants made earlier … can hold more, up to the sum of each held lease's grant" is the old bound. The restated bound is the sum of the grants' shares plus the reserved planned charges.
3. design:2263-2264: "conservative against the built design" holds for run-lock waits and for other completions' waits on the row. It does not hold for the refilling report's own latency, since the built ensure adds a commit.
4. main (`over_p99_by_path`) takes the p99 as `times[len × 99 / 100]` while the same row prints `percentile(times, 0.99)`. Use one definition. spike:2289 `lease_rows` counts the leases of every tenant, not "the tenant's".
5. design:1116 (the budget table, the ensure's row): `made` near the limit also writes the leases it reclaims, as dm:326-327 says.

## Files

Report reviewed: `docs/designs/041-step-logs.md`, `docs/adr/ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md`, `docs/adr/ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md`, `specs/041-step-logs/{research.md,data-model.md,validation-report.md,contracts.md}`, `tests/performance/scalability-ha/{step_log_spike.rs,step_log_leases_tests.rs,main.rs}`, `tests/support/engine.rs`, `docs/runbooks/scaling.md`, `docs/constitution.md`, all below `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/snapshot/`.

Verdict: **not ready**. 1 Blocker, 5 Major, 11 Minor, 5 Nit.
