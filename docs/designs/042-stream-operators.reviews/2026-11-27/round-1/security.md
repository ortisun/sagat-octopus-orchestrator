# Security review of design 042, third revision, round 1 (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 531af9fb03386ad8f1deaed27933872f35156504
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 1, Major 5, Nit 7

Scope: I reviewed `docs/designs/042-stream-operators.md` at 531af9fb as a design, concentrating on slices 2 to 5. I checked its claims against `crates/scheduler/src/events.rs`, `crates/application/src/{dispatch,event_payloads,run_input}.rs`, `crates/infra-postgres/src/{dispatch,events,instances,quotas,content_usage}.rs`, `crates/infra-ir/src/evaluator.rs`, `crates/domain/src/identity.rs`, and the events, durable-quotas and instance-registry migrations. I read the earlier review files only afterwards, and nothing below repeats a finding they raised.

1. Blocker — The apply takes the chain row `FOR SHARE` and then updates `key_count`/`mark_count` on that same row, so concurrent dispatchers deadlock
   Where: design lines 648 (`operator_chains` holds `key_count`, `mark_count`), 688-690 (step 2 `FOR SHARE`, step 3 "inserting it first when new under `key_count`"), 276-278 (`maxMarks`), 475-479 (CUJ 5); `crates/application/src/dispatch.rs:297` ("Several dispatchers share the inbox"); `crates/scheduler/src/events.rs:226-228` (each replica dispatches events one at a time).
   What: verified in code: dispatch is serial within a replica and concurrent across replicas. Scenario: two replicas each apply an event with a new key, or a new `distinct` mark, on the same chain. Each holds `FOR SHARE` on the chain row, and each then issues `UPDATE operator_chains SET key_count = key_count + 1`. That update needs a lock that conflicts with the other transaction's share lock, so PostgreSQL detects a deadlock after `deadlock_timeout` and aborts one of them. The aborted event goes down the counted retry path and can end `DEAD`. During each wait that replica's serial loop dispatches nothing for any tenant. A key flood (CUJ 5, SC-042-006) or any busy `distinct` chain makes this continuous, so one tenant stalls dispatch for all tenants. That is the opposite of CUJ 5's "other tenants' events are still dispatched". The other reading, a `count(*)` over `operator_keys` with no serialization, does not deadlock but lets `maxKeys` and `maxMarks` overshoot (a quota bypass).
   Why: constitution V (quota bypass is release-blocking), CUJ 5 and SC-042-006, G042-005.
   Correction: keep the counters off the row that is share-locked. Either reserve a key or mark slot with a conditional `UPDATE … WHERE key_count < max` on a separate counter row (or under the per-scope advisory lock of `crates/infra-postgres/src/quotas.rs:30-47`), taken before the chain row and registered in the lock order. Or take the chain row `FOR NO KEY UPDATE` only on the new-key and new-mark path, and accept serialization per chain. Add a two-replica barrier test.

2. Major — Waiting for the transaction horizon has no upper bound, and the lag metric and alert deliberately exclude it
   Where: design lines 224-228 (FR-042-017), 730-732 (lag "neither held by the horizon nor by the backlog rule"), 805-807, 878 (risk table: "the pass's lag metric and alert show it"); `docs/adr/ADR-0153-…md:52-53` ("the pass's lag shows it") and 65 (xids are cluster-wide).
   What: verified in the docs. `pg_snapshot_xmin` covers every write transaction in the cluster, including other tenants' and other databases'. One long-lived write transaction (a slow operator job, a stuck start "waiting on a buffer" per D042-162, another application on a shared server) holds every tenant's due windows indefinitely. `maxWaitMs` then bounds nothing, and held personal data stays held. Section 6 removes exactly these windows from `scheduler_lag_seconds`, and no alert is defined on `operator_seal_horizon_age_seconds`. So the mitigation the risk table and ADR-0153 cite does not fire.
   Why: constitution VI (critical path observable), cross-tenant isolation in V, consistency of the ADR with the design.
   Correction: cap the horizon wait, for example seal anyway at `D + maxHoldMs` the way the backlog rule does. This is safe because a late element is never lost (FR-042-017). Ship an alert on `operator_seal_horizon_age_seconds` with slice 3, and fix the risk table and the ADR's Consequences text.

