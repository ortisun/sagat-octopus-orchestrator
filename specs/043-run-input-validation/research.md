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

- **D043-017** (2026-11-14; the port made async and the runtime moved into the adapter by D043-071; the constructor's visibility superseded by D043-032, the key by D043-041 below) The port is narrow: `SchemaValidator` (implemented by `InputSchemas` in `crates/infra-schema`)
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
- **D043-021** (2026-11-14; superseded by D043-048: the subset's pre-check is the compile bound) Compile bounds, applied before `build` and again at publish: the classification
  pre-checks (`MAX_CARRIER_BYTES` 64 KiB of schema, 64 KiB per pattern, `MAX_SCHEMA_DEPTH` 32, `MAX_CARRIER_PATTERNS` 32,
  the dry `Walk` with `MAX_WALK_STEPS` 4,000,000), a refusal of applicator `$ref` cycles, and a retriever that refuses
  every URI. A failure is `SchemaUnusable`, remembered per key. Cache entries are weighed with the real per-pattern
  limit.
- **D043-022** (2026-11-14; superseded by D043-033 to D043-035, then by D043-047 and D043-050) Validation budget: the gate runs the validator in `spawn_blocking` behind a
  `tokio::sync::Semaphore` of four permits, under the deadline `MAX_PROJECTION_DURATION` (5 s); the cache's `Condvar`
  is waited on only on the blocking pool. A missed deadline is `503 UNAVAILABLE` (retryable) for a caller and a
  transient refusal for the engine's creators. The input is parsed with a depth limit of 64 before validation; a deeper
  input, or one deeper than `serde_json` parses (recursion limit 128), is `RUN_INPUT_INVALID` with `rule` `maxDepth` and
  `field` `/`.
- **D043-023** (2026-11-14; applies now only to the oracle of the property test, D043-047) `format`, `contentMediaType` and `contentEncoding` never assert, under every draft:
  `should_validate_formats(false)`, `without_content_media_type_support("application/json")`,
  `without_content_encoding_support("base64")`. Under drafts 4, 6 and 7 `jsonschema` 0.58.6 would otherwise assert them
  (`src/compiler.rs:66-68`, `:538-541`; `src/keywords/mod.rs:473-480`).
- **D043-024** (2026-11-14; the rule mapping superseded by D043-049; the field of repeats by D043-037) Error text: the rule comes from a `match` on `ValidationErrorKind` that names every variant
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
- **D043-033** (2026-11-14; superseded by D043-047: the validator's own counter is the bound) The bound on validation work is the classification projection's instance-aware `Walk`
  (`crates/infra-schema/src/classification/walk.rs`), charged against the input before `iter_errors` with
  `MAX_WALK_STEPS` (4,000,000). Past it the input is refused with `RUN_INPUT_INVALID`, `rule` `workLimit`, `field` `/`:
  deterministic for a schema and an input, so final on every row. `workLimit` joins the closed rule enum.
- **D043-034** (2026-11-14; the per-tenant permits removed by D043-050) Permits: four global per process; per tenant at most one shared-cache permit and one ad hoc
  permit in flight (a bounded map of per-tenant semaphores of one), because the caller chooses an ad hoc document
  (`crates/application/src/runs/ad_hoc.rs:400-405`). The tenant permit and an `OwnedSemaphorePermit` of the global
  semaphore move into the `spawn_blocking` closure, so a thread that outlives its caller's deadline keeps them until it
  ends: at most four blocking threads, and a slow tenant holds at most two permits.
- **D043-035** (2026-11-14; the miss memory and the second-miss threshold removed by D043-050) The deadline (`MAX_PROJECTION_DURATION`, 5 s) covers acquiring the permits and the work. A miss
  is never cached as `Unusable`. A bounded in-process memory (4,096 entries, least recently used out) keyed by
  `(tenant, schema digest, input digest)` counts misses: **the first miss is transient, the second is final**, answered
  as `RUN_INPUT_INVALID` with `workLimit` on every row. A first miss is `503` for a caller, a deferral for a batch item
  (`crates/application/src/batch_executor.rs:540`), a pending reset left pending (`failure_of` answers `None`,
  `crates/application/src/reset_completion.rs:237`), an owed schedule time, a retried event, and a child or continuation
  left for the next pass. The threshold of two lets a single slow moment pass and stops an input that is slow every
  time.
- **D043-036** (2026-11-14; superseded by D043-042, then by D043-048) References. The retriever that refuses every URI is the control on remote references: `build`
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

### Fourth draft, after the third review (2026-11-14, at `4d2c13cb`)

- **D043-042** (2026-11-14; superseded by D043-046 and D043-048) **No recursion: simplify rather than patch the walk.** The run input path refuses as
  `SchemaUnusable`, before `build`, by a walk of the whole document: a `$ref` that is not a JSON pointer into the document
  (`#/...`), so `#` and anchors; any `$ref` cycle, consuming input or not; `$dynamicRef`, `$recursiveRef`,
  `$dynamicAnchor`, `$recursiveAnchor`; a `$id` other than at the root (the walk resolves against the root, `jsonschema`
  within the nested resource); a `$ref` reachable under a keyword the walk does not walk. The instance-aware walk follows
  every `$ref`, walks every applicator (`contains`, `if`/`then`/`else`, `dependentSchemas`, `dependencies`,
  `unevaluated*`, `additionalItems`, `propertyNames`, which the classification walk charges by literal size,
  `walk.rs:180-182`) and charges a declared key for every matching `patternProperties` entry (the classification walk
  does not, `walk.rs:474-479`). With those, its charge is an upper bound on `jsonschema`'s evaluation, and a test holds
  it against the output units of `Validator::evaluate(...).list()`. Reason: the third review found an exponential case
  (`{"items":{"$ref":"#"},"contains":{"$ref":"#"}}` on 64-deep arrays) and a same-instance recursion that can overflow
  the stack (`{"dependencies":{"a":{"$ref":"#"}}}` with `{"a":1}`), both reachable only because D043-036 allowed `#`.
  The cost is that recursive schemas customers published are unusable (G043-014); the release-notes inventory lists
  them, and `report` mode (OD-2026-11-14-8) covers the time to republish.
- **D043-043** (2026-11-14; removed by D043-050) Two clocks: waiting for the permits has its own timeout, always transient and never counted, so
  a queue of a tenant's own valid actions never becomes `INPUT_INVALID`; the work deadline starts when the blocking
  closure holds its permits and only its miss counts. For row 10 the miss memory is keyed by `(tenant, schema digest,
  tick identity)`, because a cron input carries `firedAt` = now (`crates/application/src/schedule_triggers.rs:113`,
  `crates/scheduler/src/events.rs:802`) and its digest changes on every retry.
- **D043-044** (2026-11-14; removed with the per-tenant permits by D043-050) A per-tenant semaphore leaves its map only when the map holds the only `Arc` and its permit is
  free; eviction never runs while it is held or awaited.
- **D043-045** (2026-11-14) `CheckedRunInput` has no `Default` and no `Deserialize`, by derive or impl; the fourth
  architecture rule fails on either.

### Fifth draft, the supported subset (2026-11-14, OD-2026-11-14-14, after the review at `72331b38`)

- **D043-046** (2026-11-14; the pattern rule refined by D043-054 and D043-055) The subset of OD-2026-11-14-14, with the same meaning for `$schema` absent, draft-07 and
  2020-12 (any other `$schema` refused): `type`, `properties`, `required`, `items` (one schema), `enum`, `const`,
  `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum` (numbers), `multipleOf` (a positive integer), `minLength`,
  `maxLength`, `minItems`, `maxItems`, `minProperties`, `maxProperties`, `pattern` (the `regex` crate, 64 KiB limits, at
  most 32, without `\d`, `\w`, `\s`, `\b`), `additionalProperties` (boolean or schema), `allOf`, `anyOf`, `oneOf`, a local
  `$ref` (a JSON pointer, no cycle, no sibling keyword but annotations), `$defs` and `definitions`; ignored: `title`,
  `description`, `default`, `examples`, `$comment`, `format`, `$schema`, a root `$id`; and `x-sagat-classification`,
  the only `x-` keyword the platform reads in a schema (`crates/infra-ir/src/carrier_schema_check.rs:8`). **Left out:**
  `patternProperties`, `uniqueItems` and `not`. Each could be bounded by the counter (a charge per key and pattern;
  n·log n; one application), but each adds semantics the oracle test must cover, the platform's own documents need none,
  and adding one later breaks no published schema. Every other keyword is out as well.
- **D043-047** (2026-11-14; the charges superseded by D043-058, the nesting limit by D043-057) A validator of the platform, `InputSchemas` in `crates/infra-schema/src/run_input/`, over
  `serde_json::Value`, with a work counter: one unit per keyword applied at an instance node and per element or member
  visited, the length in bytes plus one per `pattern` match, the length in code points per length check, the size of the
  compared values for `enum` and `const`, and a nesting limit of 128 schema applications. It stops at
  `RUN_INPUT_WORK_UNITS` (1,000,000) with `workLimit`, which is final the first time on every row. The counter counts what
  the validator does, so no static estimate is made. `jsonschema` is a test oracle only on this path.
- **D043-048** (2026-11-14; refined by D043-054 to D043-057) The pre-check, at publication, at start (cached) and in the inventory: at most 64 KiB and a
  depth of 32; every keyword in the vocabulary; every `$ref` a local JSON pointer, decoded once by the validator's own
  resolver (RFC 6901 with percent-decoding), that resolves; no cycle in the `$ref` graph; every pattern within its syntax
  and size. The first violation is `Unusable { keyword, at }`. There is no walk.
- **D043-049** (2026-11-14) The validator names its own rule (the subset's keywords, `workLimit`, `maxDepth`, `other`) and
  builds its own pointer: a segment for a member matched through `properties` and an index matched through `items`,
  nothing under `additionalProperties`; `required` appends the missing name. No library error is read or formatted.
- **D043-050** (2026-11-14; the per-tenant cap restored by D043-053) Simplified, because the counter makes a budget miss deterministic: removed the per-tenant
  permits and their eviction rule (D043-034, D043-044), the miss memory, the tick-identity key, the two clocks and the
  second-miss threshold (D043-035, D043-043), and `spawn_blocking` (D043-022). Kept: four permits per process, acquired
  within 5 s; the work through `block_in_place` holding its permit; a backstop in the counter that reads the clock every
  4,096 units and stops past 5 s. A permit not obtained and a backstop stop are transient (`503`, or a retry of the
  platform), never cached and never final.
- **D043-051** (2026-11-14; the issue codes completed by D043-065) Publication runs the pre-check on every `inputSchema`: in `enforce` a violation refuses with a
  typed issue (`INPUT_SCHEMA_KEYWORD_UNSUPPORTED`, `INPUT_SCHEMA_REFERENCE_INVALID`, `INPUT_SCHEMA_TOO_LARGE`) that names
  the keyword and its pointer; in `report` it is a warning with the same fields.
- **D043-052** (2026-11-14; the informational section and the exit codes by D043-072) The inventory is `control-plane input-schema-inventory`, run with the owner's credentials as
  `migrate` is: a paged read of `workflow_versions`, the pre-check on each stored schema, one line per version that would
  be refused (ids, keyword, pointer), no schema text, nothing written. It replaces the release-notes query, which could
  not tell whether a schema would be refused.

### Sixth draft, after the fifth review (2026-11-14, at `058ba9d3`)

- **D043-053** (2026-11-14; superseded by D043-061) A tenant holds at most two of the four permits. The gate counts permits in flight per tenant
  in a map under a mutex (an entry leaves when its count is zero, so nothing in use is evicted) and refuses a tenant at
  its cap at once (`Unavailable`, transient), without waiting; the global permit is then awaited for at most 5 s.
- **D043-054** (2026-11-14; the charge, the state cap and the cache superseded by D043-063 and D043-064) Pattern cost. When the lazy DFA gives up, the `regex` crate falls back to an NFA simulation
  whose cost per byte grows with the NFA's states, and a match cannot be interrupted. So each pattern is capped at
  compile (a 16 KiB program, 2,048 NFA states, a 64 KiB `hybrid_cache_capacity`; 32 patterns and 256 KiB of programs per
  schema), and a match is charged (bytes + 1) × ⌈NFA states / 64⌉ **before** it runs; a charge past the budget stops
  with `workLimit` without matching. A test with a pattern that defeats the DFA (`[ab]*a[ab]{12}`) against 256 KiB
  checks the cost stays within a constant multiple of the charge. `pattern` stays in 0.3.0 under these bounds.
- **D043-055** (2026-11-14; refined by D043-066) Pattern syntax is checked on the parsed `regex-syntax` AST against an allowlist (literals,
  escaped metacharacters, `\n`, `\r`, `\t`, `\xHH`, bracket classes of literals and ranges, `.`, `^`, `$`, alternation,
  capturing and non-capturing groups, repetitions with bounds of at most 100), never on the text. Refused: every Perl
  class and its negation (`\d\D\w\W\s\S`) and the word boundaries (`\b\B`), inside brackets too; `\p{...}`; POSIX classes;
  flags such as `(?i)`; `\A`, `\z`; named groups; backreferences and lookaround. `.` compiles to "any character but `\n`,
  `\r`, U+2028, U+2029", its ECMA-262 meaning.
- **D043-056** (2026-11-14) Pointer decoding for `$ref`: the fragment is percent-decoded (a bad `%` sequence or bytes that
  are not UTF-8 are refused), split on `/`, then in each token `~1` becomes `/` before `~0` becomes `~`; a `~` followed by
  anything else is refused, and an array index with a leading zero is refused. The target must be a schema position
  recorded by the vocabulary walk; `#/enum/0`, `#/default`, `#/examples/0`, an `x-sagat-classification` value and a
  container such as `#/properties` are refused.
- **D043-057** (2026-11-14; the stored path and the cost count by D043-062) The compiled form is an arena of nodes, one per schema position, and a `$ref` is the index of
  its target, never an inlined copy, so a reference DAG stays the size of the document. A depth-first search with white,
  grey and black marks over every application edge refuses a cycle at a grey node and computes the longest application
  path; a path longer than 128 is a schema defect, refused at publication (`INPUT_SCHEMA_TOO_DEEP`) and listed by the
  inventory, not a `workLimit` of the input.
- **D043-058** (2026-11-14; the pattern charge by D043-063; whole-validation stop by D043-067) Every loop step and every subschema application costs at least one unit: an application
  (`{}` included) and each `$ref` followed, each `required` name, each `type` name, each `allOf`/`anyOf`/`oneOf` branch,
  each instance member's `properties` lookup and `additionalProperties` decision, each `items` element; `enum` one per
  member plus the smaller of the two sizes; `const` the smaller size; a length check its length; a pattern match per
  D043-054. An adversarial test per keyword asserts primitive operations ≤ k × units.
- **D043-059** (2026-11-14) Determinism: vectors in document order and sorted lookup tables, no `RandomState` iteration;
  instance members in `serde_json::Map` order (a `BTreeMap`, since `preserve_order` is not enabled). Every replica reports
  the same first violation.
- **D043-060** (2026-11-14) The oracle test skips `workLimit`, `maxDepth` and backstop verdicts and fails when they pass 5%
  of the cases; it generates i64 values, u64 values above `i64::MAX`, integral floats, `-0.0` and integers beyond u64. The
  validator compares numbers by value, not with `serde_json`'s `PartialEq` (`1 != 1.0` there). Integers beyond u64 are
  parsed to f64 by both validators, since `arbitrary_precision` is off, so both round them the same way.

### Seventh draft, after the sixth review (2026-11-14, at `9fb12386`)

- **D043-061** (2026-11-14; superseded by D043-068) Per-tenant fairness waits instead of refusing: a `tokio::sync::Semaphore(2)` per tenant, in a map
  whose entry leaves only when the map holds its only `Arc` and both permits are free, is awaited before the global
  permit, within the same 5 s deadline; both are owned guards, so a future dropped while it waits releases its tenant
  permit. Refusing at once (D043-053) made a single tenant on an idle process get `503` for its own third concurrent start.
- **D043-062** (2026-11-14; the count superseded by D043-069) The marked DFS stores, when a node turns black, its longest application path and its
  input-independent application count (itself, plus its `$ref` target and its `allOf`/`anyOf`/`oneOf` branches), so the
  pass is linear. A count past `RUN_INPUT_WORK_UNITS` is refused at publication (`INPUT_SCHEMA_TOO_COSTLY`) and listed by
  the inventory, so a ×2 DAG of depth 20 cannot be published and then refuse every input.
- **D043-063** (2026-11-14) A `pattern` match is charged (bytes + 1) × NFA states before it runs; no bit-parallel divisor
  is assumed, because the PikeVM and the backtracker work per state per byte and the lazy DFA may never be built. A smaller
  divisor only if the T043-2 measurement sets it (G043-017). The state cap is lowered from 2,048 to 512 so that realistic
  patterns stay affordable: a 64-character string against 512 states costs about 33,000 units; a long string stops at the
  budget, and length checks run before `pattern` in a subschema.
- **D043-064** (2026-11-14) Every regex runtime engine is bounded explicitly: forward and reverse lazy DFA at 64 KiB of
  `hybrid_cache_capacity` each, the bounded backtracker off (its visited set is 256 KiB by default), the PikeVM cache
  measured once by `Cache::memory_usage()` after a warm-up. A cache entry weighs its programs plus four times that
  per-pattern cache size.
- **D043-065** (2026-11-14) The publication issue codes are six: `INPUT_SCHEMA_KEYWORD_UNSUPPORTED`,
  `INPUT_SCHEMA_REFERENCE_INVALID`, `INPUT_SCHEMA_PATTERN_UNSUPPORTED`, `INPUT_SCHEMA_TOO_LARGE`, `INPUT_SCHEMA_TOO_DEEP`,
  `INPUT_SCHEMA_TOO_COSTLY`, published in `specs/043-run-input-validation/contracts/errors.md` and in OpenAPI (T043-14).
- **D043-066** (2026-11-14) The pattern allowlist also refuses a class set operation (`ClassSet::BinaryOp`: `&&`, `--`,
  `~~`) and a nested bracket, which ECMA-262 reads as literals.
- **D043-067** (2026-11-14) A budget stop ends the whole validation with `workLimit`; it is never the failure of one branch.
  `oneOf: [cheap match, costly match]` is refused, not accepted.

### Eighth draft, after the seventh review (2026-11-14, at `846c570f`)

- **D043-068** (2026-11-14; placed in the adapter by D043-071) Fairness is work-conserving. Under one mutex the gate keeps the permits each tenant holds and a
  FIFO queue of waiters; a free permit goes to the waiter whose tenant holds the fewest (FIFO among equals), and a tenant
  may hold more than two only while no other tenant waits. A lone tenant uses all four; a newcomer is served first as
  permits return; nothing held is taken away. A waiter's guard leaves the queue or returns its permit when the future is
  dropped; the deadline stays 5 s. The `Semaphore(2)` of D043-061 left two permits idle while a lone tenant's third
  start waited and could get `503`.
- **D043-069** (2026-11-14) The publication count is a lower bound in units of what any input pays, stored per node when it
  turns black: the application, its `$ref` target's bound, one unit per `type` name and per `enum` member, one for
  `const`, the sum over `allOf` and `oneOf` branches, the minimum over `anyOf` branches; every sum saturates. A root bound
  past the budget is `INPUT_SCHEMA_TOO_COSTLY`. Counting applications instead let a ×2 `allOf` tree of depth 10 over a
  1,000-member `enum` publish and then refuse every input; summing `anyOf` over-counted; a plain sum could overflow.
- **D043-070** (2026-11-14) The pattern allowlist refuses a `]` as the first item of a class (`[]a]`) and `[^]`: ECMA-262
  reads `[]` as an empty class and `[^]` as any character, `regex-syntax` reads both as classes holding `]`.

### Ninth draft, after the architecture review (2026-11-14, at `c1cedefa`)

- **D043-071** (2026-11-14; the port's shape by D043-076; the defaults are constants of the adapter) Option (b) of the review: `crates/application` stays free of a runtime (tokio remains its
  dev-dependency, `crates/application/Cargo.toml:30-32`). `SchemaValidator::check` is an async port; the permits, the
  work-conserving fair queue, the wait, `block_in_place` and the backstop are in `crates/infra-schema`, which gains tokio
  (`sync`, `time`, `rt-multi-thread`) as a normal dependency and its own `off_worker` (the one in
  `crates/infra-postgres/src/payload_disclosure.rs:561` is private). The port adds `SchemaRefusal::Busy`. The defaults
  (four permits, 5 s) are constants of the adapter.
- **D043-072** (2026-11-14) The inventory lists, besides the versions that would be refused, an informational section of
  versions within the subset with an `inputSchema` and a non-manual trigger (G043-004), which does not change the exit
  status. Exit 0: nothing would be refused; 1: at least one version would; 2: the command failed.
- **D043-073** (2026-11-14) The publication mode reaches `infra-ir` as a parameter: the control plane passes its process's
  mode; `sagat validate` takes `--input-validation enforce|report`, `enforce` by default.
- **D043-074** (2026-11-14) A fifth architecture rule rejects `jsonschema` (a dependency path, a `use` or a qualified path)
  under `crates/infra-schema/src/run_input/`, with a negative fixture; `jsonschema` stays only as the oracle of the tests and
  for the scripts of feature 005.
- **D043-075** (2026-11-14) The digest of `InputSchema` is computed by `infra-ir` at parse; the domain stores a `[u8; 32]`
  (`sha2` is not in `DOMAIN_ALLOWED`).

### Tenth draft, after the architecture review at `d7577f18` (2026-11-14)

- **D043-076** (2026-11-14) `SchemaValidator::check` returns `Pin<Box<dyn Future<Output = ...> + Send + 'a>>`, so the trait is
  object-safe and held as `Arc<dyn SchemaValidator>`, as the checker it replaces is (`crates/application/src/runs.rs:822`).
  `RunInputGate` is not generic; creators hold an `Arc<RunInputGate>`, so no type parameter spreads through the thirteen
  creators. The wait and the backstop are the adapter's constants `RUN_INPUT_PERMIT_WAIT` and `RUN_INPUT_BACKSTOP`, 5 s
  each; the adapter does not read an `application` constant for them.
- **D043-077** (2026-11-14) A sixth architecture rule fails when `tokio` is a normal (not a dev) dependency of
  `sagat-application`, with a negative fixture, so the layering of D043-071 holds.

### Phase 1 of the build, T043-1, T043-2, T043-13 and T043-24 (2026-11-15, on `feature/043-validator`)

- **D043-080** (2026-11-15) A boolean is a schema at every schema position, as both drafts read it: `true` accepts, `false`
  refuses with `rule` `other` (with `additionalProperties` when it is the value of that keyword). The root of an
  `inputSchema` is an object, as the IR requires.
- **D043-081** (2026-11-15) Only a new version is refused for an `inputSchema` outside the subset. A parse of a document
  that is already a version (the replay of a publication, an activation, the "already active" check of a promotion, the
  digest of a draft compared with a version) runs in `report` mode, so a version published in `report` mode stays
  reachable and a replay never turns into an internal error. A promotion into another namespace publishes a new version
  and is checked in the mode of the service.
- **D043-082** (2026-11-15) The strings of the oracle property test have no `\r`, U+2028 or U+2029: `jsonschema` 0.58.6
  hands `.` to the `regex` crate unchanged (any character but `\n`), while the subset compiles it with its ECMA-262
  meaning. The meaning of `.` against the four line terminators is a unit test of the validator instead.
- **D043-083** (2026-11-15) The forms the subset accepts for its keywords: the lengths and counts are JSON integers
  (`as_u64`, so `2.0` is refused); `multipleOf` is an integer in 1..=2^53 (past 2^53 an `f64` no longer holds every
  integer); `required` is a list of distinct strings; `type` a name or a non-empty list of distinct names; `allOf`, `anyOf`
  and `oneOf` non-empty lists; `title`, `description`, `$comment` and `format` strings and `examples` a list, as the
  meta-schemas say. A keyword of the subset in another form is `INPUT_SCHEMA_KEYWORD_UNSUPPORTED`, named with its pointer.
- **D043-084** (2026-11-15) A JSON depth past 32 is `INPUT_SCHEMA_TOO_DEEP` (with the key and the pointer of the first
  value too deep), as an application path past 128 is (at the node whose path passes it, the root for a chain that starts
  there). Cycles and paths are checked over every node of the arena, an unused definition included. A cycle is
  `INPUT_SCHEMA_REFERENCE_INVALID` at the first `$ref` on it.
- **D043-085** (2026-11-15) The lower bound of D043-069 is a bound on what every input the node **accepts** pays: a refused
  input may stop at its first violation (an `allOf` stops at its first failing branch, a node at its first failing
  keyword). `INPUT_SCHEMA_TOO_COSTLY` therefore says that no input can be accepted within the budget. So that it holds,
  `enum` charges its members and `type` its names before comparing, and every branch evaluated of `allOf`, `anyOf` and
  `oneOf` costs one unit beside its application.
- **D043-086** (2026-11-15) Two charges the design did not list: `enum` and `const` measure the input's size only up to
  the largest member (a walk that costs no more than the comparisons it bounds), and the numeric bounds, `minItems`,
  `maxItems`, `minProperties` and `maxProperties` are constant-time checks paid by the application's unit. A violation
  under `additionalProperties` names the object and the rule `additionalProperties`, never what failed beneath: a
  pointer below would start at a key the caller chose.
- **D043-087** (2026-11-15) The pre-check runs in `IrCodec::parse_with(document, mode)` after the conversion, on the
  bytes the definition carries; `parse` is the `enforce` mode, `parse_private` runs it in `enforce`, `parse_stored` and
  `parse_stored_private` never. Every other codec keeps the default `parse_with`, which ignores the mode.
- **D043-088** (2026-11-15) The refusal is a typed `IrError::InputSchema { issue, keyword, path }` rather than new fields
  of `IrIssue`: `problems_of` answers the issue's closed code and its pointer, the API answers `reason` `SCHEMA` with one
  issue that carries `code`, `keyword`, `path` and the message, and the `report` warning is
  `DefinitionWarning::InputSchemaOutsideSubset` with the same fields. The issue codes are `InputSchemaIssue` of the
  domain; the trigger warning is `DefinitionWarning::InputSchemaAppliesToTriggers` (`INPUT_SCHEMA_APPLIES_TO_TRIGGERS`).
- **D043-089** (2026-11-15) The publication mode is a parameter of `DefinitionService` and `PublishingService`
  (`with_input_validation`), `enforce` by default. Reading `SAGAT_RUN_INPUT_VALIDATION` in the control plane and passing it
  there is left to T043-12, with the strict parse, the capability and the engine's twin, so that one change gives the
  variable its whole meaning (G043-021).
- **D043-090** (2026-11-15) The pattern charge keeps no divisor (D043-063). Measured on an Apple M3 Max in release, a match
  that defeats the lazy DFA costs 0.086 ns per (byte × state), below every other unit measured (the dearest, a
  `required` lookup among 6,000 names, is 63 ns): `(bytes + 1) × states` over-counts by far, and is kept as the
  conservative choice (G043-017).
- **D043-091** (2026-11-15) The compiled-schema cache of the classification carriers is generic over its key, value,
  failure, bounds and metric names; the carriers keep theirs (`classification_cache_total{outcome}`). The run-input
  instance keys by `(tenant, digest)`, keeps 256 entries, 64 MiB and 1,024 remembered `Unusable`, and counts
  `run_input_schema_cache_total{result}` with `hit`, `miss`, `evicted`, `failure_remembered` and `unshared`. An entry weighs
  8 bytes per schema byte, its nodes, and for each pattern its programs plus 5 × (its PikeVM cache after a warm-up + 2 ×
  64 KiB of lazy DFA).
- **D043-092** (2026-11-15) The inventory's integration test is
  `tests/integration/gitops-versioning/input_schema.rs` (real PostgreSQL through the shared Testcontainers support),
  not `apps/control-plane/tests/inventory.rs`, which has no database fixture; the exit status 2 is tested through
  `inventory::exit_code`, the function `main` uses. A stored document that is not a JSON object is listed as
  `unreadable` and does not fail the command.
- **D043-093** (2026-11-15) `sagat validate` and `sagat workflow validate` take `--input-validation enforce|report`
  (`enforce` when absent, anything else a usage error); the CLI's help catalog and `docs/quickstarts/errors.md` explain
  the seven codes, which the CLI's consistency test lists among those returned through a closed table of the domain.
- **D043-094** (2026-11-15) The primitive operations the adversarial tests hold to `k × units` (`K` = 32 in the test) are
  counted under the `test-support` feature of `sagat-infra-schema`: one per application, per `type` name, per node
  measured or compared (plus one per 64 bytes of a string compared), five per number checked, one per code point of a
  length check, one per byte a pattern scans, and `log2(n)` per lookup among `n` names. It is a model of the work that
  shows proportionality; the time of a unit is measured separately (G043-017).

- **D043-095** (2026-11-15) Two existing tests of `crates/infra-ir` published `inputSchema`s outside the subset and are
  changed with the rule, never the check loosened: `production_compiles_annotated_root_schemas_but_preserves_unannotated_legacy_roots`
  now expects the unannotated root `{"type": "not-a-json-schema-type"}` refused by a new publication in `enforce`,
  published in `report` and restored from storage; the Iceberg egress tests of `iceberg_egress.rs`, whose records schemas
  use `prefixItems`, publish in `report` mode, since the levels a write sends out are read the same in either mode. An
  Iceberg write whose input schema has tuple positions is therefore publishable in 0.3.0 only in `report` mode
  (OD-2026-11-14-14).


### After the review of phase 1 by the security and correctness reviewers (2026-11-15)

- **D043-100** (2026-11-15) Measuring the input for `enum` and `const` stops before a push that would bring what it
  counted and what waits on its stack to the cap, and the steps of the walk are charged: a long array under
  `{"const": null}` was walked whole for free (about 4 to 10 s behind an `anyOf` fan-out, ending in a transient backstop
  where a final answer was due).
- **D043-101** (2026-11-15) Supersedes the `oneOf` term of D043-069: the lower bound counts the cheapest `oneOf` branch,
  plus two units (its branch and its application) for each other branch, which can fail at its first keyword. The sum
  refused good schemas with `INPUT_SCHEMA_TOO_COSTLY`.
- **D043-102** (2026-11-15) A name of `properties` or `required` is at most 1 KiB (`MAX_NAME_BYTES`), refused past it with a
  seventh issue code, `INPUT_SCHEMA_NAME_TOO_LONG` (the keyword and the pointer of the map, or of the `required` entry). A
  lookup is charged `log2(n) × (1 + length / 64)` units, the comparisons it makes and the bytes each compares (the name's
  length for `required`, the shorter of the key and the longest declared name for `properties`); one unit per lookup
  let long shared prefixes behind a fan-out compare gigabytes within budget.
- **D043-103** (2026-11-15) The oracle also generates the u64/f64 boundary pairs (`18446744073709551615` and
  `1.8446744073709552e19`, `9223372036854775809` and `9.223372036854776e18`, `±2^63`) and nodes that hold both `properties`
  and `additionalProperties`. Its runs are recorded in `validation-report.md`.
- **D043-104** (2026-11-15) The depth check knows the data keywords (`default`, `examples`, `const`, `enum`,
  `x-sagat-classification`): a value too deep inside one is reported with that keyword and its pointer, never with the
  keys of the data. Every `Unusable` pointer is truncated to 512 characters, ending in `…` (`MAX_POINTER_CHARS`).
- **D043-105** (2026-11-15) The weight of a compiled schema also counts the heap of its literals (a `Literal` and its
  value: a `Value` per element and member, the bytes of strings and keys with an allocation's overhead), its names and its
  lists. One tenant holds at most a quarter of the run-input cache (`TENANT_SHARE`), by count and by weight: an entry of a
  tenant at its share evicts that tenant's own oldest, and an entry heavier than the share is compiled for its call and
  not kept. The classification cache has no share and is unchanged.
- **D043-106** (2026-11-15) A compile that panics answers the callers that waited for it `CompileFailure::Interrupted`,
  which the check maps to `SchemaRefusal::Interrupted`: transient, never remembered, like the backstop. It used to answer a
  final `KeywordUnsupported`.
- **D043-107** (2026-11-15) `RUN_INPUT_PERMITS` (4) is the constant the gate's permits of T043-3 are built on; a
  compile-time assertion of `pattern.rs` holds the matchers a pattern's weight counts (`1 + CONCURRENT_MATCHERS`) to at
  least that many. The weight is right only while every match runs under a permit (G043-031).
- **D043-108** (2026-11-15) `multipleOf` on an integral float computes the remainder as `(mantissa mod d) × (2^exponent mod
  d) mod d`, exact and logarithmic in the exponent. IEEE `%` is exact too, but measured 1.16 µs for 2^996 by 3 (release,
  Apple M3 Max), about 150 times the dearest unit; behind a fan-out it is now 8.7 ns per unit.
- **D043-109** (2026-11-15) The publication half of T043-12: the control plane reads `SAGAT_RUN_INPUT_VALIDATION` strictly
  (unset or empty is `enforce`, `enforce` and `report` are themselves, anything else stops the process naming the
  variable), logs it, and passes it to `DefinitionService` and `PublishingService`. The registry capability is left to
  phase 2. No manifest under `infra/` or `examples/` sets `report` (`no_shipped_manifest_sets_run_input_validation_to_report`).
  Closes G043-021.
- **D043-110** (2026-11-15) Refines D043-095: the Iceberg egress tests publish in `enforce` whatever is inside the subset,
  and fall back to `report` only when `enforce` refused the document for `prefixItems`.
- **D043-111** (2026-11-15) The inventory exits 2 also when a stored document is not a JSON object (it cannot say what that
  version would do); it still lists what it could read. Its `keyword` and `at` fields are printed in double quotes, with
  `"` and `\` escaped. Its cursor starts after the nil uuid, in one statement.
- **D043-112** (2026-11-15) G043-020 is written down: `docs/quickstarts/errors.md`, the design (Technical Plan 8 and
  T043-14, for the OpenAPI description) say that publication answers `CLASSIFICATION_TOO_COMPLEX` for a schema over
  64 KiB or nested deeper than 32.
- **D043-113** (2026-11-15) Refines D043-094: a lookup counts as many primitive operations as it is charged units (D043-102),
  and the size walk counts its real steps (D043-100), so "operations ≤ k × units" is a check of the charges. The time per
  charged unit of a pattern at the state cap is measured separately (G043-030).

### Phase 2 of the build, the gate and the creators (2026-11-15, on `feature/043-creators`)

- **D043-120** (2026-11-15) A service built without a gate holds `RunInputGate::unwired()`, whose port answers `Busy` for
  every schema: a start of a version with an `inputSchema` is refused as transient (`503`, or tried again by the engine)
  and never admitted unchecked; a version without one passes as it would through a wired gate. The composition roots and
  the test support wire `RunInputValidator::gate(mode)`, a gate over `RunInputValidator::process()`, one cache and one
  pool of four permits per process however many services hold it.
- **D043-121** (2026-11-15) Every repeat (a rerun, a batch rerun, a reset, a pending reset) answers `field` `/` and `rule`
  `other`: the gate is given `FieldPolicy::Root` for them. Whether the caller's view of the source input under feature
  016's graded disclosure withholds anything is not computed on the start path (G043-040); the answer never describes a
  withheld value, at the cost of saying less than D043-037 allows to a caller who may read the source.
- **D043-122** (2026-11-15) The legacy-continuation rule of the architecture check lets a `.continue_as_new(` call stand
  inside a function itself named `continue_as_new`: the `ChainProgressStore` implementation of `PgRunStore` forwards to
  `PgChainProgressStore`. Any other call outside the tests fails.
- **D043-123** (2026-11-15) The reason a schedule action records for a refused input: `RUN_INPUT_SCHEMA_UNUSABLE` for a
  schema outside the subset, `NOMINAL_TIME_FIELD_INVALID` when the refusal's pointer is the one of `nominalTimeField`,
  `SCHEDULE_INPUT_INVALID` for anything else.
- **D043-124** (2026-11-15) Trigger-now answers the pointer into the action's input as it is (`/orderId`), since its request
  carries no input; the schedule writes prefix it with `/action/input`.
- **D043-125** (2026-11-15) The write check of row 14 needs the gate and a definition cache
  (`ScheduleService::with_run_inputs`); without them a write is checked only at action time, which is the authority
  (D043-008). It reads the target version's document (`ScheduleStore::target_version`, one statement on the write path)
  and fills `nominalTimeField` with the write's instant, RFC 3339 to the second like an action's, rather than computing the
  next nominal time.
- **D043-126** (2026-11-15) The check before `decide` of rows 6 to 8 is `ScheduledStarts::check_scheduled`: the run service
  resolves the version and its definition as a start does and runs the gate. The start then checks again (the compile is
  cached). A refusal at the start after the check passed (a buffered action released into a version promoted meanwhile)
  records `INPUT_INVALID` through the same decision, and the waiting row is settled where it stands: `settle_waiting`
  accepts `INPUT_INVALID` beside the business-id outcomes. Red first: `a_buffered_action_is_checked_again_at_its_release`
  failed with "the schedule already acted on that time" before that line.
- **D043-127** (2026-11-15) The port's types (`Rule`, `PathSegment`, `SchemaRefusal`, `Unusable`, `pointer_of`) live in
  `crates/application/src/run_input.rs` and `crates/infra-schema` re-exports them: one shared type at the boundary
  (AGENTS.md rule 6). `SchemaRefusal` gains `Busy`.
- **D043-128** (2026-11-15) The audit records: in `report` mode, `run.input_refused` as a follow-up of the run's own record,
  on the run, reason `reported: <reason> rule=<rule>`; for a trigger, on the workflow version, reason
  `<reason> rule=<rule> via trigger <id>`, and for a cron tick `... at tick <key>`. None names a field or a value.
- **D043-129** (2026-11-15) `trigger_events_total{outcome="input_invalid"|"input_schema_unusable"}` counts once per dispatch
  (CUJ 3); the delivery and the audit are written once.
- **D043-130** (2026-11-15) The six SDK errors carry `field` whenever the server's answer names one, a schedule spec or a
  start option included (the Rust SDK test that said `details.field` is not carried changed with `sdks/SPEC.md`). The
  conformance runners record `field` only beside `rule`, so the goldens of the other scenarios are unchanged.
- **D043-131** (2026-11-15) The migration names the widened constraints `*_known`, and also widens
  `platform_requirement_history_capability_valid`, which the design did not list; its down migration refuses while a
  history row names the capability.
- **D043-132** (2026-11-15) Before failing a parent step (D043-038) the engine looks the child up with
  `RunStore::replay_start` under the child's own idempotency (`child:{parent}`, `child-start:{node}`, the reserved child
  id), as `system:engine`: one statement on the refusal path only.
- **D043-133** (2026-11-15) A continuation whose input cannot be checked now is `ContinuationAdvanceError::InputUnavailable`,
  counted as a wait (`scheduler_continuation_waits_total{blocker="input_check"}`); a refused one ends the predecessor through
  the fenced path of `finalize_continuation_expression`, now given the failure, with
  `scheduler_continuation_input_refusals_total`.
- **D043-134** (2026-11-15) A refused backfill step ends the backfill `CANCELLED` and does not pause the schedule; a refused
  scheduled or triggered action pauses it when `pauseOnFailure` is set.
- **D043-135** (2026-11-15) The 422 text of each start route, the schedule writes, trigger-now and the publication is not
  written into their OpenAPI operations: the operation definitions outside the additions of features 016 and 035 are
  pinned byte for byte (`contract_payload_classification`, `contract_iceberg_writes`, which are never blessed). The
  contract carries design 043 in the shared `Error` schema instead (the `details.reason` list, `details.rule` as a closed
  enum, `details.field`), in the schedule action outcome enum, and in `specs/043-run-input-validation/contracts/errors.md`
  (G043-045).

### After the review of phase 2 (2026-11-15)

- **D043-140** (2026-11-15) The waiter's guard owns the receiver of its grant, and `dispatch` no longer takes a grant back:
  a release between the end of a wait and the drop of its guard was counted twice, so the pool grew past four
  (security review). Red first: `a_release_in_the_gap_of_a_timed_out_waiter_is_not_counted_twice` (free 2 of 1).
- **D043-141** (2026-11-15) A final refusal whose key holds a run goes through the `Admitted::Replayed` branch of
  `finish_start` (with-start receipts, "already acted", "already completed"), for `start_with` and `start_private`
  (correctness M1). Red first: the four `*_the_gate_refuses_*` tests (500, `[]` delivered, `Ok` twice).
- **D043-142** (2026-11-15) An exhausted schedule (`remaining - reserved == 0`) is not checked: `decide` answers it
  `Exhausted` (correctness M2). Red first: `an_exhausted_schedule_records_no_input_invalid_*` (3 `INPUT_INVALID`).
- **D043-143** (2026-11-15) `continue_evaluated` writes the `report` audit in the hand-off transaction. Red first.
- **D043-144** (2026-11-15) Trigger-now answers `/` and `other`: the stored input of a schedule comes out only through the
  audited read of FR-018-042, which trigger-now is not; the disclosure check itself is not evaluated there. Red first.
- **D043-145** (2026-11-15) The migration is split: `20270118000000` adds the CHECKs `NOT VALID`, `20270118000100`
  validates them and drops the old ones; a static test keeps every later migration so. Red first.
- **D043-146** (2026-11-15) A manual admission refused at its start (a version promoted after the check) returns the
  refusal and records nothing. Red first: it answered 200 with an `INPUT_INVALID` action.
- **D043-147** (2026-11-15) `RunInputGate::recheck` checks without counting: a schedule's action is counted by its check
  before the decision, the buffered release included (now checked before it starts), and not again at the start. Its
  test was written with the new method and was not seen red.
- **D043-148** (2026-11-15) A private rerun (row 13) answers `/` and `other`. Red first.
- **D043-149** (2026-11-15) OD-2026-11-15-3: the six start operations (`startRun`, `signalWithStart`, `updateWithStart`,
  `operateRun`, `resetRun`, `startDatabaseExecution`) describe their `422`. The baselines are re-pinned this once after
  reading the diff: in the 035 fixture only the three digests it pins that changed (the bless would have added every
  later operation, which was not done), and the 016 digest with its comment. Closes G043-045.

- **D043-150** (2026-11-15) Design 042 gives `sagat-application` `tokio` with `rt` alone, for one blocking parse of a blob
  source (`collection_source.rs`). The runtime rule of D043-077 now allows exactly that (no default features, no feature
  but `rt`) and also refuses any `tokio` in `run_input.rs` outside its tests, which is what D043-071 protects (G043-051).
- **D043-151** (2026-11-15) Any refusal, a transient one too, first looks the key up: a retry with no room in the validator,
  whose key holds a run, is answered with that run as before design 043 (re-review). Red first:
  `a_retry_with_no_room_replays_the_run_its_key_holds` (503).
- **D043-152** (2026-11-15) A recheck counts its refusal (`refused`, `schema_unusable`), and runs in the same
  `run_input.check` span as a check. Red first: `a_refusing_recheck_is_counted`.
- **D043-153** (2026-11-15) Each old CHECK is dropped in the `ALTER TABLE` that adds its `NOT VALID` successor (a
  superset), and the second migration only validates; every later CHECK must be added `NOT VALID` (static test).
- **D043-154** (2026-11-15, technical, the coordinator's call) The narrow exception of D043-150 stands: `sagat-application` may
  have `tokio` with no default features and no feature but `rt`, for design 042's blocking parse of a blob source's
  array, and `run_input.rs` names no runtime. Reason: two merged designs need it, and the gate's layering (D043-071) is
  what the rule protects. The way back is G043-052: move that parse behind a port of the adapter, then restore the
  stricter rule. Closes G043-051.
- **D043-155** (2026-11-15) The SQS failure of the runtime-role run (`SQS answered 404 Not Found: not found`, once, passing
  alone) is the test support's: `TestSqs` was used as soon as ElasticMQ printed its start line, before its routes
  answered. `sqs()` now asks `ListQueues` until it answers, at most 50 times. The cause is probable from the plain-text
  404 (ElasticMQ's own errors are XML), not reproduced.
### After the review run of 2026-11-15 at `ce2e10e2` (phase 3)

- **D043-156** (2026-11-15) A refused cron tick is audited in the transaction that moves its trigger, only when that move
  found the trigger still at the tick (`refuse_cron_tick`, a compare-and-set on `next_fire_at`), and the round's own
  advance then finds it moved. Supersedes the separate audit of D043-027 for a tick: with the audit first and the move
  in its own statement, a move that failed or a writer that stopped between the two left the tick due and every later
  round audited it again, and N dispatchers wrote N audits (correctness review of 2026-11-15, Major 1). Red first:
  `two_engines_on_one_refused_cron_tick_audit_it_once_with_the_tick_key` and
  `a_cron_refusal_whose_writer_stops_before_the_trigger_moves_is_audited_once_by_the_next_round`
  (`tests/fault/events-webhooks-messaging/run_input.rs`), 2 audits each before the change. The design's T043-7 named
  the test "at most twice"; its text now cites this decision. Closes G043-002.

## 3. Gaps

| ID | Gap | Reason / closure | Status |
|---|---|---|---|
| G043-001 | A control plane of 0.2.x running beside new engines during the upgrade meets the new outcome `INPUT_INVALID`. The action listing reads the outcome as a string by design (`crates/application/src/schedules.rs:634-635`, `crates/infra-postgres/src/schedules.rs:547`), so it passes through; the outcome filter parses the name strictly (`crates/api-http/src/routes/schedules.rs:275`) and an old binary answers `422` to a filter on `INPUT_INVALID`. | Read in the code, not yet tested: T043-11 adds the test of an action list with an unknown outcome. The filter case is stated in the release notes. Tested 2026-11-15: `actions::the_listing_passes_an_outcome_it_does_not_know_through_as_a_string` (`integration_schedules_bulk_operations`) lists an outcome no binary knows beside known ones, and a filter on another outcome leaves it out. The filter of an old binary on `INPUT_INVALID` stays a documented limit. | Open (listing tested; filter documented) |
| G043-002 | A trigger refusal could be audited again on every retry of its event. | Closed for inbox events by D043-027 (the delivery row), proven by T043-7. Left: two engines that reach the same cron tick at once can both audit it; the reason names the tick key, so the duplicate is recognisable. Closed 2026-11-15 by D043-156: the tick's audit is written with the move of its trigger, once (`two_engines_on_one_refused_cron_tick_audit_it_once_with_the_tick_key`, `a_cron_refusal_whose_writer_stops_before_the_trigger_moves_is_audited_once_by_the_next_round`). | Closed |
| G043-003 | `details.field` names property names declared in the schema to a caller with `run:start`. | Closed 2026-11-14: every role that holds `run:start` also holds `definition:read` (`crates/domain/src/identity.rs:607-691`), so the caller may read the schema the names come from. T043-23 adds the unit test that keeps it so for every `Role`. | Closed (by reading; test in T043-23) |
| G043-004 | A published version with a non-manual trigger and an `inputSchema` that describes a manual input (and not the event envelope `{event, data}`) refuses every triggered start after the upgrade. | `control-plane input-schema-inventory` lists these versions in its informational section, without changing its exit status (D043-072), and publication warns on new ones; neither can tell which schemas accept the envelope without real events. `report` mode covers the time to fix them. | Open |
| G043-005 | `format` is an annotation, so a schema that relies on it to refuse an input does not refuse it. | D043-046; documented in the runbook. | Open (documented) |
| G043-006 | The CPU cost of validating a 256 KiB input on the API path is not measured. | Counters, not timers, prove the compile count; a benchmark is not a target of this design. | Open |
| G043-007 | Inputs stored before the upgrade (schedules, pending resets) are not checked at the upgrade; they are refused at action time. | By design: the action time is the authority (D043-008). Stated in the release notes. | Open (documented) |
| G043-008 | Signals, Updates, worker results and activity outputs are not validated against a schema by this feature. | Out of scope: OD-2026-11-14-2 is about starts. | Open (out of scope) |
| G043-009 | `infra-schema::validate` (scripts, feature 005) reports instance paths that can hold caller-chosen keys. | Observed while reading the code; not changed here (another feature's contract). Reported to the owner. | Open |
| G043-010 | Until the operator records `run-input-validation-v1`, replicas may run different modes and a 0.2.x control plane admits what a new one refuses; the first upgraded engine runs `enforce` unless the manifest said `report` before the migration. | D043-040: the runbook sets the mode before migrating; the requirement can only be recorded once every live process is in `enforce` (ADR-0082 decision 2). | Open (documented) |
| G043-011 | A validation under load can be refused for a while. | Superseded 2026-11-14 by D043-050: only a permit not obtained or the backstop is transient, and a budget miss is deterministic and final. | Closed (by design) |
| G043-012 | A rerun or reset refused for its input answers `field` `/` to a caller who may not read the source run's input, which says less than the caller needs to fix it. | Accepted: the field names a property of a payload the caller may not see. | Open (accepted by design) |
| G043-013 | The miss memory of D043-035 was per process and evicted. | Removed by D043-050: there is no miss memory. | Closed (by design) |
| G043-014 | A published schema with a keyword outside the subset (recursion, `patternProperties`, `not`, `if`, `unevaluated*`, ...) refuses every start of its version in `enforce`. | OD-2026-11-14-14; `control-plane input-schema-inventory` lists them (D043-052); `report` mode covers the time to republish. | Open (documented) |
| G043-015 | `multipleOf` accepts only positive integers, so a schema with `multipleOf: 0.01` (a money amount) is outside the subset. | Floating-point remainders differ between implementations; an exact decimal rule can be added later. | Open (documented) |
| G043-016 | `pattern` refuses Perl classes and their negations, word boundaries, Unicode and POSIX classes, flags and named groups, which authors use often; they write `[0-9]` and the like instead. Characters are code points, as in the `jsonschema` crate, not the UTF-16 units of ECMA-262 without the `u` flag, so an astral character counts as one. | D043-055; the publication issue `INPUT_SCHEMA_PATTERN_UNSUPPORTED` names the pattern's pointer. | Open (documented) |
| G043-017 | The constant k per keyword (primitive operations per unit) and the pattern divisor (work per byte per state on this machine) are not measured; a full budget's time is not measured either. | Measured 2026-11-15 by `every_adversarial_case_stops_at_the_budget_with_operations_bounded_by_the_units` and `measure_k_and_the_cost_of_a_full_budget` (`crates/infra-schema/tests/run_input.rs`, release build, Apple M3 Max, 16 cores, rustc 1.100.0), each case stopping at `workLimit`. k (operations per unit, D043-094): `required` 12.98, `properties` 8.48, `items` 3.00, `allOf` 3.00, `anyOf` 2.33, `oneOf` 2.33, `type` 1.44, `const` 1.34, `minLength` 1.00, `additionalProperties` 0.67, `pattern` 0.06, `enum` 0.02; the test holds every case to k ≤ 32. Time per unit: `required` 63 ns, `properties` 36 ns, `const` 9.5 ns, `items` 4.1 ns, `additionalProperties` 3.5 ns, `allOf` 3.0 ns, `anyOf`/`oneOf` 2.7 ns, `type` 1.9 ns, `minLength` 0.36 ns, `pattern` (a 2 KiB string matched 28 times) 0.09 ns, `enum` 0.07 ns. **A full budget costs at most 63 ms** on this machine (the `required` case; 302 ms in the unoptimised test build). Pattern: `a[ab]{12}c` (17 NFA states, lazy DFA past its 64 KiB cache, prefilter off) scans 256 KiB in 0.38 ms, 0.086 ns per (byte × state): no divisor is set (D043-090). Not measured on the CI machine nor in production (G043-006). Re-measured 2026-11-15 after D043-100 and D043-102: a lookup is charged by its comparisons, so `required` costs 5.8 ns per unit (k 1.00) and `properties` 6.2 ns (k 1.42); the dearest unit is `const` at 7.5 ns and `multipleOf` on a huge float at 8.7 ns (D043-108), and **a full budget costs at most 7.5 ms** on this machine. | Closed (measured on the development machine) |
| G043-020 | At publication, an `inputSchema` past 64 KiB or nested past 32 is refused by the classification bound of feature 016 first (`CLASSIFICATION_TOO_COMPLEX`, `crates/infra-ir/src/classification_check.rs`), not with `INPUT_SCHEMA_TOO_LARGE` or `INPUT_SCHEMA_TOO_DEEP`; the start gate and the inventory answer the validator's own code. | The schema is refused either way; only the code differs. Seen in `every_issue_has_its_closed_code_and_an_unannotated_schema_is_checked_too`. Left as it is: changing the order would change what feature 016 answers. | Open (accepted) |
| G043-021 | The control plane publishes in `enforce` mode whatever `SAGAT_RUN_INPUT_VALIDATION` says: the parameter exists (D043-089) and nothing reads the variable yet. | Closed 2026-11-15 by D043-109: the control plane reads it strictly and passes it to publication; tests `the_run_input_validation_mode_is_read_strictly_and_enforce_by_default` and `no_shipped_manifest_sets_run_input_validation_to_report` (`apps/control-plane/tests/config.rs`). The registry capability is phase 2 (T043-12). | Closed |
| G043-022 | The oracle property test generates references only to `$defs` entries with plain names and strings from a small alphabet; percent-encoded and `~`-escaped pointers, and `.` against line terminators, are covered by unit tests, not by the oracle. | Regression check, not a proof (D043-060); the unit tests `a_reference_must_name_a_schema_position_of_the_document` and `the_pattern_allowlist_refuses_what_ecma_262_and_the_regex_engine_read_differently` cover them. | Open (documented) |
| G043-030 | Matching a pattern at the cap of 512 NFA states costs, per charged unit, what this machine measured and no more is known. | Measured 2026-11-15 (release, Apple M3 Max): with the production configuration, `a[ab]{12}c{100}d{100}e{100}f{100}g{50}` (466 states, the most a pattern reaches before its 16 KiB program) costs 0.06 ns per charged unit in the test build; the raw automaton with the prefilter off costs 0.0032 ns per (byte × state) at 466 states and 0.0144 at 225 states of classes, against 5.8 to 8.7 ns per unit for the dearest keywords. `a_pattern_at_the_state_cap_runs_its_charge_within_the_backstop` holds a whole budget of it within the 5 s backstop. Not measured on the CI machine. | Open (measured on the development machine) |
| G043-031 | The run-input cache bounds compiles with four permits, but a key that is not shared (an ad hoc start) compiles outside the cache and its permits, and a pattern's weight counts `1 + 4` matchers only while every match runs under one of the gate's permits. | Closed 2026-11-15: `RunInputValidator::check` takes one of the `RUN_INPUT_PERMITS` permits before it compiles or validates, shared or not, and every creator, row 13 included, calls it through the gate (`crates/infra-schema/src/run_input/mod.rs`). | Closed |
| G043-040 | Repeats answer `/` and `other` always (D043-121): the graded-disclosure check of D043-037, which would let a caller who may read the source input see the field, is not built. | Never describes a withheld value; says less than allowed. | Open |
| G043-041 | The check before `decide` of a schedule action reads the workflow and the version again and validates twice (D043-126). | The schedule path has no statement budget; the compile is cached. | Open (accepted) |
| G043-042 | Rows without an integration test of their own: the uploaded-input start (row 2), Update-with-Start (row 3), the batch `RERUN` item (row 4), reset and pending reset (row 5), the ad hoc start (row 13); and `RUN_INPUT_SCHEMA_UNUSABLE` on rows 3 to 5, 8 and 13. They go through the paths the tested rows use (`start_with`, `start_private`) and the mappings have unit tests (`a_rerun_refused_for_its_input_fails_with_validation_failed`, `a_pending_reset_refused_for_its_input_ends_admission_refused_and_no_room_is_retried`). | SC-043-001 is not yet met for these rows. | Narrowed 2026-11-15: row 13 (`tests/integration/database-connections/run_input.rs`), Update-with-Start (`an_update_with_start_whose_input_is_refused_applies_nothing_to_an_open_holder`), the replays of rows 3, 5 and 6 (D043-141) now have tests; what is left is in G043-050. Open |
| G043-043 | The lookup of an existing child before a parent step is failed (D043-038, D043-132) has no integration test: it needs a child created by a peer between the reservation and the check. | Built; untested. | Closed 2026-11-15: `Scheduler::with_before_child_check` lets a test create the child where a peer would; `a_child_a_peer_created_before_the_check_is_not_failed_by_the_refusal` is red without the lookup (`CHILD_INPUT_INVALID`) and green with it. Closed |
| G043-044 | Not built in phase 2: the fault tests of T043-7, T043-9 and T043-11 (a crash between the delivery and the audit, two engines on one refused cron tick, a restart between the evaluation and the failure, a crash before the schedule decision commits); the budget of a child start and a continuation (T043-18 has the start and the triggered start); the cache counters of SC-043-004 as an integration test; the problem-shape assertions in `contract_rest_durable_execution`; the metric-label check of `integration_observability_operations`; the sentinel on rows 2, 3, 5 to 8, 10, 12 to 14 (T043-17 covers rows 1, 4, 9 and 11, the answers, logs, spans and audit rows, not the metrics text). | Phase 3. | Narrowed 2026-11-15: the child-start budget, the contract problem shapes (`crates/api-http/tests/run_input_errors.rs`), the metric labels and count (`an_action_checked_before_its_decision_and_at_its_start_is_counted_once`), `workLimit` and the transient cases on rows 6, 9, 11 and 12 are built; what is left is in G043-050. Open |
| G043-045 | The OpenAPI operations of the start routes, the schedule writes, trigger-now and the publication do not describe the design 043 refusals in their own text (D043-135). | The pinned baselines of features 016 and 035 would have to be renewed by their owners; the shared `Error` schema and `errors.md` carry the contract. | Reopened and closed 2026-11-15 by OD-2026-11-15-3 (D043-149). |
| G043-046 | `CONTINUATION_INPUT_INVALID` does not reach an SDK's run view: the SDK run and event views carry no run-level failure code, only the graph's node codes, so the conformance golden of `run-input-validation` shows the chain `FAILED` and the continuation node cancelled, while `CHILD_INPUT_INVALID` is shown on the parent's child node. | FR-043-015 is met for the child and not for the continuation; the run's stop code is in the journal (`run.stop_requested`) on the server. | Open 2026-11-15: FR-043-015 and T043-15 require it, and it is not built. The REST `Run` has no run-level failure code, so it needs a field of `Run` read from the run's stop, the six SDK run views, the Studio types and a conformance case. |
| G043-050 | Left after the second review round: integration tests of row 2 (an uploaded input), the batch `RERUN` item (`FAILED VALIDATION_FAILED`, deferred with no room), the synchronous reset, the pending reset left pending, and `RUN_INPUT_SCHEMA_UNUSABLE` on row 5; the four fault tests (a crash between the delivery and the audit, two engines on one refused cron tick, a restart between the evaluation and the failure of a continuation, a crash before a schedule decision commits); the continuation statement budget; the sentinel on rows 2, 3, 5 to 8, 10, 12 to 14 and on the metrics text. Done 2026-11-15: SC-043-004 through real starts in a binary of its own (`integration_run_input_cache`), `RUN_INPUT_SCHEMA_UNUSABLE` on rows 3, 4 and 8, the D043-038 lookup. | Not built. Done 2026-11-15 (phase 3): row 2 (`run_input::row_2_an_uploaded_input_start_checks_the_descriptor_and_a_refusal_claims_nothing`, workflow primitives); the batch `RERUN` item failed and deferred, the synchronous reset, `RUN_INPUT_SCHEMA_UNUSABLE` on row 5, the pending reset ended and left pending (`run_input_repeats::*`, schedules and bulk operations); the four fault tests (`run_input::*` of the events and workflow-primitives fault suites, the `InputInvalid` cases of `firing_decisions`); the continuation budget (`a_checked_continuation_costs_what_an_unchecked_one_does`); the sentinel on rows 3, 5, 6, 8, 12, 14, the schedule records, the journal's stops and the metrics text (`run_input_sentinel_rows`). Left: the sentinel on rows 7 and 10 (row 10 carries no caller value) and G043-046. | Open (narrowed 2026-11-15) |
| G043-051 | Designs 042 and 043 disagree on whether `sagat-application` may hold a runtime: 042 spawns a blocking task there for a blob source's array. D043-150 keeps both by allowing `tokio` with `rt` alone; moving the parse behind a port would restore the stricter rule. | For the owners of 042 and 043. | Closed 2026-11-15 as decision D043-154. |
| G043-052 | Design 042's blocking parse of a blob source runs in `sagat-application` with `tokio` `rt` (D043-154); moved behind a port of the adapter, the runtime rule could refuse any `tokio` there again. | Follow-up of 042. | Open |

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
