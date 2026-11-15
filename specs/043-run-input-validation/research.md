# Research: run input validation (design 043)

Registers of decisions (`D043-…`), gaps (`G043-…`) and owner questions (`O043-…`) for
`docs/designs/043-run-input-validation.md`. Append-only: a number is never reused or renumbered, a gap is
closed by evidence and never deleted, and a later finding is a new dated entry.

The owner decision this feature implements is OD-2026-11-14-2 ("Validar todos os starts já", answered on
2026-11-14 against the recommendation to defer gap G1955 of feature 018). It is recorded on the branch
`feature/018-schedules-default-on` and is not on `develop` at the time of writing.

## 1. What exists today (verified 2026-11-14 on `develop` at `c2e9341f`)

- `inputSchema` is a document-level field of the IR (`specs/001-platform-foundation/contracts/workflow.schema.json:47`,
  `{"type": "object"}`), not part of the parsed `WorkflowDefinition` (`crates/domain/src/workflow.rs:1228`): it lives
  only in the stored document bytes (`VersionRecord.document`).
- At publish, an `inputSchema` is compiled **only when it carries an `x-sagat-classification` annotation**
  (`crates/infra-ir/src/carrier_schema_check.rs:12-23`, called from `crates/infra-ir/src/lib.rs:333`). An unannotated
  schema of a published version was never compiled and may not compile.
- The annotation is inert to validation (`crates/infra-schema/tests/annotation_is_inert.rs`). No other `x-sagat-*`
  keyword appears in a schema (`x-sagat-error` and `x-sagat-remediation` are OpenAPI extensions).
- `crates/infra-schema/src/lib.rs:74` compiles with `jsonschema` 0.58.6, `default-features = false` (no network or file
  resolution: an external `$ref` is a compile error, test `claim_check_schema_validation_refuses_external_refs_without_exposing_payloads`),
  the linear-time regex engine and a 1 MiB pattern size limit. The draft comes from `$schema`, else 2020-12; `format` is
  an annotation under 2019-09 and 2020-12 (the crate's default).
- `crates/infra-schema/src/lib.rs:90` `validate` reports up to three violations as `"<instance path> breaks <rule>"`.
  The instance path can hold keys the caller chose (a map key under `additionalProperties`), so it is not value-free for
  a run input.
- The only start that checks its input against `inputSchema` is a start with an uploaded input
  (`crates/application/src/runs.rs:1673`, the `RunInputChecker` port of `crates/application/src/blobs.rs:284`,
  implemented at `crates/infra-schema/src/lib.rs:16`). It compiles the schema on every call; nothing is cached.
- `crates/infra-schema/src/classification/cache.rs` is a bounded compiled-validator cache with single flight per key,
  remembered failures and at most four concurrent compiles, keyed by `(version, carrier)`.
- Every run row is written by one of three `INSERT INTO runs`: `PgRunStore::create`
  (`crates/infra-postgres/src/execution.rs:1108`, insert at :1369) and the two continue-as-new writes
  (`crates/infra-postgres/src/chain_progress.rs:1267` and :1750).
- No creator classifies the run input at start; classification applies when a payload is read.

## 2. Decisions

- **D043-001** (2026-11-14; port split by D043-017, the test constructor by D043-017 and D043-019) One port, `RunInputValidator`, in `crates/application`, implemented in
  `crates/infra-schema`. Every creator calls it **before** the transaction that creates the run opens. The creation
  inputs (`NewRun.input` and the input of a `ContinuationSelection`) become a `CheckedRunInput`, a type only the port
  builds, so a new creator that skips the check does not compile. An escape constructor for tests is allowed only under
  `tests/` and `#[cfg(test)]`, checked by `scripts/check-architecture.py`. Rejected: putting the check in
  `PgRunStore::create` (infra layer, inside the creation transaction, and continue-as-new does not pass there).
- **D043-002** (2026-11-14; cache key superseded by D043-020) The parsed `WorkflowDefinition` carries the canonical bytes of its `inputSchema` (an opaque
  byte string in the domain, no `serde_json`). Whoever holds the definition holds the schema, so no creator reads the
  database for it: the start, the trigger and the child already read the version row, and the continuation holds the
  predecessor's definition from the run transaction. The compiled validator is cached per version id (a version is
  immutable), in a cache of the same design as the classification cache, with its own bounds.
