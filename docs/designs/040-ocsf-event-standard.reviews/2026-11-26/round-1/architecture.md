# Architecture review of design 040, round 1 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: 9b81a0ca3064ccb26ffda28162fc177eb2c2e1cb
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 1, Major 9, Nit 15

1. Blocker — The OCSF encoder port cannot be implemented in `crates/telemetry`
   - Where: Technical Plan section 2 (lines 307-311); Constitution Check II (line 487); slice 1 (line 525).
   - What is wrong: `crates/application` would define the port `OcsfEncoder`, and its one implementation would live in `crates/telemetry/src/ocsf.rs`. To implement that trait, telemetry has to depend on application. But `crates/application/Cargo.toml:22` already depends on `sagat-telemetry`, so this is a dependency cycle, and Cargo refuses it. `scripts/check-architecture.py:90,97` also classes telemetry as `support`, which may depend only on `domain` and `support`. The port is also needless: application already depends on `serde_json` (Cargo.toml:24). A port with a single implementation, mapping `OcsfEvent` one-to-one onto JSON, is the kind of DTO mapper with no independent meaning that Principle II forbids.
   - Why it matters: the design cannot be built as written, and the Constitution Check marks II "Pass" over it.
   - Correction: drop the port. Either `OcsfEvent` derives `Serialize` in `crates/application`, or the encoder goes in an adapter crate (for example a new `crates/infra-ocsf`) that depends on application. Keep the vendored schema and the validation test beside the encoder. Fix section 2, slice 1's crate list and Constitution Check II.

2. Major — The export pass runs "on each scheduler tick", so a slow collector holds up timers and liveness
   - Where: section 5, "The pass" (lines 399-401).
   - What is wrong: `run_scheduler` (`apps/execution-engine/src/loops.rs:242-263`) calls `health.ticked()` only after a tick returns, and `/livez` fails when ticks stop (ADR-0020 decision 7). One pass can make many sequential HTTP calls with a 10-second timeout each. Put inside the tick, that delays every timer, retry and deadline on the replica and can fail liveness because a customer's SIEM is slow. Every other outbound pass has its own loop (`run_deliveries`, `run_schedules`, `run_batches`).
   - Correction: give the pass its own loop (`run_audit_export`) with its own poll interval, concurrency and `while_serving` gate. State the poll interval and say that the scheduler tick never waits on it.

3. Major — A 30-second claim is shorter than one claim's worst-case work, and nothing renews it
   - Where: section 5 steps 1 and 5 (lines 403-404, 424-427); Success criterion 040-006.
   - What is wrong: one claim can send a 500-record batch and then halve it down to one record. That is about 19 sequential requests, each with a 10-second timeout, so well over 30 seconds. The design does not say that a sink is due only when its claim has expired, nor that the claim is renewed during the work. Once it lapses, a second replica claims the same sink and sends the same range at the same time. The receiver then sees records out of order, against the ordering promise of Requirement 040-009. The first holder's commit then fails on `claim_token`, so its accepted halves are sent yet again. The design also does not say whether the cursor is committed after each accepted half.
   - Correction: make the due predicate include `claim_expires_at < now()`. Renew the claim before every request (and fail the work if the renewal fails), or bound one claim's work below its expiry. Commit the cursor after each accepted half. Add a Success criterion 040-006 case with a held endpoint that outlives the claim.

4. Major — Destination-level 4xx answers are treated as record-level, so a revoked collector token skips the whole backlog
   - Where: section 5 step 5 (lines 424-427); Requirement 040-009 (lines 122-128).
   - What is wrong: "Any other `4xx` ... is permanent for the batch", then the pass halves down to one record, which is recorded as refused and passed. A `401` or `403` from an expired collector token, or a `404`/`410` for a moved path, refuses every single record. The pass then marks the entire backlog `audit.sink_record_refused` and moves the cursor past all of it. The SIEM silently loses everything until someone notices. Requirement 040-009 promises the opposite: "MUST NOT skip records".
   - Correction:
     - Treat `401`, `403`, `404`, `405`, `410`, and an egress `Denied`, as destination failures: a closed state such as `PAUSED_DESTINATION_REFUSED`, audited and alerted, with no record passed.
     - Halve only on `400`, `413` and `422`.
     - Pass a record only when a sibling half was accepted under the same request shape.
     - Add a test: a `401` for 1,000 records refuses none.

