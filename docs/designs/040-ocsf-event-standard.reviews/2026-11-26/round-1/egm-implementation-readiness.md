# EGM stage 9, implementation readiness with codebase details, of design 040 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: 95524886ed183110634887add4c737fca0729501
Reviewer: egm-implementation-readiness
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 13, Nit 9

1. Major — The Datastore Activity `database` object has nothing to be filled from
   - Where: Technical Plan 2, table row "Datastore Activity 6005" (line 329); appendix rows for `database.result_read`, `database.catalog_browsed`, `iceberg.write_committed`, `destination.decided` and `database.denied` (lines 733-737); Task 040-4.
   - What is wrong: the plan says `database.name` is "the connection's resource id". No writer records a connection as the resource of these actions. `database.denied` is written with `ResourceType::Run`, `Workflow`, `Namespace` or `WorkflowVersion` (`crates/application/src/database_results.rs:96`, `definitions.rs:1762`, `runs/ad_hoc.rs:676`, `crates/scheduler/src/events.rs:565`). `database.result_read` and `iceberg.write_committed` are written with `ResourceType::Run` (`worker.rs:1641`, `worker/lake_complete.rs:151`). `database.catalog_browsed` and `destination.decided` have no writer at all. The connection id could only come from the reason, and Requirement 040-005 forbids parsing the reason.
   - Why it matters: the builder of Task 040-4 must either invent a value or put a run id into `database.name`. Success criterion 040-001 needs this object to validate.
   - Correction: choose one rule in the plan. Either remap these five actions to API Activity, or state that `database.name` is the resource id whatever its type, with `type_id` 99. Add a fixture per real resource type.

2. Major — `src_endpoint.svc_name` ("the surface") cannot be derived from the stored record
   - Where: Technical Plan 2 (line 326), Gap 040-001, Task 040-4.
   - What is wrong: API Activity and Datastore Activity require `src_endpoint`. The plan fills it with `rest`, `grpc` or `engine`. Neither `audit_records` nor `AuditEntry` (`crates/application/src/operations.rs:404`) holds the surface. Only `ResourceType::Endpoint` ids carry a `grpc ` prefix.
   - Why it matters: every builder will pick a different heuristic, and the value a SIEM receives is undefined.
   - Correction: write a closed derivation rule in the plan, for example:
     - actor starting `system:` gives `engine`;
     - resource type `endpoint` with an id starting `grpc ` gives `grpc`;
     - anything else gives `rest`, documented as a best effort.
     
     Or use a constant such as `svc_name: "sagat"` and say so in Gap 040-001.

3. Major — Slice 2 waits on an unbuilt task of a Draft design, and the sealing API is contradictory
   - Where: slice 2 header (line 603); Technical Plan 5 "Credentials" (lines 473-479); Task 040-12 ("or the KV path under `transit`").
   - What is wrong: design 015 is Draft with 0 of 25 tasks ticked. Its Task 015-10 (`KeyAuthority`, `seal_with`, `open_with`) does not exist. Section 5 says sealing goes through `SecretCipher`, which already exists with associated data (`crates/domain/src/secrets.rs:158-176`). The slice header says slice 2 needs the port of Task 015-10. The OpenBao Agent KV path for the operator sink needs design 015's `transit` mode, which is also unbuilt.
   - Why it matters: nobody can start slice 2 tomorrow, and the builder does not know which API to call.
   - Correction: decide one of the following.
     - Build slice 2 on today's `SecretCipher::seal`/`open`. Register the four sealed columns in design 015's inventory, and move the `transit` operator-sink source to a follow-up task gated on Task 015-11.
     - Or keep the dependency, mark it explicitly on Tasks 040-9 to 040-12, and accept that slice 2 cannot start.
     
     Also give the exact associated-data encoding. The existing `associated_data` uses a `sagat/secret/v1` label, length prefixes and the key id; the plan's `tenant || sink || column` has none of these.

4. Major — The concurrent-index migration as written will fail
   - Where: Technical Plan 4 (lines 349-353), Task 040-9.
   - What is wrong: sqlx 0.9 honours `-- no-transaction`, so the check that Task 040-9 asks for will pass and the builder will take the migration path. That migration holds two `CREATE INDEX CONCURRENTLY` statements and a `pg_index.indisvalid` check in one file. sqlx sends the file as one simple query with several statements. PostgreSQL runs such a query as an implicit transaction block, so `CREATE INDEX CONCURRENTLY` fails with "cannot run inside a transaction block".
   - Why it matters: the fallback is only taken when the marker is unsupported, so it never triggers. The builder hits a runtime failure the plan does not foresee.
   - Correction: put one statement in each no-transaction migration file (index 1, index 2, then the validity check `DO` block). Or always use the checked start-up step. Note that the repository's precedent (`20261017000000_scheduler_shards.up.sql:69`) is a plain `CREATE INDEX IF NOT EXISTS` that an operator may pre-build concurrently.