- **D043-003** (2026-11-14) Which creators validate. All of them, except the legacy continue-as-new
  (`chain_progress.rs:1149`), which is called only from `tests/integration/workflow-primitives/chain_progress.rs` and
  carries the predecessor's input byte for byte to the same version. The table is in the design doc, Technical Plan 1.
- **D043-004** (2026-11-14) A refused start does not consume its idempotency key. Before it answers the refusal, the
  start looks the key up (`replay_start_authorized`) and replays a run created under the same key and fingerprint. A
  client that retries, after the upgrade, a start that was accepted before it, gets the run, not a refusal. The lookup
  is on the refusal path only.
- **D043-005** (2026-11-14; confirmed by OD-2026-11-14-10) Signal-with-Start, Update-with-Start and a start with `USE_EXISTING` validate the input
  before admission, also when an open holder will answer and no run is made. Otherwise the answer would depend on a race
  with the holder's end.
- **D043-006** (2026-11-14; `format` and the compile options superseded by D043-021 and D043-023) Dialect: the existing compile options (D043-001's adapter reuses `compile` of
  `crates/infra-schema/src/lib.rs:74`): `$schema` honoured, 2020-12 otherwise, `format` not asserted, no external
  reference, linear regex. Changing any of them is a separate decision.
- **D043-007** (2026-11-14; the rule mapping and the cut rule superseded by D043-024) What a refusal says. The first violation only: `details.reason`, `details.rule` (the
  keyword, from a closed list: `type`, `required`, `enum`, `const`, `minimum`, `maximum`, `exclusiveMinimum`,
  `exclusiveMaximum`, `multipleOf`, `minLength`, `maxLength`, `pattern`, `minItems`, `maxItems`, `uniqueItems`,
  `minProperties`, `maxProperties`, `additionalProperties`, `propertyNames`, `dependentRequired`, `oneOf`, `anyOf`,
  `allOf`, `not`, `if`, `contains`, `other`) and `details.field`, a JSON pointer cut before the first segment that is
  neither an array index nor a property name declared under some `properties` of the schema. For `required`, the
  missing name (from the schema) is appended. No value, no unexpected key, no `enum` member.
- **D043-008** (2026-11-14) Schedules. At write, the input is checked against the version the target names at that
  moment (the pinned one, or the active one), with `nominalTimeField` filled with the next nominal time in RFC 3339.
  `nominalTimeField` must be a declared top-level property whose `type` is or includes `"string"` when the version has
  an `inputSchema`; with no schema it is accepted, as the input is. The action-time check is the authority (the active
  version can change between write and action).
- **D043-009** (2026-11-14; refined by D043-026: one decision, the backfill `CANCELLED`, the check before `decide`) A schedule action whose input is refused is recorded with a new outcome `INPUT_INVALID`
  and the reason (`SCHEDULE_INPUT_INVALID` or `NOMINAL_TIME_FIELD_INVALID`). The time is acted on, not owed: today a
  `NotAllowed` defers the time and retries it forever (`crates/application/src/schedule_firing.rs:1203`), which would
  loop on an input that can never pass. No remaining action is used up, as for `BUSINESS_ID_CONFLICT`
  (`record_held`, :1333). The schedule pauses when its `pause_on_failure` policy is set, as for a run it started that
  failed. A backfill stops at its first `INPUT_INVALID`. Trigger-now answers `422` and records nothing, as for its other
  admission refusals.
- **D043-010** (2026-11-14; the audit refined by D043-027) A trigger whose run input is refused starts nothing, writes a value-free audit record
  (`run.input_refused`, resource the workflow version) and counts `trigger_events_total{outcome="input_invalid"}`, the
  pattern of the database denial (`crates/scheduler/src/events.rs:526`). The event is not retried for that trigger and
  the other consumers of the event are served. No dead letter: the event was accepted into the inbox, and the connector
  dead letters (`connector_dead_letters`) are for what could not be ingested.
- **D043-011** (2026-11-14) A child whose input is refused is not created; the parent step is told
  `ChildOutcome::Failed(CHILD_INPUT_INVALID)`, as for `CHILD_NOT_FOUND` (`crates/scheduler/src/passes.rs:5217`).