5. Major — The operator sink's "one tenant per message" conflicts with cursor order, and partial success is undefined
   - Where: section 5 step 3 (lines 408-409); Requirements 040-009 and 040-011 (lines 124, 136-140).
   - What is wrong: a batch whose records interleave tenants A, B, A is "split into one message per tenant in cursor order". Two readings, both broken:
     - Grouped by tenant: A's second record leaves before B's, so the order is broken.
     - Contiguous runs: one HTTP request per run, which under interleaved load approaches one request per record, and the operator sink cannot keep up.
     - In both readings, where the single cursor commit goes after message k of n fails is not defined.
   - Correction: decide and write down one of these:
     - Order is promised per tenant only. The batch is split into per-tenant groups, and the cursor moves to the end of the longest fully acknowledged prefix in `(tx_id, id)` order, with a test for message k failing.
     - Or the operator sink keeps one cursor per tenant.

     Then give Non-functional requirement 040-003 a figure for the operator sink under interleaved tenants.

6. Major — After a logical restore, records restored above the new cluster's transaction counter are never streamed
   - Where: section 4, "A cluster whose transaction ids went back" (lines 371-375); ADR-0150 decision 4.
   - What is wrong: `pg_dump`/`pg_restore` copies `tx_id` as data, so restored rows keep the old cluster's high values. `CURSOR_INVALID` repairs the cursor. But every record written after the sink's last delivery and before the dump has a `tx_id` above the new cluster's `xmin`. The condition `tx_id < pg_snapshot_xmin(...)` stays false for those rows until the new counter catches up, which can take billions of transactions. `audit_records` is append-only, so an operator cannot repair the column. The planned restore test (Task 040-13) only checks the state change, so it would pass while exactly these records stay unsent. The same holds for a move by logical replication.
   - Correction: in the restore runbook, advance the new cluster's transaction-id epoch above `max(tx_id)` before the restore (`pg_resetwal --epoch` on the stopped cluster), so that every new `xid8` is higher. At start-up, refuse to run the export while `max(tx_id) >= pg_snapshot_xmax(pg_current_snapshot())`, using the `(tx_id, id)` index. That makes `CURSOR_INVALID` unnecessary. Extend Task 040-13 to assert that a record written after the last delivery and before the dump arrives.

7. Major — An idle sink costs a write and a transaction id on every tick
   - Where: section 4 (line 373, `pg_current_xact_id() < cursor_tx`); section 5 step 1 (lines 401-404).
   - What is wrong:
     - `pg_current_xact_id()` assigns a transaction id to the transaction that calls it, and the check runs "before each batch".
     - The claim is an `UPDATE` that sets a token.
     - A due sink is one with `next_attempt_at` passed, and nothing moves `next_attempt_at` forward after an empty pass. So every `ACTIVE` sink is claimed, written and given a transaction id on every tick, whether or not it has records. Example: 1,000 tenants × 4 sinks at a one-second tick is about 345 million transaction ids a day. That forces anti-wraparound vacuums and adds dead tuples to `audit_sinks`. The audit table carries that cost no matter how few events there are.
   - Correction:
     - Detect "the counter went back" with `cursor_tx >= pg_snapshot_xmax(pg_current_snapshot())`, which assigns no transaction id.
     - Before claiming, run a read-only `EXISTS` for a record below `xmin` above the cursor.
     - After an empty pass, set `next_attempt_at = now() + idle_poll`.
     - State the idle poll, and add a budget test: transaction ids consumed by N idle sinks over M ticks.

