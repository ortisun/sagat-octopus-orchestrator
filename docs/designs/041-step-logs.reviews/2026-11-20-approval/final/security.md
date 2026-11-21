Doc: docs/designs/041-step-logs.md
Reviewed-commit: 543417134ff7ebda70a96690329bb95ddde55c8c
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811

**Verdict.** NF-1 is resolved, and so are Nits 1 to 4. The fixes bring no new Blocker and no new Major. Three new Nits remain, all about the pooler case. I read the documents at the reviewed commit, which is the head of `feature/041-step-logs-server`, and ran nothing.

I checked these classes and found nothing new in them:

- **Tenant boundary:** the slot keys are still built inside the function from the principal's tenant. *Q* is a value per replica that the store computes from its pool; nothing in a request sets it.
- **Coupling between tenants:** the replica cap couples tenants only through load. The signal is coarse: no tenant's identity or resource can be reached through it.
- **Authorization:** unchanged.
- **Secrets:** unchanged. The top-up's metric label set is still closed, now without `held`.
- **Injection:** none. `p_wait_slots::text` is cast from an integer and clamped where it is read.
- **Runtime-role privileges:** the carry and the wait-slots helper stay `SECURITY INVOKER`.
- **The `SET search_path` clause on the carry:** it does not undo the transaction-local `set_config` of a different setting. In any case the refill reads that setting inside the carry's own call.
- **A NULL or empty value:** either one reads as 1, which fails safe.

## Earlier findings

- **NF-1 (the connection bound behind a transaction pooler): resolved.**
  - (a) *Q* is now passed with each transaction and is never set at connect:
    - docs/designs/041-step-logs.md:1404-1411, :941 (the batch's T1) and :1982 (the marker);
    - specs/041-step-logs/data-model.md:252-256, :623-624 and :651-653;
    - ADR-0097:161-163.
    - Tests: :2606-2607 and :3044-3045. Through the pooler it is checked at :3142-3143.
  - (b) The floor is now stated per client pool (:1414-1418). The server-pool sizing rule is at :1422-1425, ADR-0097:166-167 and docs/runbooks/step-logs.md:95-100.
  - T041-37 sizes PgBouncer by that rule (:3129-3130). It also runs the three-tenant regime behind the pooler (:3180-3186).
  - The startup `options` problem is handled at :3138-3141 and in the runbook at :98-100.
- **Nit 1 (no reserve for small pools): resolved by a statement.**
  - "At N of 2 or less nothing is left" is at :1418. The runbook advises a pool of at least 3 (docs/runbooks/step-logs.md:70-72). The arithmetic holds: 3 − 1 − 1 = 1.
- **Nit 2 (A1 to A3 counted, B not judged): resolved.**
  - A1 to A3 are excluded at :3197-3201, and B's busy share is bounded at 0.1% (:3186).
  - When that criterion is missed, the consequence is at :3210.
- **Nit 3 (a wait that takes no slot): resolved.**
  - An absent row now takes the slots before its insert (:1388-1391, data-model.md:261-264 and :296, ADR-0097:155-157). The test is at :2607-2608.
- **Nit 4 (N-4 not recorded): resolved.**
  - It is now recorded in docs/designs/041-step-logs.reviews/2026-11-20-approval/resolution.md:331-332.

## New findings

**1. Nit, fix: the pooler sizing rule leaves out the pool's other clients, and its reserve is thin.** (docs/designs/041-step-logs.md:1422-1425, ADR-0097:166-167, docs/runbooks/step-logs.md:95-98)

- *Scenario:* three tenants with contended usage rows can push step-log waits and Logs calls up to 120 of the 128 server backends. That leaves 8 backends for the whole fleet's heartbeats, task leases and reports that do not wait. With direct connections the reserve would be 40 (5 per replica).
- The control-plane replicas, the registry pool and the one-shot processes also use that PgBouncer pool, but the sum does not count them, so their load eats into those 8.
- The holds are short (at most *b* per wait), so the delay is bounded. T041-37 measures it and has a consequence if it fails.
- *Fix (documents only):*
  - say that every other client of the same pooler pool is added on top of the sum;
  - state why one backend per replica is enough, or let T041-37's result set the reserve.

**2. Nit, fix: behind the pooler, T041-37 does not measure B's heartbeats and task leases directly.** (:3184-3185)

- PgBouncer's `maxwait` is a figure for the whole pool (database and user), which all tenants share. It is the age of the oldest waiting client at the moment of the sample. Sampled once a second, it neither isolates B nor catches waits that fall between samples.
- "No call of B failing to get a connection" checks sqlx's client pool. Behind the pooler nothing queues there; the queueing happens in PgBouncer.
- *Fix:*
  - behind the pooler, judge the p99 latency of B's heartbeat and task-lease calls, measured per tenant by the driver, against its solo figure;
  - call `maxwait` the pool's, not B's.

**3. Nit, accept: the statement-timeout hard bound fails open behind a pooler that drops `options`.** (ADR-0097:170, docs/runbooks/step-logs.md:98-100)

- If an operator lists `options` in `ignore_startup_parameters` and does not run `ALTER ROLE … SET`, the statement and idle timeouts are silently missing in production. Only the benchmark checks for them (:3141).
- The S-1 bound does not depend on these timeouts: each wait on the usage row is still capped at *b* by the function's own `lock_timeout`. This is existing platform behaviour.
- *Optional:* make the role-level timeouts a MUST in docs/runbooks/scaling.md, or check `SHOW statement_timeout` through the pooler at startup.

Counts: Blocker 0, Major 0, Nit 3