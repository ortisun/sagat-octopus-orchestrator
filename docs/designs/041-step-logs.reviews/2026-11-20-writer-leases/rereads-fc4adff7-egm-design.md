# Design re-read of decision D041-119 (the ensure before the report, the 200 ms fallback, the fourth run's setup lease)

Reviewer: `egm:egm-design` subagent, read-only, on `fc4adff7`. Saved verbatim.

The review is incomplete in one respect: I had no shell, so I could not confirm that the worktree is at `fc4adff7`. I read the files as they are on disk. No target standard was supplied, so I judged the documents against the questions in your task.

Files read: the design ("design"), ADR-0097 ("ADR"), research.md, data-model.md, validation-report.md, and the earlier egm-design.md and resolution.md of the writer-lease review.

**Overall.** The ensure before the report is the right mechanism, and the 200 ms fallback that answers mark-only is the right trade compared with failing the report. ADR-0084 decision 7 decides that, and the busy answer is counted, alerted and shown to the reader. Four things keep it from doing what the text says: the room cache can be wrong; the ensure's room can be taken back near the limit; the ensure's own wait has no bound and no failure path; "at most 200 ms" is promised where the mechanism only limits each lock wait. The fourth run's expectation is now wrong in the other direction: at 8 × 128 no refill happens inside a row. The cold-start criterion cannot see the herd it was written to catch.

## Status of the previous findings

| Previous finding | Status |
|---|---|
| Blocker 1 (first refill per new backend) | Closed for the spike by the setup lease, and moved to T041-9's cold start. The new expectation is wrong the other way (Major 1), and the cold-start criterion is blind (Major 7). |
| Major 1 (waiting under the run lock against the force) | Closed in substance: the force is restated, risk row `design:1781` exists, and T041-9 has a fan-out scenario. Remaining problem: the 200 ms is stated as a hard bound (Major 4). |
| Major 2 (*b* derived from `max_connections`) | Closed. Same remaining problem (Major 4). |
| Major 3 (options 1c and 1e, the top-up) | Closed. 1c is restated with the uniqueness argument, 1e is re-argued, and the top-up became decision 3b. Its gaps are Majors 2, 3 and 5. |
| Major 4 (failing the report past *b*) | Closed. The busy answer is caught in SQL and FR-041-005 is amended. The ensure's failure path is new and unspecified (Major 5). |
| Major 5 (held refusal and the bound) | The bound is fixed, with a remaining gap (minor 1). The held-refusal half is not answered (minor 2). |
| Major 6 (reading rules) | Mostly closed. Gaps remain (minor 3), and closing G041-025 this way is not honest (Major 6). |
| Major 7 ("one in 300", subtransactions) | Closed. The low-volume criterion is not fixed in advance (minor 4). |
| Minors 1, 2 and 4 | Closed. |
| Nits | Mostly closed. G041-023 is still stale (see the nits below). |

### Major 1. The fourth run's expectation does not follow from its declared setup: at 8 × 128 no refill happens inside a row
**Where:** `design:2209-2216` (the setup lease), `:2223-2228` (the expectation), `:2233-2235` (rule ii); `ADR:6-9` (the acceptance trigger); `validation-report.md:281`, `:738`, `:201-202`.

**The arithmetic:** A row has 9,000 carries per workload. An (a) carry charges 3,280 bytes, so a 1 MiB lease lasts about 320 carries. At 8 × 128 the carries spread over about 128 to 160 backends, so each backend makes about 56 to 70 carries per row — about a fifth of one lease. So at 8 × 128 almost no refill happens inside a report, not "about 0.3%". The size that carries gate (2b)'s close call measures only the fast path. At 1 × 16, each of 16 to 20 backends makes about 450 to 560 carries, so about one refill per backend. Every lease starts full at the same moment and drains at the same rate, so these refills come as one burst of 16 to 20 under run locks, about 60% to 70% into the row.

