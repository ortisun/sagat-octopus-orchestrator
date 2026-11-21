**Verdict.** At c52b5d37, S-1 is fixed for a replica's own pool with direct or session-pooled connections. The arithmetic of D041-134 checks out: 10 + 5 of 20 held at most, 5 left; 4 + 2 of 8, 2 left. Three tenants can no longer take 21 of 20.

The fix brings one new Major. The bound relies on session settings and per-replica slots, and that does not hold behind a transaction-mode pooler. The runbook recommends exactly that setup at scale. No new Blocker. I found nothing new on tenant boundary, secrets, authorization, injection or the runtime role's privileges.

This is a read of the documents at that commit; I ran nothing.

## Earlier findings

- **S-1: resolved for direct connections, partly behind a pooler.**
  - The replica cap Q and the tenant cap P = ⌈Q/2⌉ bound the waits across tenants. Logs cap plus Q leaves N − ⌊N/2⌋ − Q connections free.
  - The top-up takes no slot and never waits. It only tries the usage row with NOWAIT, so it adds nothing to the count.
  - The slots and the tickets add up conservatively: a waiting batch counts against both the cap and Q.
  - Q now follows the pool size.
  - Where it is stated: docs/designs/041-step-logs.md:1360-1375 and :2055, specs/041-step-logs/data-model.md:251-275 and :469-473, docs/adr/ADR-0097-*.md:147-157, docs/runbooks/step-logs.md:62-69. Tests: T041-5 at docs/designs/041-step-logs.md:2519-2524; T041-37 at :3045-3055.
  - The pooler case is below as NF-1.
- **N-1 (the slot key reads `application_name`): resolved.** The exception is now stated and argued: it limits load, not trusted code (docs/designs/041-step-logs.md:1377-1383, data-model.md:269-275, T041-5 :2559-2561).
- **N-2 (32-bit keys): resolved.** The keys are now single 64-bit `hashtextextended(…, 0)` values, with a test (data-model.md:259-262, docs/designs/041-step-logs.md:1364-1366 and :2524).
- **N-3 (`failed` missing): resolved** (data-model.md:19, :487-489).
- **N-4 (`ceiling = tenant_quota` visible across namespaces): resolved, but not recorded.** Reading any stream already needs the PII reveal grant in every namespace (D041-077, docs/designs/041-step-logs.md:332). So anyone who sees `ceiling = tenant_quota` also meets D041-133's condition for seeing the usage. resolution.md's "Second round" does not say this (see Nit 4).

## New findings

**NF-1, Major, fix: the connection bound does not hold behind a transaction pooler**

Where: docs/designs/041-step-logs.md:1366-1373, :2495-2498 and :2055; data-model.md:251-254; ADR-0097:154-157; docs/runbooks/step-logs.md:66-68 and :82-87.

This is verified in the documents. The PgBouncer behaviour in (a) is from my knowledge of PgBouncer and needs checking against the pinned version.

1. **(a) How Q reaches the database is unspecified behind a pooler.**
   - Q is a session setting the client pool sets once, at connect. In transaction mode, a server backend serves clients from every replica.
   - Sent as the startup `options` parameter, PgBouncer either refuses the connection or drops the setting (with `ignore_startup_parameters`). Dropped means Q = 1: safe, but more `service_busy`.
   - Sent as a `SET` after connect, the value stays on whichever server backend ran it. As far as I know, `track_extra_parameters` cannot track a custom setting that PostgreSQL does not report.
   - So with replicas of different pool sizes, a small replica (N = 8, cap 4) can run with a larger replica's Q, say 50. Its waits can then take all 8 of its connections. That is S-1 again.
   - `application_name` is tracked by PgBouncer, so the per-replica slot keys themselves survive.
2. **(b) The free-connection floor protects each replica's client pool, not the server backends they all share.**
   - In T041-37's own setup (docs/designs/041-step-logs.md:3010), 8 replicas share 40 server backends.
   - One tenant's waits can hold 8 × P = 24 of the 40, and three tenants all 40. On top of that come 8 × 10 Logs-cap batches.
   - Every replica's heartbeats and task-lease calls then queue in PgBouncer behind waiters that each hold up to b per lock, with the statement timeout (5 s) as the hard bound.
   - The three-tenant regime (:3052-3055) is not specified behind the pooler, so nothing measures this case.

What an attacker or a noisy tenant gets: two or three tenants with contended usage rows delay every tenant's heartbeats and task leases on every replica sharing the pooler. That is the cross-tenant exhaustion S-1 was about, moved to the server tier.

Smallest fix:
- State the floor as per client pool. Add a pooler sizing rule, for example "server pool ≥ Σ over replicas (⌊N/2⌋ + Q) + a reserve", or derive Q from the server pool.
- Pass Q per transaction: `set_config('sagat.step_log_wait_slots', $q, true)` in the `BEGIN` the report, batch and marker transactions now send themselves. Or state that replicas behind one pooler must have the same pool size, and have T041-5 check Q on a pooled connection.
- Run the three-tenant regime behind the pooler as well.

## Nits

1. **Small pools have no reserve.** The floor formula ignores the cap's "at least 1". At N = 2 the floor is 0, and at N = 1 it is −1. State a minimum pool size (N ≥ 4 leaves 1) or check it at startup (docs/designs/041-step-logs.md:1371-1373).
2. **The three-tenant regime's induced busy answers are counted.** Those of A1 to A3 are not excluded from the 0.1% rule (only tenant A of the two-tenant regime is, :3064-3067). B's own busy share there is not judged either, yet it is the cross-tenant loss: B's waits answer busy at once while others hold Q. Exclude A1 to A3 and add a criterion for B (:3052-3055, :3064-3067).
3. **Suspicion: one wait may take no slot.** When the usage row does not exist, the NOWAIT select finds nothing, so no slot is taken. The `INSERT … ON CONFLICT` can then wait on a concurrent inserter. That wait is bounded by b and happens about once per tenant, but it contradicts "every wait holds a slot". Take the slots before the insert path too, or state the exception (data-model.md:286-292).
4. **The N-4 resolution is not recorded.** Note the by-construction answer above in resolution.md's "Second round".

Counts: Blocker 0, Major 1, Nit 4