3. Major — The delivery retry operation is under-specified: which outcomes it accepts, its permission, how it identifies a delivery, and its not-found answer
   Where: design lines 270-276 (FR-042-021), 207-208, 288-289 (FR-042-023 says only "flush and retry"), 797-798 ("a retry of one delivery", no path), 1049-1052 (T042-18); `crates/infra-postgres/migrations/20261012000000_events.up.sql:64,67,94-95`; `crates/scheduler/src/events.rs:327-358`.
   What: verified in the design and code.
   - (a) Nothing restricts the retry to `FAILED` deliveries. Retrying a `HELD` or `SUPPRESSED` delivery after its window started re-applies the event into a new window, which gives a second run from one event and breaks FR-042-014's "at most once".
   - (b) Returning a `DEAD` or `DISPATCHED` event to `PENDING` offers it to every trigger and wait again. A trigger registered after the original dispatch has no delivery row, so it starts a run of another workflow from an old event. That run is attributed to the engine, not to the caller.
   - (c) The delivery rows cascade with the inbox row, so "purged" cannot be told apart from "never existed" or "another tenant's". `409 DELIVERY_EVENT_PURGED` therefore becomes the answer for a missing or foreign resource instead of the uniform 404 and probe audit.
   - (d) The update must also clear `dispatched_at` (the `inbox_events_dispatched_consistent` constraint).
   - (e) No permission is classified for this operation: D042-032 covers the window `:retry` only.
   Why: constitution V (indistinguishable 404, audited attempt), every operation classified, FR-042-014.
   Correction: accept only `FAILED` deliveries of the named trigger, addressed as namespace + source + event id + trigger id. Require `run:start` + `run:operate`. Answer 404 with the probe audit when no inbox row exists in scope, and drop `DELIVERY_EVENT_PURGED`. Re-dispatch the event to that trigger alone: a targeted apply, not `PENDING` for every consumer. Clear `dispatched_at`.

4. Major — Key rows, which hold the key text in plain (personal data), are never purged for a chain without `skip` or `take`
   Where: design lines 212-213 (the epoch ends after `keyIdleMs`, "required with `skip` or `take`"), 282-283 and 649 (purge only when "its epoch has ended"), 457-458 (CUJ 3 declares no `keyIdleMs`), 417 (`maxKeys` 10,000).
   What: verified in the design. In a `debounce` or `window` chain with no `skip` or `take`, the epoch never ends, so no key row is ever purged. CUJ 3 ("one run per burst per order") then holds every order number in plain text for good. After 10,000 distinct orders, every new order fails `KEY_LIMIT` permanently.
   Why: FR-042-024 (held personal data deleted when its use ends), feature 016, CUJ 3.
   Correction: purge a key row once it has no open or sealed window and no live epoch. Make `keyIdleMs` apply to every keyed chain with a default, for example `maxWaitMs`. Consider storing no key text at all and showing only `key_id`.

5. Major — Key text is revealed with no reveal request and no reveal audit
   Where: design lines 294-297 (FR-042-024), 1105-1108 (T042-39); `crates/application/src/event_payloads.rs:22-25,325,386-392`; `crates/domain/src/identity.rs:314-315`.
   What: verified in code. Today an event read returns structure unless the caller explicitly asks for `reveal=pii` while holding the namespace grant, and every such reveal or refusal writes `PayloadPiiRevealed` or `PayloadRevealDenied`. The design only says the API "shows the key text only to a caller who may reveal personal data". Read as written, any grant holder listing a chain's windows receives up to 10,000 key texts per chain with no request and no audit record.
   Why: constitution VI (secret and personal-data access audited), feature 016's reveal model.
   Correction: return key text only on an explicit reveal parameter, through the same projector and the same two audit actions as event reads, and add a test for each.

6. Major — The capability removal guard can be raced by a concurrent publication
   Where: design lines 830-836 ("Publication reads the requirement without a lock and the guard is the registry's removal check"); `crates/infra-postgres/src/instances.rs:241-256`; `crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql:404` (`sagat_app` has only `SELECT` on the epoch).
   What: verified in code. `remove` takes the epoch lock and runs `verify` under READ COMMITTED, and publication does not take that lock. Sequence: a publication reads the requirement as present and inserts its `operator_chains` row, uncommitted. The operator's `remove` then finds no chain in `verify`, deletes the requirement and commits. Then the publication commits. The result is an active chain with no requirement, so a process without `trigger-operators-v1` can register and serve it. That is the over-delivery the requirement exists to prevent.
   Why: check-then-act outside the lock; the stated guard (FR-042-003, section 10) does not hold.
   Correction: have publication and `switch` insert or activate the chain first and re-read the requirement in a later statement, refusing if it is gone. Have `verify` run `LOCK TABLE operator_chains IN SHARE MODE` (and the same for chained versions) before it reads. Add a held-lock test for both orders.

7. Nit — Flush and discard on an `OPEN` window have no lock order, and the start does not require that its update hit a row
   Where: design lines 233-236, 667-670, 715-720, 288-289.
   What: suspicion. The design says how the pass's seal is ordered against an apply (the key lock), but not how flush (`OPEN`→`SEALED`) or discard (`OPEN`→`DISCARDED`) are. An apply racing them could add a member to a sealed window (its `HELD` element never reaches a run) or write held values after the discard deleted them. The start's `UPDATE … WHERE state = 'SEALED'` is not stated to roll back the created run when it matches zero rows. The window table has no column recording who flushed or retried, although FR-042-023 requires recording that caller as the starter.
   Correction: flush and discard lock chain, key and window rows in the seal's order and re-check `OPEN`. The apply adds members only to a window it has locked in state `OPEN`. The start rolls back when its update affects zero rows. Add a `requested_by` column read by the start.