- **D043-012** (2026-11-14) A continue-as-new whose evaluated input is refused creates no successor; the predecessor
  ends `FAILED` with `CONTINUATION_INPUT_INVALID` through the path of an expression failure
  (`finalize_continuation_expression`, `crates/scheduler/src/passes.rs:982`).
- **D043-013** (2026-11-14) A batch `RERUN` item whose input is refused ends `FAILED` with `VALIDATION_FAILED`
  (`error_code`, `crates/application/src/batch_executor.rs:134`). A pending reset whose run is refused ends with the
  existing `ADMISSION_REFUSED` (`failure_of`, `crates/application/src/reset_completion.rs:220`): no new value in that
  closed set.
- **D043-014** (2026-11-14) A new `AppError::InvalidRunInput { reason, field, rule }`, mapped to `VALIDATION_FAILED`
  (422) with those three `details`. `InvalidStart` is not reused: its message says "start option".
- **D043-015** (2026-11-14) Ad hoc runs (feature 017, `start_private`) go through the same port and are validated when
  their document declares an `inputSchema`.
- **D043-016** (2026-11-14) The design is written on `develop` at `c2e9341f` (pull request 98 merged), because
  Signal-with-Start, Update-with-Start, reset and the batch executor are there and were not at the branch's first base
  `a4f6e9db`. The branch had no commit of its own and was fast-forwarded.

### Second draft, after the three reviews and the owner's answers (2026-11-14)