5. Major — Success criterion 040-007 contradicts the idle-sink design
   - Where: Technical Plan 5, pass step 1 (lines 429-431); Success criterion 040-007 ("Idle sinks consume no transaction id"); Task 040-14.
   - What is wrong: step 1 moves every idle sink's `next_attempt_at` forward with an `UPDATE` batched per tick. That update takes a transaction id on every tick.
   - Why it matters: the measurement of Task 040-14 fails by design, or the builder drops the update and changes the schedule semantics on their own.
   - Correction: either keep idle back-off in memory with no write, or restate the criterion as "at most one transaction id per tick, independent of N".

6. Major — The `unmapped` content and its shape are undefined
   - Where: Requirement 040-006; Technical Plan 2 ("per action, the closed keys the appendix lists", line 334); appendix; Tasks 040-4 and 040-8.
   - What is wrong: the appendix lists no per-action `unmapped` keys, only `sagat.action`, `analytic.name` and titles. Requirement 040-006 says the resource type goes into `unmapped` "when OCSF has no equivalent", but never says when that is; `resources[0].type` already carries it. It is also not stated whether `unmapped.sagat.action` is serialized as `{"sagat":{"action":…}}` or as the flat key `"sagat.action"`.
   - Why it matters: this is part of the wire format and of the generated reference. Two builders would emit different JSON.
   - Correction: add an "unmapped keys" column to the appendix, or state "none beyond `sagat.action` and `sagat.region_epoch`". Fix the JSON shape with one literal example event per class.

7. Major — The sink REST contract is not in any task, and slice 3 needs it
   - Where: Task 040-10, slice 2 file list (lines 599-603), Task 040-18, Verification Plan slice 3 (`npm run api`).
   - What is wrong: only Task 040-7 touches `openapi.yaml`. Nothing adds the sink operations: schemas, which fields `PATCH` may change, the `startFrom` body, the response fields (`scheme://host`, path fingerprint, last delivered record, oldest undelivered age), the refused-records page, or the error codes. Only `AUDIT_EXPORT_NOT_ACTIVATED` is named. Still unnamed are the fifth-sink refusal, egress not allowed, `pii` not permitted, `startFrom` older than 90 days, the 5-second lookup timeout, and the test rate limit. The path-fingerprint algorithm is also unspecified.
   - Why it matters: this breaks the documentation-first rule (AGENTS.md Rule 23). The Studio types of slice 3 cannot be generated, and every error code would be invented.
   - Correction: add a task before Task 040-10, "OpenAPI operations, schemas and error codes for `/audit/sinks*` with the contract baseline". List the error codes and the fingerprint rule (for example, the first 16 hex characters of HMAC-SHA256 under a platform key) in the plan.

8. Major — The internal contracts of slice 2 are missing, and Tasks 040-10 and 040-11 are too coarse to split
   - Where: Technical Plan 5 (lines 402-408), Tasks 040-9 to 040-11.
   - What is wrong: the plan gives no DDL for `audit_sinks` or `audit_operator_export`:
     - column types;
     - the representation of each sealed value (`key_id`, `nonce`, `ciphertext` times four);
     - the `state` CHECK values;
     - the claim columns;
     - the expiry of the previous key;
     - a last-test time for the one-per-minute limit.
     
     It also names no application port traits and no module or file names: for example `crates/application/src/audit_export.rs`, `crates/infra-postgres/src/audit_sinks.rs`, `crates/scheduler/src/audit_export.rs`. Task 040-11 bundles the look, claim, renewal, checks, query, batching, signing, commit, three failure classes, halving, eight states and operator grouping into one checkbox.
   - Why it matters: the API builder and the loop builder must agree on the same schema and ports, and nothing written lets them work in parallel.
   - Correction:
     - Add the DDL and the port signatures to the plan, with methods such as `look`, `claim`, `renew`, `read_batch`, `commit_cursor`, `set_state` and `record_refused`.
     - Split Task 040-11 into at least three tasks: claim and commit (store), send and failure classes (loop), and the operator grouping.

9. Major — What is signed and what the headers carry is undecided, and the contract task is ordered last
   - Where: Technical Plan 5 step 4 (lines 437-447), Task 040-20, Task 040-11.
   - What is wrong: `signing::Covered` (`crates/application/src/signing.rs:61`) signs `event_id`, `event_type`, `subject` and `correlation_key`. The plan does not say what fills `event_id` (the batch id or the delivery id, both "new per attempt"), `subject` or `correlation_key` for a batch. It also does not say why two per-attempt ids are both needed. Task 040-20 decides "the signed string" but is listed after Task 040-11, which implements it.
   - Why it matters: the documentation-first rule requires the contract before the code. The known signature vectors cannot be written until the fields are fixed.
   - Correction: fix the fields in the plan, for example `event_id` = delivery id, `event_type` = `ocsf.batch`, `subject` = the tenant id, `correlation_key` empty. Either drop `x-octopus-event-id` or define it. Move Task 040-20 before Task 040-11, and add a step to the CI `contracts` job that validates the new AsyncAPI file.