8. Major — Sealing the sink credentials is unspecified and conflicts with ADR-0152, accepted the same day
   - Where: Requirement 040-008 (lines 117-119); Requirement 040-011 (lines 136-138); section 5 (lines 379-385).
   - What is wrong:
     - "Sealed with the platform's key authority (design 015)" names a design that is still Draft (0 of 22 tasks). The seal that exists today is `SecretCipher` (`crates/domain/src/secrets.rs:158`, the local key ring).
     - Under `openbao`, ADR-0152 requires a seal under a tenant-derived data key, covered by the rewrap pass, and decision 1 forbids any platform key outside OpenBao. The design gives no AAD (tenant, sink, column). It does not put the sink columns into 015's rewrap and re-seal inventory. It says nothing about what the pass does on `CipherError::Open` or `KEY_AUTHORITY_UNAVAILABLE`.
     - It keeps the operator sink's signing key in engine configuration, which decision 1 forbids under `openbao`.
     - Design 040 depends on design 015, and no owner gate or dependency records it.
   - Correction:
     - Seal through `SecretCipher` with AAD = `tenant_id || sink_id || column`.
     - Add the sink's sealed columns to design 015's rewrap and re-seal inventory, through an amendment and an `Owner-gate:` dependency.
     - Define a closed state for "key authority unavailable" (retry, not skip).
     - Under `openbao`, take the operator sink's signing key from a Transit key or the `secret/sagat/credentials` custody, or record a refinement of ADR-0152 decision 1.

9. Major — The push to sinks has no versioned wire contract
   - Where: section 5 step 3 (lines 409-417); Task 040-11 (lines 563-567); Constitution Check IV (line 488).
   - What is wrong: third-party receivers must verify `x-octopus-signature`, whose signed string starts `octopus-audit-export-v1\n{ts}\n{eventId}\nocsf.batch\n\n\n` and is followed by the gzip bytes. They also need the NDJSON body, `Content-Encoding`, the meaning of `x-octopus-event-id` and `x-octopus-delivery-id` (both new on every attempt), deduplication by `metadata.uid`, and how each status code is treated. Webhook deliveries specify all of this in `specs/001-platform-foundation/contracts/events.asyncapi.yaml:777`. The design adds no AsyncAPI channel, no baseline and no contract test; the test vectors exist only inside `signing.rs`. Principle IV requires wire contracts to be versioned, and the CI contract gates cannot see a change to this one.
   - Correction: add an audit-export channel to the AsyncAPI document (or `specs/040-ocsf-event-standard/contracts/`) with every header, the signed string, the body format and the status semantics. Add its baseline and compatibility test. Publish the known vectors there, and make the slice-2 task read them.

10. Major — Delivery Constraints marked "Pass" while the region-loss tombstone is declined
    - Where: Gap 040-005 (lines 225-226); Constitution Check, Delivery Constraints (line 493).
    - What is wrong: in a mode that can lose acknowledged work, the constitution's Delivery Constraints require that "downstream consumers receive a tombstone for what the loss undid". The design says "no tombstone is sent". It leaves "the failover report lists the window" to feature 032, with no task here and no dependency there. A SIEM fed with events that the surviving timeline never had cannot tell them apart. The integrity of the security stream is the purpose of this feature, so declining the tombstone is a security exception, and only the owner can grant one.
    - Correction: either carry the region epoch in every event (`unmapped.sagat.region_epoch`) and, after an activation, send a tombstone message per sink covering the positions above the surviving cursor; or record the deviation as an owner row (not a delegate one) and mark the check "Pass with conditions". Either way, add the task, or the feature 032 dependency, for the failover-report entry.

11. Nit — ADR-0150, the research register and the owner rows are stale or mis-cited
    - `docs/adr/ADR-0150-…md:7-12,53`:
      - It "refines ADR-0008 (observability and audit)", but ADR-0008 is "Runtime graph as a journal projection".
      - Its Enforced by names a "compile-fail test (Task 040-3)" that the design dropped in favour of the clippy lint.
      - Its Consequences still say "the lag shows it".
      - It omits the column-grant test (Task 040-15), the restore test and the state-race tests.
    - `specs/040-ocsf-event-standard/research.md:15` (D040-006: "never dead-letters ... a cursor never becomes invalid") is contradicted by the design, and D040-011 does not name it as amended.
    - `docs/owner-decisions.md:17` (Owner decision 2026-11-26-103) keeps the wrong reason "a declared profile makes its attributes required", and line 21 (Owner decision 2026-11-26-107) keeps "more than 300 places".
    - Correction: amend each to match the design, and name the lint, the grant test and the restore test in Enforced by.