8. Nit — The definer function's search path does not follow the codebase's pattern, and the HMAC's threat model is not stated
   Where: design lines 654-660, 932-935; every definer function in the migrations pins `pg_catalog, pg_temp` with schema-qualified names (for example `20270113000000_instance_registry.up.sql:183,197`). No migration revokes `TEMPORARY` from PUBLIC.
   What: verified. "`pg_catalog` and the schema" leaves `pg_temp` unlisted, and an unlisted `pg_temp` is searched first for tables. `sagat_app` could create a temporary `operator_hmac_keys` and have the function compute with a key it chose. The impact is limited to a session that already holds `sagat_app`.
   - `CREATE EXTENSION IF NOT EXISTS pgcrypto` does nothing if pgcrypto already lives in another schema, and the pinned path then cannot find `hmac`.
   - Key text is stored in plain text, so the HMAC protects only `distinct` values, and only from a reader without `operator_hmac_keys`.
   - The design does not say what the function does for a tenant with no key; it should fail closed.
   Correction: use `SET search_path = pg_catalog, pg_temp`, schema-qualify both the table and `hmac`, install the extension `WITH SCHEMA` explicitly, raise an error when the key row is missing, and state the threat model.

9. Nit — Trigger stages re-compile and re-parse the event document on every call, on the shared dispatcher
   Where: design lines 497-498 (`trigger_stage(stage, element, event)`); `crates/infra-ir/src/evaluator.rs:462-481`; `crates/scheduler/src/events.rs:334`.
   What: verified. Today's `event_filter` compiles the expression and parses the document (up to 240 KiB) on each call, and runs on the async runtime. A chain adds up to ten such calls per trigger per event (key, eight stages, `flatMap`), across up to 50 chains per tenant, on a queue every tenant shares. The design solved this exact cost for collections (FR-042-006) but not here.
   Correction: one evaluator call per chain that parses the document once and caches compiled expressions per chain digest.

10. Nit — "The tenant's event retention" is not a retention period
    Where: design lines 255, 283-284; `crates/infra-postgres/src/content_usage.rs:590-613`; `20261024000000_durable_quotas.up.sql:14-18,26`.
    What: verified. `event_retention_seconds` is a per-namespace ceiling on the deduplication TTL an event may request (0 is allowed). So the lifetime of a `START_EXPIRED` window's held values and of finished window rows is undefined.
    Correction: name a concrete bound, for example 7 days after `REFUSED`, with a purge test.

11. Nit — Removing a trigger discards only its `OPEN` windows
    Where: design lines 284-286.
    What: `SEALED` windows, including `flatMap` fan-outs still in progress, keep starting runs for up to 24 hours after a trigger is removed, which is often done as an incident response.
    Correction: on removal, mark `SEALED` windows `REFUSED` (or `DISCARDED`) with the held values deleted, or state the behavior and audit it.

12. Nit — How three limits are enforced is not stated
    Where: design lines 271 (the `KEY_LIMIT` audit "collapsed per chain and minute"), 278-281 (`operator_held_bytes`, the tenant sums).
    What: suspicion. The design does not say how these are enforced. A per-tenant usage row updated by every apply serializes the whole tenant. An unlocked check overshoots. An in-memory collapse of audit counts loses them on a crash.
    Correction: name the mechanism for each, such as the advisory-lock pattern of `crates/infra-postgres/src/quotas.rs` and a durable per-minute counter row.

13. Nit — Stale code citations
    Where: design line 115 (the claim is at `crates/infra-postgres/src/dispatch.rs:155-161`; line 227 is `finish_event`), lines 111-113 (`parse_stored` is at `crates/infra-ir/src/lib.rs:172`; `published_collection_definition` is at `crates/infra-ir/src/convert.rs:757`).
    Correction: update the line references.

Classes I checked and found nothing in:
- **Metric labels:** all closed, with no tenant or namespace label.
- **Cross-tenant 404 on the chain and window resources:** handled; only the delivery retry has a problem (finding 3).
- **Secret values:** no new path reaches them; trigger stages see what filters see today (`dispatch.rs:154-170`).
- **Grants:** `sagat_app` already holds `DELETE` on `event_deliveries` and `UPDATE` on `inbox_events`, so the retry needs no new grant.
- **Injection, regular expressions, SSRF and egress:** none is introduced.
- **JSON depth and size:** held elements are bounded by the 240 KiB inbox payload check and the window byte ceiling.