10. Major — The egress path is misnamed, and the operator sink has no egress guard
   - Where: Technical Plan 5 step 4 ("the egress port of the home namespace (`crates/infra-egress`)"), step 3, Task 040-12.
   - What is wrong: `crates/infra-egress` is an adapter with no port. The scheduler crate is in the application layer and may not depend on it (`scripts/check-architecture.py:94-101`). The existing port is `WebhookSender` with `OutboundRequest { tenant_id, namespace_id, … }` (`crates/application/src/outbound.rs:953-988`), implemented by `sagat-infra-connectors`. It needs a namespace. The operator sink has no namespace, only "allowed hosts".
   - Why it matters: the builder must design a new port or bend `WebhookSender`. That decision sits between Tasks 040-11 and 040-12.
   - Correction: say that tenant sinks reuse `WebhookSender`. Add a port, for example `AuditExportSender` taking an `EgressPolicy` built from the operator's allowed hosts, implemented in `sagat-infra-connectors` over `EgressClient`. Name the response-status mapping onto `SendFailure`.

11. Major — The operator sink's claim, states and resume are undefined
   - Where: Technical Plan 5 (lines 406-408, 465-468), Requirement 040-011, Task 040-12, Success criterion 040-006.
   - What is wrong: "only its cursor is persisted", yet section 4 says commit correctness comes from the claim token and the compare-and-set, not from the shard fence. Several things are left open:
     - whether `audit_operator_export` carries a claim token, expiry and state;
     - how `PAUSED_DESTINATION_REFUSED` or `CURSOR_INVALID` is left when no API can call `/resume` (a restart? a configuration change?);
     - where its `STALLED` back-off lives across replicas.
   - Why it matters: after a shard handover, two replicas can send the same range. The builder must also invent how the operator resumes the sink.
   - Correction: give `audit_operator_export` columns for the claim token and expiry, the state and `next_attempt_at`, and the CAS statement. State that the operator sink leaves a paused state on a configuration change or a restart, and write that into the runbook task.

12. Major — The `pii` recheck at commit has no defined function and no defined outcome
   - Where: Requirement 040-010; Technical Plan 5 step 5 (lines 448-453).
   - What is wrong: `has_effective_payload_reveal_grant` (`crates/infra-postgres/src/reveal_grants.rs:40`) checks one namespace and takes a shared grant-set lock. The "every namespace" read of the step-log quota rule is a different single-snapshot query, `step_log_usage_if_revealed`, which takes no fence. The plan cites both as one rule. It also does not say what happens when the recheck in the commit transaction fails: is the cursor advanced (the batch has left) and the sink paused, or is the cursor left behind?
   - Why it matters: lock order across N namespaces and the resend behaviour are both left to the builder.
   - Correction: name a new store function, for example `holds_reveal_grant_in_every_namespace(tx, tenant, subject)`, with its locking rule. State the failure path, for example: "commit the cursor, set `PAUSED_GRANT_REVOKED`, write `audit.sink_paused`, all in one transaction".

13. Major — The OCSF server fixture is unidentified, and the "contract job" does not run it
    - Where: Task 040-6, Success criterion 040-002, Task 040-2.
    - What is wrong: the plan names no image reference or digest, and no validation endpoint (for example `POST /api/v2/validate`). It does not say whether the image bundles schema 1.10.0 or must be built in this repository. It also does not say where Task 040-2's files are fetched from: the public server or the same pinned image. The CI `contracts` job (`.github/workflows/ci.yml:600-632`) runs `npx` tools only, not cargo or Testcontainers.
    - Why it matters: the builder must pick an artifact and a CI home. Vendored files fetched from the public server may not match the image used to validate.
    - Correction:
      - Name the image and its digest, or add `infra/docker/ocsf-server/Dockerfile` pinned to the server tag and to the `ocsf-schema` tag v1.10.0.
      - Name the test target, for example `contract_ocsf_event_standard` in `tests/Cargo.toml`.
      - Fetch the Task 040-2 files from that same image, with the exact URL and the `profiles` query per class.