12. Nit — There is no "role migration" for permissions
    - Where: section 6 (lines 434-435); Task 040-9 (line 557).
    - What is wrong: role permissions are code, `Role::ADMIN_PERMISSIONS` (`crates/domain/src/identity.rs:546-601`). The database only checks role names (`memberships_role_valid`).
    - Correction: add `Permission::AuditSinkManage` in the domain, add it to the admin list, and move it out of the migration task.

13. Nit — "Counted under a lock of the tenant's row" is not available to the runtime role
    - Where: Requirement 040-008 (line 121); Task 040-10 (line 561).
    - What is wrong: `sagat_app` has only `SELECT` on `tenants` (`20261008000200_runtime_role_grants.up.sql:19`). Any `FOR UPDATE` or `FOR SHARE` needs `UPDATE` privilege, so the lock fails as the runtime role.
    - Correction: use `pg_advisory_xact_lock` keyed by the tenant, or a `slot smallint CHECK (slot BETWEEN 1 AND 4)` column with `UNIQUE (tenant_id, slot)`.

14. Nit — The new alert rule tests would not run in CI
    - Where: Task 040-16 (line 580).
    - What is wrong: `.github/workflows/ci.yml:690` runs `promtool test rules` on `batch_stalled_test.yaml` only. This is the same gap ADR-0082's amendment records.
    - Correction: add the new test files to that step, or glob `/tests/*_test.yaml`, in the same change.

15. Nit — `postgres_oldest_xid_holder_seconds` cannot see other roles' backends
    - Where: section 6 (line 450).
    - What is wrong: without `pg_read_all_stats`, `pg_stat_activity` hides `xact_start` and `backend_xid` for sessions of other roles, which are exactly the sessions the risk names. Design 041 recorded the same gap (Gap 041-016).
    - Correction: state the runbook grant, or report the gauge as "unknown" when it is blind.

16. Nit — The state table has no action, reason or state for several transitions
    - Where: lines 389-397, 405-407, 443-445.
    - What is wrong: there is no state for an egress refusal found by the check of step 2. Nothing says which action and reason record `STALLED`, `CURSOR_INVALID` and the return to `ACTIVE` after `STALLED`. Nothing says that `audit.sink_record_refused` commits in the same transaction as the cursor moving past the record.
    - Correction: give one row per transition with its action and closed reason, and state that atomicity.

17. Nit — The route style differs from the repository's
    - Where: section 6 (lines 435-439).
    - What is wrong: `{sinkId}:pause` and the other `:verb` paths do not match the existing contract, which uses sub-paths (`openapi.yaml:4224`, `/schedules/{scheduleId}/pause`; `:4252`).
    - Correction: use `/audit/sinks/{sinkId}/pause`, `/resume`, `/rotate-signing-key` and `/test`.

18. Nit — The form of the reveal-grant check is ambiguous
    - Where: Requirement 040-010 (lines 131-134).
    - What is wrong: `has_effective_payload_reveal_grant` (`reveal_grants.rs:40-67`) takes a shared fence per namespace. The step-log rule (`reveal_grants.rs:84-109`) is a single fence-free `SELECT`. Rechecking with the first form "in every namespace" at commit holds N namespace fences in the cursor transaction, and those block revocations.
    - Correction: name the single-snapshot form, and say how the commit recheck orders against a revocation.

19. Nit — The migration fallback and recovery are not workable as written
    - Where: section 4 (lines 339-341).
    - What is wrong:
      - The fallback "checked start-up step" cannot build the indexes as `sagat_app`, which owns nothing.
      - A failed concurrent build leaves the sqlx migration dirty, and no recovery step is given.
      - The two `ALTER TABLE` statements set no `lock_timeout`, so they can queue behind a long transaction and block every audited write.
    - Correction: make the fallback an operator command run as the owner, write a dirty-migration recovery step in the runbook, and set `lock_timeout` on the `ALTER` statements.

20. Nit — OCSF codes in the domain, and one status type for two OCSF enums
    - Where: section 1 (lines 292-303).
    - What is wrong: the `match` would be just as total in the adapter, yet the design puts the version-specific numeric codes of an external schema into `crates/domain`. `OcsfStatus` must also hold both the event `status_id` and the finding lifecycle (New), which are different OCSF enums.
    - Correction: either justify the domain placement as security classification, or move the codes beside the encoder. Split the status type per class.