**Why it matters:** "Conservative against the built design" (`:2213-2214`) means nothing at 8 × 128: zero refills is the optimistic case. The built ensure (decision 3b) is measured nowhere before the ADR goes to the owner, yet `ADR:8` says the fourth run "measures this ADR's text". Rule (ii)'s refill branch cannot be reached at 8 × 128.

**Fix:** Restate the expectation for each size, with the arithmetic above. Better: make the setup grant each connection a uniformly random share of 1 MiB. Each backend then refills once at a random point, about 0.3% of carries, spread over the row. This makes the declared 0.3% true and removes the synchronised burst. Reword the ADR trigger: the fourth run measures decisions 1 to 4 on the leases; decision 3b is measured by T041-9.

### Major 2. The room cache only sees carries, so the steady refills still happen under the run lock
**Where:** `design:2565-2568`, `:1181-1188`; `data-model.md:561-562`; `ADR:100-103`. Pooler claim: `ADR:70-72`, `design:1170-1171`.

**Other charges are invisible to the cache.** Ingest batches and script markers charge the same backend's lease, from the same pool (`design:930-932`, `:1189-1191`). A batch can charge up to 192 KiB plus 128 bytes a row. Only a carry's returned room updates the cache. When a lease runs out between an ingest and the next carry, the cache still shows room, no ensure runs, and the refill happens inside the report, under the run lock. For tenants with long steps (workload (g)), that is roughly the batches' share of the bytes charged.

**A transaction pooler breaks "same connection, so same backend".** Behind a pooler in transaction mode, the ensure and the report can run on different server backends. The ADR says the backend key works behind such a pooler; decision 3b does not.