- **D043-017** (2026-11-14; the constructor's visibility superseded by D043-032, the key by D043-041 below) The port is narrow: `SchemaValidator` (implemented by `InputSchemas` in `crates/infra-schema`)
  takes a key, the schema bytes and the input bytes and answers accept or a `SchemaRefusal` (`Invalid` with segments and
  rule, `Unusable`, `TooDeep`). A concrete `RunInputGate` in `crates/application` owns the mode, the metrics, the
  blocking pool and the only non-test constructor of `CheckedRunInput`, which is `pub(crate)`. The test constructor is
  behind a `test-support` cargo feature of `sagat-application`.
- **D043-018** (2026-11-14) `CheckedRunInput` carries the version id it was checked against; `PgRunStore::create` and
  `continue_evaluated` refuse a mismatch as an internal error.
- **D043-019** (2026-11-14) Three architecture rules in `scripts/check-architecture.py`: `INSERT INTO runs` only at its
  three places (`crates/infra-postgres/src/execution.rs:1369`, `crates/infra-postgres/src/chain_progress.rs:1267`, `:1750`;
  the rule counts per file so that a moved line is not a failure); `.continue_as_new(` only under `tests/`,
  `crates/*/tests/` and `#[cfg(test)]`; the `test-support` feature only in `[dev-dependencies]`.
  `EvaluatedChainHandoffCommand.input` (`crates/application/src/chain_progress.rs:246`) and `ContinuationSelection` hold a
  `CheckedRunInput`.
- **D043-020** (2026-11-14) The compiled-validator cache is keyed by `(tenant, SHA-256 of the canonical schema bytes)`,
  computed once at parse and carried by `InputSchema` (an `Arc<[u8]>` and the digest). Identical schemas of one tenant
  share an entry; two tenants never do. Ad hoc starts (`crates/application/src/runs/ad_hoc.rs:459`, a new version id per
  call) compile and validate outside the shared cache, under their own single permit.
- **D043-021** (2026-11-14; references and cycles refined by D043-036) Compile bounds, applied before `build` and again at publish: the classification
  pre-checks (`MAX_CARRIER_BYTES` 64 KiB of schema, 64 KiB per pattern, `MAX_SCHEMA_DEPTH` 32, `MAX_CARRIER_PATTERNS` 32,
  the dry `Walk` with `MAX_WALK_STEPS` 4,000,000), a refusal of applicator `$ref` cycles, and a retriever that refuses
  every URI. A failure is `SchemaUnusable`, remembered per key. Cache entries are weighed with the real per-pattern
  limit.
- **D043-022** (2026-11-14; the permits and the deadline superseded by D043-033 to D043-035) Validation budget: the gate runs the validator in `spawn_blocking` behind a
  `tokio::sync::Semaphore` of four permits, under the deadline `MAX_PROJECTION_DURATION` (5 s); the cache's `Condvar`
  is waited on only on the blocking pool. A missed deadline is `503 UNAVAILABLE` (retryable) for a caller and a
  transient refusal for the engine's creators. The input is parsed with a depth limit of 64 before validation; a deeper
  input, or one deeper than `serde_json` parses (recursion limit 128), is `RUN_INPUT_INVALID` with `rule` `maxDepth` and
  `field` `/`.
- **D043-023** (2026-11-14) `format`, `contentMediaType` and `contentEncoding` never assert, under every draft:
  `should_validate_formats(false)`, `without_content_media_type_support("application/json")`,
  `without_content_encoding_support("base64")`. Under drafts 4, 6 and 7 `jsonschema` 0.58.6 would otherwise assert them
  (`src/compiler.rs:66-68`, `:538-541`; `src/keywords/mod.rs:473-480`).
- **D043-024** (2026-11-14; the field of repeats refined by D043-037; `workLimit` added to the enum by D043-033) Error text: the rule comes from a `match` on `ValidationErrorKind` that names every variant
  (no `_` arm), into a closed enum (`type`, `required`, `enum`, `const`, `minimum`, `maximum`, `exclusiveMinimum`,
  `exclusiveMaximum`, `multipleOf`, `minLength`, `maxLength`, `pattern`, `minItems`, `maxItems`, `uniqueItems`,
  `contains`, `minProperties`, `maxProperties`, `additionalProperties`, `unevaluatedProperties`, `unevaluatedItems`,
  `propertyNames`, `dependentRequired`, `dependentSchemas`, `oneOf`, `anyOf`, `allOf`, `not`, `if`, `falseSchema`,
  `maxDepth`, `other`). A `ValidationError` is never formatted. The pointer keeps an instance segment only where the
  evaluation went through `properties/<name>`, or an index through `items`, `prefixItems` or `contains`; `required`
  appends the missing name from the schema. For a rerun or reset the field is `/` unless the caller passes the read
  check of the source run's input.
- **D043-025** (2026-11-14) `SchemaUnusable` has an outcome on every row: `422 RUN_INPUT_SCHEMA_UNUSABLE` for a caller;
  the trigger's delivery and audit with that reason; a schedule action `INPUT_INVALID` with reason
  `RUN_INPUT_SCHEMA_UNUSABLE`; the failure code `RUN_INPUT_SCHEMA_UNUSABLE` for a child step and a continuation; a batch
  item `FAILED VALIDATION_FAILED`; a pending reset `ADMISSION_REFUSED`. None is retried (OD-2026-11-14-11).
- **D043-026** (2026-11-14) A schedule refusal is one `ScheduleDecision`: the action, the auto-pause with its
  `ScheduleAutoPaused` audit when `pause_on_failure` is set, and a backfill's end as `CANCELLED`, as `revoke_authority`
  does (`crates/application/src/schedule_firing.rs:1464`). The check runs before `decide`, so trigger-now answers `422`
  instead of `BUFFERED`; a buffered action is checked again at its release. The arm sits before `NotAllowed` (`:1203`),
  so the new variant does not fall to `Err(other)` (`:1214`) and refire every tick.
- **D043-027** (2026-11-14) A trigger refusal writes, in one transaction, an `event_deliveries` row to `trigger:<id>`
  with no run (`run_id` is nullable, `crates/infra-postgres/migrations/20261012000000_events.up.sql:89`) and, only when that
  row is inserted, the audit. A retry of the event after another consumer failed (`crates/scheduler/src/events.rs:304-334`)
  writes nothing again. A cron firing has no inbox event; its audit names the tick key (`tick_identity`, `events.rs:801`).
- **D043-028** (2026-11-14, OD-2026-11-14-8) The mode: `SAGAT_RUN_INPUT_VALIDATION` parsed strictly (`enforce`,
  `report`; unset or empty is `enforce`; anything else stops the process naming the variable), as ADR-0081 does for its
  flag. A process lists `run-input-validation-v1` (a new `Capability`, `crates/application/src/instances.rs:108`, and the
  two closed lists of `crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql:42-54`) only in `enforce`.
  The operator records it as a requirement of the control plane and the engine after the upgrade; a 0.2.x binary and a
  process in `report` are then refused (ADR-0082). Going back to `report` means removing the requirement first. No
  shipped manifest sets `report` (a test like `no_shipped_manifest_turns_the_reset_switch_on`,
  `apps/execution-engine/tests/config.rs:1159`). In `report` mode a reported refusal writes an audit row, one statement on
  that start; NFR-043-PERF is stated for `enforce` only.
- **D043-029** (2026-11-14) Replay: the fingerprint of a start holds the version id (`crates/application/src/runs.rs:1538`),
  so a retry after another version is promoted is a different request and gets the existing `409` for a reused key, as
  today. Ad hoc starts have an idempotency of their own and get their own lookup before a refusal.
- **D043-030** (2026-11-14) Runs created before the upgrade: their children and continuations are checked
  (OD-2026-11-14-9), their reruns and resets are checked (OD-2026-11-14-11), and a Signal-with-Start or
  Update-with-Start to an open holder is refused for a mismatched start input (OD-2026-11-14-10). The first is a deviation
  from constitution principle IV that the owner accepted; the release notes say that an old run can fail at the step that
  creates a child or continues.
- **D043-031** (2026-11-14) One migration: the action outcome `INPUT_INVALID` and the capability `run-input-validation-v1`.
  Each widened `CHECK` is added `NOT VALID` under a new name, then `VALIDATE CONSTRAINT`, then the old one is dropped;
  the down migration refuses while a row holds a new value.

### Third draft, after the second review round (2026-11-14, at `42082d73`)

- **D043-032** (2026-11-14) The fields and the only non-test constructor of `CheckedRunInput` are private to
  `crates/application/src/run_input.rs`, not `pub(crate)`. A fourth architecture rule fails on a struct expression or an
  associated function call of `CheckedRunInput` in any other file outside test paths.
- **D043-033** (2026-11-14) The bound on validation work is the classification projection's instance-aware `Walk`
  (`crates/infra-schema/src/classification/walk.rs`), charged against the input before `iter_errors` with
  `MAX_WALK_STEPS` (4,000,000). Past it the input is refused with `RUN_INPUT_INVALID`, `rule` `workLimit`, `field` `/`:
  deterministic for a schema and an input, so final on every row. `workLimit` joins the closed rule enum.
- **D043-034** (2026-11-14) Permits: four global per process; per tenant at most one shared-cache permit and one ad hoc
  permit in flight (a bounded map of per-tenant semaphores of one), because the caller chooses an ad hoc document
  (`crates/application/src/runs/ad_hoc.rs:400-405`). The tenant permit and an `OwnedSemaphorePermit` of the global
  semaphore move into the `spawn_blocking` closure, so a thread that outlives its caller's deadline keeps them until it
  ends: at most four blocking threads, and a slow tenant holds at most two permits.
- **D043-035** (2026-11-14) The deadline (`MAX_PROJECTION_DURATION`, 5 s) covers acquiring the permits and the work. A miss
  is never cached as `Unusable`. A bounded in-process memory (4,096 entries, least recently used out) keyed by
  `(tenant, schema digest, input digest)` counts misses: **the first miss is transient, the second is final**, answered
  as `RUN_INPUT_INVALID` with `workLimit` on every row. A first miss is `503` for a caller, a deferral for a batch item
  (`crates/application/src/batch_executor.rs:540`), a pending reset left pending (`failure_of` answers `None`,
  `crates/application/src/reset_completion.rs:237`), an owed schedule time, a retried event, and a child or continuation
  left for the next pass. The threshold of two lets a single slow moment pass and stops an input that is slow every
  time.
- **D043-036** (2026-11-14) References. The retriever that refuses every URI is the control on remote references: `build`
  resolves every `$ref`, `$dynamicRef` and `$schema` through it, wherever they are. The dry walk sees only what is
  reachable from the root and is not that control. Before `build`, a walk of the whole document checks that every
  reference is document-local and that the graph of non-consuming applicator edges has no cycle; recursion through a
  keyword that moves to a child instance is allowed. The shared `Walk` refuses `#` and anchors (`walk.rs:139`), which
  would make recursive tree schemas unusable: the run input path calls it with a reference policy that resolves `#`,
  `#/...` and the document's `$anchor`s; the classification path keeps its policy. `$dynamicRef` and `$recursiveRef` are
  unusable.
- **D043-037** (2026-11-14) For a rerun or reset, `FieldPolicy::Show` only when the caller's view of the source run's input
  under feature 016's graded disclosure withholds nothing; otherwise `field` `/` and `rule` `other`, so the answer never
  describes a withheld value.
- **D043-038** (2026-11-14) Before failing a parent step for a refused child, the engine looks up the child's idempotency
  record (`child:{parent}`, `child-start:{node}`, `crates/scheduler/src/passes.rs:5302-5306`). A child that a peer, a 0.2.x
  engine or an engine in `report` already created is answered as a replay, and the step is not failed.
- **D043-039** (2026-11-14) In `report` mode the audit of a reported refusal is one of the records written in the
  transaction that creates the run, so a retry or a replay does not audit twice.
- **D043-040** (2026-11-14) Before the upgrade, the runbook runs the inventory query and, when the operator wants it, sets
  `SAGAT_RUN_INPUT_VALIDATION=report` in every engine's and control plane's manifest before migrating: the engines are
  upgraded first and default to `enforce`, so they would otherwise record final outcomes before anyone could choose. The
  requirement `run-input-validation-v1` can be recorded only once every live process lists it (ADR-0082 decision 2
  refuses it while a live instance lacks it).
- **D043-041** (2026-11-14) `SchemaValidator::check` takes the `SchemaKey`, the `&InputSchema` and the input. The gate builds
  the key from the tenant, the digest that `InputSchema` carries and the creator (`shared` is false for `Creator::AdHoc`).

## 3. Gaps

| ID | Gap | Reason / closure | Status |
|---|---|---|---|
| G043-001 | A control plane of 0.2.x running beside new engines during the upgrade meets the new outcome `INPUT_INVALID`. The action listing reads the outcome as a string by design (`crates/application/src/schedules.rs:634-635`, `crates/infra-postgres/src/schedules.rs:547`), so it passes through; the outcome filter parses the name strictly (`crates/api-http/src/routes/schedules.rs:275`) and an old binary answers `422` to a filter on `INPUT_INVALID`. | Read in the code, not yet tested: T043-11 adds the test of an action list with an unknown outcome. The filter case is stated in the release notes. | Open |
| G043-002 | A trigger refusal could be audited again on every retry of its event. | Closed for inbox events by D043-027 (the delivery row), proven by T043-7. Left: two engines that reach the same cron tick at once can both audit it; the reason names the tick key, so the duplicate is recognisable. | Open (narrowed 2026-11-14) |
| G043-003 | `details.field` names property names declared in the schema to a caller with `run:start`. | Closed 2026-11-14: every role that holds `run:start` also holds `definition:read` (`crates/domain/src/identity.rs:607-691`), so the caller may read the schema the names come from. T043-23 adds the unit test that keeps it so for every `Role`. | Closed (by reading; test in T043-23) |
| G043-004 | A published version with a non-manual trigger and an `inputSchema` that describes a manual input (and not the event envelope `{event, data}`) refuses every triggered start after the upgrade. | The release notes and the runbook carry a query that lists versions with both (OD-2026-11-14-8); it cannot tell which schemas accept the envelope without real events. `report` mode covers the time to fix them. | Open |
| G043-005 | `format`, `contentMediaType` and `contentEncoding` are not asserted under any draft, so a schema that relies on them to refuse an input does not refuse it. | D043-023; documented in the runbook. | Open (documented) |
| G043-006 | The CPU cost of validating a 256 KiB input on the API path is not measured. | Counters, not timers, prove the compile count; a benchmark is not a target of this design. | Open |
| G043-007 | Inputs stored before the upgrade (schedules, pending resets) are not checked at the upgrade; they are refused at action time. | By design: the action time is the authority (D043-008). Stated in the release notes. | Open (documented) |
| G043-008 | Signals, Updates, worker results and activity outputs are not validated against a schema by this feature. | Out of scope: OD-2026-11-14-2 is about starts. | Open (out of scope) |
| G043-009 | `infra-schema::validate` (scripts, feature 005) reports instance paths that can hold caller-chosen keys. | Observed while reading the code; not changed here (another feature's contract). Reported to the owner. | Open |
| G043-010 | Until the operator records `run-input-validation-v1`, replicas may run different modes and a 0.2.x control plane admits what a new one refuses; the first upgraded engine runs `enforce` unless the manifest said `report` before the migration. | D043-040: the runbook sets the mode before migrating; the requirement can only be recorded once every live process is in `enforce` (ADR-0082 decision 2). | Open (documented) |
| G043-011 | A validation that misses its deadline under load is refused for a while: transient once, final the second time for the same schema and input. A final refusal caused by load, not by the input, is possible. | D043-035; the instance-aware walk (D043-033) bounds the work, so a miss means a starved machine. Counted `outcome="deadline"`. The deadline value is not measured (G043-006). | Open |
| G043-012 | A rerun or reset refused for its input answers `field` `/` to a caller who may not read the source run's input, which says less than the caller needs to fix it. | Accepted: the field names a property of a payload the caller may not see. | Open (accepted by design) |
| G043-013 | The miss memory of D043-035 is per process: across processes and restarts a slow input can be tried at most twice per process lifetime before it is final there. | A durable memory would add a statement on the refusal path and a table; not worth it for a backstop. | Open (accepted by design) |
| G043-014 | `$dynamicRef` and `$recursiveRef` make a schema unusable, so a published schema that uses them refuses every start of its version. | D043-036; the inventory query lists versions whose schema fails the compile. | Open (documented) |

## 4. Owner questions

All answered on 2026-11-14 (`docs/owner-decisions.md`). OD-2026-11-14-2 had already decided that validation is on in
0.3.0 for every start; none of these asked that again. The questions are kept as they were put.

- **O043-001: answered (a)**, OD-2026-11-14-8, with the mode going through the instance registry (D043-028).
- **O043-002: answered (a)**, OD-2026-11-14-11, permanent for the creators nobody waits for (D043-025).
- **O043-003: answered (a)**, OD-2026-11-14-11.
- **O043-004: answered, against the recommendation**, OD-2026-11-14-9 (D043-030). D043-005 was put to the owner
  as well and confirmed by OD-2026-11-14-10, against the recommendation to let an open holder answer.


- **O043-001 — What protects existing users from the behaviour change.** (a) A deployment setting
  `run_input_validation` with `enforce` (the default) and `report` (validate, count and audit, but admit), read at
  start by every process, logged at start, removed no earlier than 0.4.0; with the release-note warning and the
  inventory of G043-004. (b) A per-namespace setting with the same two values, through the API, the six SDKs, the CLI
  and the Studio, audited. (c) The release-note warning and the inventory only, no switch. (d) `report` by default in
  0.3.0: excluded, it contradicts OD-2026-11-14-2. **Recommended: (a)** — the owner's default holds, an operator has one
  lever that needs no contract change, and (b) costs six SDK surfaces for a transition aid.
- **O043-002 — A published `inputSchema` that does not compile** (an unannotated schema was never compiled at
  publish). (a) Fail closed: every start of that version is refused with `RUN_INPUT_SCHEMA_UNUSABLE` until a new
  version is published; `report` mode admits it and counts it. (b) Fail open: start without validation, count
  `schema_unusable`, warn in the log once per version and process. **Recommended: (a)** — a contract that cannot be
  checked is not enforced by being skipped, and from 0.3.0 no new version can be published with one (FR-043-012).
- **O043-003 — Rerun, reset and batch rerun of a run whose stored input its version refuses** (a run created before
  0.3.0). (a) Validate, as every start: the repeat is refused and the remedy is a new start with a corrected input.
  (b) Exempt the repeats: the same version admitted the same bytes once. **Recommended: (a)** — one rule for every
  creator, and (b) lets 0.3.0 create runs whose input breaks their own contract.
- **O043-004 — Children and continuations of runs created before the upgrade** (raised by the reviews of the first
  draft). (a) Exempt them: a run in progress must not break (constitution IV). (b) Check them like every start.
  **Recommended: (a).** Answered (b) by the owner (OD-2026-11-14-9), recorded as an accepted deviation from principle IV.