21. Nit — `startFrom` scans a tenant's whole history inside a request
    - Where: lines 366-369.
    - What is wrong: `min(tx_id)` over `created_at >= t` reads every record of the tenant since `t` through `audit_records_tenant_created_idx`, plus heap fetches.
    - Correction: bound `t` (for example 90 days), or resolve it with a statement timeout and a typed error.

22. Nit — Slice boundaries and the appendix's abbreviations
    - Where: lines 525-527 and 614-618.
    - What is wrong: slice 2's crate list omits `crates/api-http`, `crates/domain` and `crates/application`, which it needs for the sink API, the new variants and the service. The seven new actions must land in slice 1 for "134 of 134" (Success criterion 040-001) to hold there. The appendix legend's UM, EM, DS and DF go against the constitution's "Words in full".
    - Correction: list the crates per slice, put the seven actions in slice 1, and write the class names out.

23. Nit — The signing key in create and rotate responses
    - Where: lines 119-120 and 436.
    - What is wrong: Principle V keeps secret values out of ordinary management responses. One-time issuance has precedent (`leaseToken`, feature 026's form token), but the design gives `Cache-Control: no-store` only for `GET /audit/ocsf`.
    - Correction: state `no-store`, never logged and never in an audit reason on `POST /audit/sinks` and `:rotateSigningKey`, citing the precedent.

24. Nit — The OCSF server image is not placed in the repository's infrastructure
    - Where: Success criterion 040-002 (lines 168-170); Tasks 040-2 and 040-6.
    - What is wrong: AGENTS.md Rule 16 requires a pinned image with loopback-only ports and a healthcheck, declared in the repository. The design does not say where the image is declared, nor that its schema is bundled rather than fetched at run time.
    - Correction: name the compose service or the Testcontainers fixture and the schema it mounts.

25. Nit — Sharding sinks by tenant and an unfenced commit
    - Where: lines 399-401 and 418-421.
    - What is wrong: "the shard of its tenant" is a new use of ADR-0020's `Shard::of` (decision 1 keys shards by run and rejects tenant sharding for run work). The commit is guarded by the claim token, not by the fence that ADR-0020 decision 3 puts in every action's transaction.
    - Correction: say in section 5 that the claim token, not the fence, makes the commit correct, and why tenant keying is acceptable for at most 4 sinks per tenant.

I checked these claims in the code and found them true:
- 127 `AuditAction` values (`crates/domain/src/audit.rs:328-454`).
- The six `INSERT INTO audit_records` statements in four modules, all with explicit column lists, so a column grant keeps older binaries working.
- `WITHHELD_FROM_TENANT` filtered by `operations.rs:44,821`.
- `scrub` at 2,048 characters (`crates/telemetry/src/redact.rs:16`).
- Two signing versions (`crates/scheduler/src/deliveries.rs:35`).
- Namespace-scoped `egress_policies`.
- The role subject in `membership.granted`'s resource id and its role only in the reason (`identity.rs:159-167`).
- Owner decisions 2026-11-26-100 to 107 are present in `docs/owner-decisions.md`.

I found no OCSF or audit-sink code in any sibling worktree, so the work is not duplicated elsewhere. I could not check the OCSF class and activity numbers (such as User Management 3007) offline; the vendored schema of Task 040-2 must confirm them.

Files:
- docs/designs/040-ocsf-event-standard.md
- docs/adr/ADR-0150-security-and-audit-events-leave-as-ocsf-projections-of-the-native-record.md
- docs/adr/ADR-0152-openbao-transit-holds-the-platform-keys-and-each-custody-has-its-own-mount-and-token.md
- specs/040-ocsf-event-standard/research.md
- scripts/check-architecture.py
- crates/application/Cargo.toml
- apps/execution-engine/src/loops.rs
- crates/infra-postgres/migrations/20261008000200_runtime_role_grants.up.sql
- crates/infra-postgres/src/reveal_grants.rs
- crates/domain/src/identity.rs
- specs/001-platform-foundation/contracts/events.asyncapi.yaml
- .github/workflows/ci.yml