14. Nit — No enum-generating macro exists, and other domain changes are implied but unlisted
    - Where: Requirement 040-001, Task 040-3.
    - What is wrong: `AuditAction` is a hand-written enum with a hand-written `as_str` (`crates/domain/src/audit.rs:19-456`). Task 040-3 says "from the enum's macro", as if one existed. Adding `Permission::AuditSinkManage` also means growing `Permission::ALL: [Self; 55]` (`crates/domain/src/identity.rs:401`), not only `ADMIN_PERMISSIONS`. One-per-minute collapsing of `audit.sink_delivered` needs a 60-second `repeat_window` and an entry in `collapses_repeats`, which no task lists.
    - Correction: say "write a declarative macro that generates the enum, `as_str`, `ALL` and `parse`, keeping the doc comments", and list the `Permission::ALL` and collapse changes.

15. Nit — The Rust sketch in Technical Plan 1 is inconsistent
    - Where: lines 303-311.
    - What is wrong: `OcsfMapping.status: Status` names an undefined type. `OcsfSeverity` is undefined. `UNKNOWN` uses "status Unknown", which `EventStatus { Success, Failure }` lacks.
    - Correction: define `enum Status { Event(EventStatus), Finding(FindingStatus) }` with `EventStatus::Unknown = 0`, and give `OcsfSeverity` its variants.

16. Nit — The reason cap conflicts with `scrub`
    - Where: Requirement 040-003.
    - What is wrong: `sagat_telemetry::scrub` cuts at 2,048 characters and appends `…[N characters cut]` (`crates/telemetry/src/redact.rs:53-60`). The result exceeds 2,048 characters and does not end with `…[truncated]`.
    - Correction: state the order of operations (scrub, then re-cap to 2,048 including the marker) or adopt scrub's marker.

17. Nit — The projection's interface is unstated
    - Where: Tasks 040-4, 040-7 and 040-11.
    - What is wrong: the plan never says what the projection takes and returns. Open points:
      - The input type is undefined. `AuditEntry` has no `tenant_id` or `tx_id`, and the export reads need another row type.
      - Nothing says how the ceiling is passed. `GET /audit/ocsf` should include the scrubbed reason, as `GET /audit` does, but this is not written.
      - Nothing says where the product major and minor version come from.
      - The region epoch has no source; nothing named `region_epoch` exists in the code, so it is a constant 1.
    - Correction: give the signature, for example `project(entry: &AuditRow, tenant: TenantId, ceiling: Ceiling, epoch: u64) -> OcsfEvent`.

18. Nit — Activating the registry capability needs a migration no task lists
    - Where: Task 040-16.
    - What is wrong: `audit-export-v1` must be added to the CHECK constraints of `serving_instances`, `platform_requirements` and `platform_requirement_history` (`20270113000000_instance_registry.up.sql:42-85`, widened again in `20270118000000_run_input_validation.up.sql`), and to `Capability` in `crates/application/src/instances.rs`. No task, including Task 040-9, says so.
    - Correction: add this to Task 040-9 or Task 040-16, with the kinds `EXECUTION_ENGINE` and `CONTROL_PLANE`.

19. Nit — Non-functional requirement 040-002 has no task and no test
    - Where: Task 040-7.
    - What is wrong: no task cites Non-functional requirement 040-002 or asserts one query per page.
    - Correction: add the reference and a statement-count assertion to Task 040-7.

20. Nit — The records written for the seven new actions have no defined shape
    - Where: Technical Plan 6 (audit actions), Task 040-10.
    - What is wrong: the plan never fixes the actor (for example `Subject::engine` for the pass), the resource type and id, or the reason format. `GET /audit/sinks/{sinkId}/refused` cannot be written until it is known whether the refused record id is the resource or sits in the reason.
    - Correction: add one line per action giving its resource type, resource id and reason.

21. Nit — Paths and mechanisms the builder must look up
    - Where: Tasks 040-5, 040-8, 040-9; slice 2 test target.
    - What is wrong: the plan leaves these unstated:
      - "classification entries" means `crates/infra-postgres/storage-classification.json`;
      - the validation test needs `jsonschema = "0.58"` as a dev-dependency of `sagat-application`;
      - the reference drift test should follow the existing `SAGAT_DOCS_BLESS=1 … contract_developer_experience` pattern;
      - the new target `integration_ocsf_export` must be declared in `tests/Cargo.toml`;
      - "the migration is retried when it times out" names no actor.
    - Correction: name each of these in its task.

22. Nit — Task hygiene
    - Where: Tasks 040-1 and 040-21; line 373.
    - What is wrong:
      - Task 040-1's work is already done (`docs/owner-decisions.md`, rows OD-2026-11-26-100 to 106) and its `Refs: Requirement 040-001` is not what it closes.
      - Task 040-21 cannot be built until feature 032 exists and has no dependency line. Its Base Event (class 0) schema is not among the vendored classes.
      - "When every record read was withheld, to the last record read" (line 373) is dead logic, because the SQL already filters out withheld actions.
    - Correction: fix the reference on Task 040-1. Move Task 040-21 under Follow-up tasks with its dependency, and include the class 0 schema. Delete the withheld-cursor sentence.