**Fix:** Have every call that charges or refills (an ingest's T8, the script marker, the ensure, the carry) return the room left and update the cache. Key the cache by backend pid plus room. Either state that decision 3b needs session pooling, or withdraw the pooler claim. Say where the cache lives and when it is dropped (connection closed, idle reclaim). Add to T041-9: fallbacks per carry in (g), with a criterion.

### Major 3. Near the limit, other refills take back the ensure's room, so a tenant that sits at its limit gets fallbacks on almost every charge
**Where:** `ADR:126-127`, `design:1192-1193`, `:1269`, `:1806`; the "rare" claims at `design:153-154`, `:898`, `:1781`.

**The race:** When free room is under max(*n*, 1 MiB), a refill reclaims every lease that no open transaction holds. An ensure commits its grant. The report then begins, takes the run row, and reserves the lease only just before CC. For that whole window the lease is held by no transaction, so any other backend's refill near the limit takes its room. Near the limit there is about one refill per charge, so this race is frequent.

**Why it is not rare:** the design says a full step-log quota is a normal state ("fills in hours … until old streams expire", `:1806`). Such a tenant hovers in the band where the purge frees room and carries refill. "Rare" is true only far from the limit. Busy answers then call it "the quota was busy" for a tenant that is in fact full.

**Fix:** Exempt an ensure's grant from the near-limit reclaim for a short window, for example a `reserved_until` column, or skip grants younger than one statement timeout. The room stays counted in G, so exactness is kept. Add those grants to the held-room bound. Or qualify "rare" everywhere as "far from the limit", and give the run-lock rate at the limit in risk row `:1781`. T041-9's near-limit scenario should report the share of ensures whose room was reclaimed before the reserve.

### Major 4. "At most 200 ms" is stated as a guarantee in places where the mechanism only limits each lock wait
**Where:** Correctly stated: `design:1203-1204`, `ADR:115-116`. Overstated: FR-041-005 `design:154`; the lock order `:870`, `:881`; "What a report can wait for" `:898-902` ("queue behind it for at most *b*"); section 2a's heading `:1019`; risk row `:1781`; `ADR:234-235`; T041-9's fan-out criterion `:2865-2866`.

**The problem:** `lock_timeout` limits each lock acquisition, not the whole wait. On a row with a queue, a `SELECT … FOR UPDATE` waits for the tuple lock, then for the holder's transaction id, and under READ COMMITTED possibly again for a newer version. Two or more acquisitions are the normal case on a busy row, so the real hard bound is the statement timeout, 5 s.

**Fix:** Everywhere "at most 200 ms" appears, state the actual guarantee: each acquisition at most *b*, the statement timeout as the hard bound. Or enforce a real deadline: `FOR UPDATE NOWAIT` in a loop with a short sleep until `clock_timestamp()` passes the start plus *b*. Record the refill's maximum measured wait in the fourth run and in T041-9. Set the fan-out criterion from that maximum, not from *b*.

### Major 5. The ensure's wait has no bound except the statement timeout, and nothing says what happens when it fails
**Where:** `data-model.md:261-264`, `ADR:104-105`, `design:1185`, FR-041-005 `design:153`.

**The wait:** The ensure waits for the step-log fence shared, so it queues behind a queued limit change. It then waits for the usage row for up to 5 s, holding a pool connection the whole time. The benchmark's pools give up acquiring a connection after 2 s. FR-041-005 now lets the ensure add to a report's time with no bound at all, after the revision dropped a 2 s bound for being too long. A cold start or a busy usage row becomes report latency and pressure on the pool that heartbeats and leases use. Lease expiry is another way for capture to change a step's outcome.

**No failure path:** nothing says what the store does when the ensure fails: `57014` (statement timeout); `55P03` (lock not available); `tenant_step_log_quota_exceeded` or `tenant_step_log_quota_held`; a lost connection.

**Fix:** Give the ensure a budget: try the fence instead of waiting for it, and a `lock_timeout` on the usage row. State that any ensure error or refusal is counted and dropped, and the report goes on to its reserve. Pass a quota refusal from the ensure to the carry, so it answers `tenant_quota` without a second refill under the run lock. Add T041-36 tests for each case, and add the ensure's bound to FR-041-005.

### Major 6. Closing G041-025 through the fourth run hides a loss path the design keeps, and the owner has not accepted that loss
**Where:** `research.md:160` (G041-025), `:130` (D041-119), `:158` (G041-023); `design:1782`, `:2231-2233`; `resolution.md:13-14`.

G041-025 records that the owner "will not accept" losing records for no quota reason. D041-119 keeps such a loss (the busy answer past *b*, visible), approved by the coordinating session, not by the owner. Rule (i) closes G041-025 when no carry is answered mark-only. But at the catalogue's maximum quota, with no refills inside a row at 8 × 128 (Major 1), zero busy answers is close to certain by construction. The regimes where busy answers occur are all T041-9's, run after the build: near the limit, a small quota, a cold start, and connections shared with ingests. G041-023 still says a refill past its bound fails the report and is resent. G041-025 still says "bounded and loud".

**On the trade itself:** answering mark-only after 200 ms is acceptable and better than failing the report. The issue is how it is recorded, not the choice.

**Fix:** Close G041-025 for its measured cause (the shared pool's collisions). Open a new gap: "records dropped as `tenant_quota_busy` past the fallback's budget", with how often it happens near the limit. Put it to the owner as an explicit choice: accept the visible loss, fail and resend, or write the records after the commit. Tie its acceptance to T041-9's near-limit and cold-start criteria, and correct G041-023.

### Major 7. T041-9's cold-start criterion cannot detect the herd
**Where:** `design:2851-2856`; `validation-report.md:201-202`, `:281`.

At about 470 completions a second, a 9,000-transition row at 8 × 128 lasts about 19 s. "The first 30 seconds" is then the whole row, and "the same row's steady p99" does not exist. Even with longer rows, the herd is one report per backend, about 160 out of roughly 14,000 in 30 s (about 1.1%). The p99 barely moves even if each of those reports waited seconds. "No `mark_only:busy`" cannot fail because of the herd: ensures have no budget and never answer busy.

**Fix:** Judge the herd directly: the maximum and p99 of the ensure's wait; the time until every backend holds a lease; the maximum completion time of each backend's first carrying report (for example, under the 2 s acquire timeout); no heartbeat or lease call failing to get a connection. Fix the row length so that a steady window exists. Add a cold start with many tenants (backends × tenants ensures).

### minor 1. The held-room bound ignores the writers' own charges and the ensure's grants
**Where:** `design:1227-1235`, `ADR:131-137`, `research.md:158`.

**The step that fails:** F_i ≥ F_{i−1}(1 − 1/(2*i*)) holds only when the grant is the F/(2(*A* + 1)) term. When max(*n*, …) = *n*, the free room drops by *n*. For example, F₀ = 32 KiB and *n* = 24 KiB leaves F₁ = 8 KiB, less than F₀/2.

**What the bound leaves out:** An ensure grants max(planned, …) and charges nothing, so a carry that has reserved but not yet charged holds its whole planned charge. A reserve keeps its lease locked until the report commits, so held room is always present at high concurrency: tens of leases at 8 × 128. It is not a passing "in flight" effect.

**Fix:** State the bound on unused held room, plus the planned charges of reserved carries. Add the bound an operator can read: a held refusal happens only within min(k_open × 1 MiB, …) + *n* of the limit, where k_open is the tenant's number of concurrently open charging transactions.

### minor 2. Half of the old Major 5 (held refusals) has no answer
**Where:** `resolution.md:33`; `design:1714`, `:1235-1236`.

Neither the answer nor the "Not taken" list covers carries losing their records on a held refusal. Studio shows "quota is full" while the quota view shows room left. **Fix:** record it as not taken, with a reason, or give held refusals their own reader text.

### minor 3. The reading rules have gaps
**Where:** `design:2229-2238`.

Rule (ii) attributes by the carry's own time, but the gate is completion p99. A refill holds the usage row to its commit, which also delays other completions' content finalizers ((a) stores a 1 KiB output); that delay never shows in the refill carry's own time. Rule (iii) covers (1) and (3) at 1 × 16, but (2b) at 1 × 16 is also expected to fail. Gate (4) has no reading rule. **Fix:** add those cases, and attribute (ii) by completions as well as by carries.

### minor 4. The T041-9 criteria are not usable as written before the runs
**Where:** `design:2861`, `:2863-2864`.

"Within the deployment's freeze budget stated in the runbook": that number does not exist yet. "Held refusals only while the free room is under the bound of section 2b": checking this needs *k* and F₀ at each refusal, which the harness does not record. The low-volume scenario has no latency criterion, although nearly every carry there pays an extra transaction with its commit. **Fix:** state the numbers now, record F, the held leases and their room at each refusal, and add a completion p99 criterion.

### minor 5. The fan-out scenario does not say how fallback waits are caused
**Where:** `design:2865`.

Far from the limit there are no fallback waits to measure. **Fix:** cause them on purpose, either with a test transaction that holds the usage row for 150 ms and for 400 ms, or by running near the limit.

### minor 6. The busy alert will fire constantly
**Where:** `design:1728-1731`, `ADR:160-161`.

The alert fires on any increase over 5 minutes. A tenant sitting at its limit (Major 3) keeps it firing, and with no tenant label nobody can tell which tenant it is. **Fix:** alert on a ratio of busy answers to carries, and put the tenant in a log line or span attribute that the runbook points to.

### Nits
- `design:1714`: "the tenant's log quota was busy" reads as a quota problem, so users may raise their quota. Say the platform was busy and that it is not their quota.
- `research.md:158` (G041-023): "a grant is at most 1 MiB, or *n* − room" should be max(*n*, …). Its parenthetical still says the report fails and is resent.
- `ADR:31-32`: the source-text check leaves out the ensure and the idle reclaim; `design:2367` includes them.
- `ADR:73-76` and `:144-146` describe the idle reclaim twice; only the first mentions trying the fence.
