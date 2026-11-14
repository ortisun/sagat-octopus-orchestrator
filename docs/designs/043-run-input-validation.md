# Run input validation

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-14

--------------------------------------------------------------------------------

## Overview

From release 0.3.0 the input of every run is checked against the `inputSchema` of the workflow version the run will
pin, before the run is created. One port does the check for every path that creates a run: a start through the API
(and so the six SDKs, the CLI and the Studio), Signal-with-Start and Update-with-Start, a rerun, a reset, a batch rerun,
an event, webhook, message or cron trigger, a schedule's action, backfill and trigger-now, a child run, a
continue-as-new and an ad hoc database execution. An input that does not match is refused where it used to be
accepted. A caller who waits gets `422 VALIDATION_FAILED` with a closed reason, a JSON pointer and the rule that failed,
never the value. A creator that nobody waits for records its own outcome: a trigger audits and counts, a schedule
records an `INPUT_INVALID` action, a child fails its parent step, a continuation fails its run.

The owner decided this on 2026-11-14 (OD-2026-11-14-2, "Validar todos os starts já"), against the recommendation to
defer gap G1955 of feature 018. This design does not ask that question again. It asks three narrower ones (O043-001 to
O043-003): what protects existing users, what to do with a published schema that does not compile, and whether a
repeat of a pre-0.3.0 run is validated.

The registers are in `specs/043-run-input-validation/research.md` (D043-001 to D043-016, G043-001 to G043-009,
O043-001 to O043-003). The rule is proposed as ADR-0090.

## The Problem

The IR declares `inputSchema` (`specs/001-platform-foundation/contracts/workflow.schema.json:47`) and the platform
enforces it almost nowhere. Only a start with an uploaded input checks it (`crates/application/src/runs.rs:1673`); a
plain JSON input, a triggered run, a schedule's run, a child and a continuation are created with whatever they carry,
as long as it is a JSON object of at most 256 KiB (`runs.rs:1333-1337`). A workflow's first step finds out, or does
not.

Feature 018 could not close its own input check because of this (G1955, G2081 in
`specs/018-schedules-bulk-operations/research.md`): a schedule that validated would be stricter than a person starting
the same workflow, and `NOMINAL_TIME_FIELD_INVALID`, already in `specs/018-schedules-bulk-operations/contracts/errors.md:16`,
is never answered.

What makes it more than a check in `start_with`:

1. **Thirteen creating paths, and many with nobody waiting.** Triggers, schedule actions, backfills, children,
   continuations, batch items and pending resets run in the
   engine; a refusal needs an outcome someone can see, not an error that is logged and lost. A schedule action refused
   today with `NotAllowed` is deferred and retried forever (`crates/application/src/schedule_firing.rs:1203`), which
   for an input that can never pass is a loop.
2. **The schema is not in the parsed definition.** `WorkflowDefinition` (`crates/domain/src/workflow.rs:1228`) has no
   input schema; it is in the stored document only. Continue-as-new holds the definition and not the document.
3. **The hot path must not pay for it.** The design 038 budget (`tests/integration/claim-transition-efficiency/work_budget.rs`)
   holds what a lease and a completion cost; a start must not add a statement either.
4. **Published schemas were never checked to compile.** At publish only a schema with an `x-sagat-classification`
   annotation is compiled (`crates/infra-ir/src/carrier_schema_check.rs:12-23`). A version published with a schema
   that does not compile (an external `$ref`, a backtracking pattern) has run for months without anyone knowing.
5. **Existing callers may not match.** A definition whose schema its callers never satisfied, or a version with an
   event trigger whose schema describes a manual input rather than the event envelope `{event, data}`
   (`crates/application/src/schedule_triggers.rs:113`), stops starting runs the day the release is installed.
6. **The value must not leak.** The run input is a classified payload; the existing `infra-schema::validate`
   (`crates/infra-schema/src/lib.rs:90`) prints instance paths, which can hold keys the caller chose.

## Requirements

### Functional

- **FR-043-001** Every path that creates a run MUST check the run's input against the `inputSchema` of the exact
  version the run pins, before the transaction that creates the run opens, through one port. The paths and what each
  does on a refusal are the table of Technical Plan 1. *Acceptance:* for each row of that table an integration test
  starts a run with an input the schema refuses and observes the row's outcome and no new `runs` row, and starts one
  with a matching input and observes the run.
- **FR-043-002** A version without `inputSchema` MUST accept any input that passes today's checks (object, size). The
  dialect is the one `crates/infra-schema/src/lib.rs:74` compiles: `$schema` if present, else draft 2020-12; `format`
  is an annotation; an external `$ref` does not compile; patterns use the linear-time engine; `x-sagat-classification`
  annotations do not change the result. *Acceptance:* unit tests in `crates/infra-schema` for each of those five
  statements, and `annotation_is_inert.rs` still passes.
- **FR-043-003** A synchronous refusal MUST answer `422 VALIDATION_FAILED`, `retryable: false`, with
  `details.reason = RUN_INPUT_INVALID`, `details.rule` (a keyword from the closed list of D043-007) and `details.field`
  (a JSON pointer cut as D043-007 says). Nothing is created and no idempotency record is written. *Acceptance:*
  `contract_rest_durable_execution` asserts the problem shape for `POST /runs`, Signal-with-Start, Update-with-Start
  and `RERUN_AS_NEW`.
- **FR-043-004** A start refused for its input whose idempotency key already holds a run created with the same
  fingerprint MUST replay that run instead of refusing (D043-004). *Acceptance:* an integration test creates a run with
  validation off (the `report` mode, or a store fixture), then repeats the request with validation on and gets the
  same run.
- **FR-043-005** Signal-with-Start, Update-with-Start and a start with `USE_EXISTING` MUST validate the input before
  admission even when an open holder answers (D043-005). *Acceptance:* with an open holder, an invalid input is
  refused and the holder receives no signal and no Update.
- **FR-043-006** Schedules. Create and update MUST refuse an input that the target version refuses
  (`SCHEDULE_INPUT_INVALID`, `details.field` under `/action/input`) and a `nominalTimeField` that is not a declared
  top-level string property of a version that has an `inputSchema` (`NOMINAL_TIME_FIELD_INVALID`,
  `details.field = /action/nominalTimeField`), as D043-008 says. At action time a refused input MUST be recorded as an
  action with outcome `INPUT_INVALID` and that reason, the time acted on and not owed, no remaining action used up; the
  schedule pauses when `pause_on_failure` is set; a backfill stops; trigger-now answers `422` with the reason and
  records nothing (D043-009). *Acceptance:* integration tests for write, action, backfill and trigger-now in
  `integration_schedules_bulk_operations`; the action outcome appears in `GET` of the schedule's actions.
- **FR-043-007** An event, webhook, message or cron trigger whose run input is refused MUST start nothing, write one
  value-free audit record `run.input_refused` with the reason, count
  `trigger_events_total{outcome="input_invalid"}`, and still deliver the event to its other consumers (D043-010).
  *Acceptance:* `integration_events_webhooks_messaging` publishes an event matching two triggers, one refusing, and
  observes one run, one audit row and a woken wait.
- **FR-043-008** A child run whose input is refused MUST NOT be created; the parent step fails with
  `CHILD_INPUT_INVALID` (D043-011). *Acceptance:* `integration_workflow_primitives`.
- **FR-043-009** A continue-as-new whose evaluated input is refused MUST create no successor; the predecessor ends
  `FAILED` with `CONTINUATION_INPUT_INVALID` (D043-012). *Acceptance:* `integration_workflow_primitives`.
- **FR-043-010** A batch `RERUN` item refused for its input ends `FAILED` with `VALIDATION_FAILED`; a pending reset
  refused for its input ends `ADMISSION_REFUSED` (D043-013). *Acceptance:* `integration_schedules_bulk_operations`.
- **FR-043-011** A stored `inputSchema` that does not compile MUST be handled as O043-002 decides; the recommended rule
  refuses every start of that version with `RUN_INPUT_SCHEMA_UNUSABLE`. *Acceptance:* a version stored with an
  external `$ref` (inserted by the test as the owner role, since publish refuses it after FR-043-012) is refused, and
  the compile failure is remembered (one compile attempt per process, by counter).
- **FR-043-012** Publish (and `sagat validate`) MUST refuse a new version whose `inputSchema` does not compile, annotated
  or not, and MUST warn (a `DefinitionWarning`, not a refusal) when a version has an `inputSchema` and a non-manual
  trigger: the schema applies to the event envelope. *Acceptance:* `integration_gitops_versioning` and the CLI tests.
- **FR-043-013** The protection of existing users is what O043-001 decides; the recommended rule is a deployment setting
  `run_input_validation` with `enforce` (default) and `report`. In `report` mode every check runs, a refusal is counted
  and audited, and the run is created. *Acceptance:* a process started with `report` admits an invalid input and counts
  `outcome="reported"`; the setting is logged once at start.
- **FR-043-014** The value of an input MUST NOT appear in an error, a log line, a span, an audit record or a metric
  label, on any path of FR-043-001. Metric labels are closed sets (`creator`, `outcome`), with no tenant, namespace,
  workflow, version, field or rule. *Acceptance:* `integration_payload_classification` starts each creator with a
  sentinel value in an input that fails at that value and asserts the sentinel is in none of the response, the
  captured logs and spans, the audit rows and the metrics text.
- **FR-043-015** The six SDKs, the CLI and the Studio MUST surface `reason`, `field` and `rule` of the refusal; the
  Studio and the SDK schedule views show the `INPUT_INVALID` outcome. *Acceptance:* a conformance scenario
  `run-input-validation` passes in the six languages; Studio and CLI tests.
- **FR-043-016** A path that builds a run's creation input without the port MUST fail the build or the architecture
  check (D043-001). *Acceptance:* `scripts/check-architecture.py` fails on a negative fixture that calls the test-only
  constructor outside `tests/`.

### Non-functional

- **NFR-043-PERF** Validation adds no SQL statement to a start, a triggered start, a child start or a continuation, and
  compiles a version's schema at most once per process while it stays cached. Compiling never runs inside a database
  transaction.
- **NFR-043-SEC** No new grant, role membership or `SECURITY DEFINER` function for the runtime role. The schema is
  data evaluated in-process with the linear regex engine and bounded compile memory; no code of the tenant runs in the
  control plane or the engine.
- **NFR-043-COMPAT** The upgrade order of OD-2026-11-14-3 (migrate, engines, control planes) holds: the migration that
  widens the schedule action outcomes runs before an engine writes the new value, and old and new binaries run side by
  side for the length of the upgrade (G043-001).

### Success criteria

- **SC-043-001** Each of the fourteen rows of the creator table has the test of FR-043-001, passing, on real PostgreSQL.
- **SC-043-002** The sentinel test of FR-043-014 passes for every row.
- **SC-043-003** In `integration_claim_transition_efficiency`, a start, a triggered start, a child start and a
  continuation of a version with an `inputSchema` cost exactly the statements, transactions and rows written of the same
  shape without one.
- **SC-043-004** The cache counters show one compile for 1,000 starts of one version in one process, and one remembered
  failure for 1,000 starts of a version whose schema does not compile.
- **SC-043-005** `sdk_conformance` with `SAGAT_SDK_SCENARIO=run-input-validation` passes in TypeScript, Go, Python, C#,
  Java and Rust.

### User stories

- **US01** As a caller, I am told at once, with the field and the rule, that my input does not fit the workflow, and
  nothing is started.
- **US02** As an operator, I can see why a trigger, a schedule, a child or a continuation did not start a run, from an
  audit row, an action record or a failure code, without reading the input.
- **US03** As an operator upgrading to 0.3.0, I can find the versions likely to start refusing and I have a lever if the
  release breaks a caller I could not fix in time (O043-001).

## Goals

- One rule for every run creator, enforced by the type system and an architecture check rather than by convention.
- A refusal that a person or an operator can act on, with no input value in it.
- No cost on the statement budget of design 038.
- An upgrade an operator can prepare for.

## Non-Goals

- Validating signals, Updates, worker results or activity outputs against a schema (G043-008). OD-2026-11-14-2 is about
  starts.
- Asserting `format`, resolving external references, or a second dialect (G043-005, D043-006).
- A validation-only endpoint ("would this input start?"). The Studio form of feature 026 already renders the schema.
- Re-checking stored inputs (schedules, pending resets) at the upgrade (G043-007).
- Migrating or editing published versions: a version is immutable (constitution IV). A schema that does not fit is
  fixed by publishing a new version.
- Fixing the instance paths in the script schema errors of feature 005 (G043-009): reported, not changed.

## Open Questions

- What a 0.2.x control plane does with an `INPUT_INVALID` action during the upgrade (G043-001). The listing reads
  the outcome as a string and passes it through (`crates/application/src/schedules.rs:634-635`); the outcome filter
  parses names strictly (`crates/api-http/src/routes/schedules.rs:275`) and an old binary refuses a filter on the new
  value. T043-10 tests the listing; the release notes state the filter case.
- Whether the security review accepts that `details.field` names declared properties to a caller without the
  permission to read the version (G043-003).
- Whether the inventory of G043-004 should also try a synthetic envelope against each schema. It would flag more
  versions and could not prove any.

## Owner decisions needed

OD-2026-11-14-2 is decided: validation is on in 0.3.0 for every start. Three questions remain, each with a
recommendation; the registers have the options and the reasons (`research.md`, section 4).

- **O043-001** What protects existing users: (a) a deployment setting `enforce` (default) or `report`, with the
  release-note warning and an inventory; (b) a per-namespace setting; (c) warning and inventory only.
  **Recommended: (a).** Gates T043-13 and T043-18.
- **O043-002** A published `inputSchema` that does not compile: (a) fail closed with `RUN_INPUT_SCHEMA_UNUSABLE`;
  (b) fail open with a metric. **Recommended: (a).** Gates T043-12.
- **O043-003** Rerun, reset and batch rerun of a run whose stored input its version refuses: (a) validate as every
  start; (b) exempt the repeats. **Recommended: (a).** Gates T043-4 for the repeat paths.

## Critical User Journeys (CUJs)

### CUJ 1: A caller sends the wrong input

*   **Given**: a version whose `inputSchema` requires `orderId` as a string.
*   **When**: a caller starts it through the TypeScript SDK with `{"orderId": 42}`.
*   **Then**: the call fails with `VALIDATION_FAILED`, `reason` `RUN_INPUT_INVALID`, `field` `/orderId`, `rule`
    `type`; no run exists; `42` appears nowhere.

### CUJ 2: A schedule whose input no longer fits

*   **Given**: a schedule targeting the active version, written when the active version accepted its input, and a new
    version published that requires a field the schedule does not send.
*   **When**: the next time falls due.
*   **Then**: the action is recorded `INPUT_INVALID` with `SCHEDULE_INPUT_INVALID`; no run starts; the schedule goes on
    to its next time, or pauses if `pause_on_failure` is set; the operator sees the action in the Studio.

### CUJ 3: An event trigger on a version written for manual starts

*   **Given**: a version with an `inputSchema` `{"required": ["orderId"]}` and an event trigger.
*   **When**: an event arrives.
*   **Then**: no run starts; an audit row `run.input_refused` names the trigger's version and the reason; the metric
    counts `input_invalid`; a wait elsewhere for the same event is still woken. The publish of that version, had it
    happened after 0.3.0, would have warned that the schema applies to the envelope.

### CUJ 4: A child with a bad input

*   **Given**: a parent whose `child` node evaluates an input the child's version refuses.
*   **When**: the engine starts the child.
*   **Then**: no child run exists; the parent step fails with `CHILD_INPUT_INVALID`; its `onError` route, if any, runs.

### CUJ 5: The upgrade breaks a caller that cannot be fixed today

*   **Given**: 0.3.0 installed and a production caller that sends an input its version refuses.
*   **When**: the operator sets `run_input_validation = report` (if O043-001 is answered (a)) and restarts.
*   **Then**: the starts are admitted again, each counted `reported` and audited, until the caller or the version is
    fixed and the setting is put back.

## Technical Plan (Architecture)

### 1. The creators

Every row validates (D043-003) except the last, which is called from tests only. Line numbers are on `develop` at
`c2e9341f`.

| # | Creator | Entry | Where the run is created | Input | Refusal |
|---|---|---|---|---|---|
| 1 | `POST /runs` (SDKs, CLI `apps/cli/src/workflow.rs`, Studio) | `crates/api-http/src/routes/execution.rs:505` → `RunService::start_with_options` `crates/application/src/runs.rs:989` | `start_with` `runs.rs:1313` → `store.create` `runs.rs:1725` | caller | `422 RUN_INPUT_INVALID` |
| 2 | Start with an uploaded input | `execution.rs:489` → `start_with_blob` `runs.rs:1150` | as 1; today's check at `runs.rs:1673` is replaced by the port | the blob descriptor | `422 RUN_INPUT_INVALID` |
| 3 | Signal-with-Start, Update-with-Start | `execution.rs:857`, `:993` → `runs.rs:1010`, `:1057` | as 1 | caller | `422`, also when a holder exists (D043-005) |
| 4 | `RERUN_AS_NEW`, batch `RERUN` | `execution.rs:1612` → `rerun_item` `runs.rs:1232`; batch `crates/application/src/runs/batch_items.rs:134` | as 1 | the stored input of the ended run | `422`; batch item `FAILED VALIDATION_FAILED` (O043-003) |
| 5 | Reset, pending reset | `crates/api-http/src/routes/resets.rs:158` → `reset.rs:550`; pass `crates/application/src/runs/pending_reset.rs:424` | `create_from_prefix` `crates/application/src/runs/reset.rs:696` → `start_with` | the stored input of the source | `422`; pending reset `ADMISSION_REFUSED` (O043-003) |
| 6 | Schedule action | `crates/scheduler/src/schedules.rs:278` → `fire` `crates/application/src/schedule_firing.rs:611` → `start` `:1174` | `ScheduledStarts::start_scheduled` `runs.rs:3156` → `start_with` | the schedule's input with `nominalTimeField` (`schedule_firing.rs:1545`) | action `INPUT_INVALID` |
| 7 | Schedule backfill | `crates/scheduler/src/schedules.rs:168` → `backfill_step` `schedule_firing.rs:405` | as 6 | as 6 | action `INPUT_INVALID`, backfill stops |
| 8 | Schedule trigger-now | `crates/api-http/src/routes/schedules.rs` → `trigger` `schedule_firing.rs` → `trigger_once` `:240` | as 6 | as 6 | `422 SCHEDULE_INPUT_INVALID`, nothing recorded |
| 9 | Event, webhook, message trigger | `crates/scheduler/src/events.rs:294` `dispatch` → `start_run` `:386` → `start` `:407` | `store.create` `events.rs:486` | the event envelope | audit + metric, no run |
| 10 | Cron trigger (legacy pass until the handover) | `cron_pass` `events.rs:748` → `start` `:809` | as 9 | `{event, data: {scheduledAt, firedAt}}` | as 9 |
| 11 | Child run | `crates/scheduler/src/passes.rs:5142` `start_child` | `store.create` `passes.rs:5302` | the parent node's evaluated input | parent step `CHILD_INPUT_INVALID` |
| 12 | Continue-as-new (evaluated) | `advance_continuation_in_view` `passes.rs:775`, input at `:838` | `continue_evaluated` `crates/infra-postgres/src/chain_progress.rs:1494` | the evaluated expression | predecessor `FAILED CONTINUATION_INPUT_INVALID` |
| 13 | Ad hoc database execution, connection probe, private rerun | `crates/api-http/src/routes/database_executions.rs:103`, `:141` → `crates/application/src/runs/ad_hoc.rs:365` | `store.create` `ad_hoc.rs:523` | caller, or `{}` | `422 RUN_INPUT_INVALID` when the document declares a schema |
| 14 | Schedule create and update (not a run, the same check) | `crates/application/src/schedules.rs:1301`, `:1560` → `check` `:1240` | none | the schedule's input | `422 SCHEDULE_INPUT_INVALID`, `NOMINAL_TIME_FIELD_INVALID` |
| — | Continue-as-new (legacy, tests only) | `chain_progress.rs:1149` | `chain_progress.rs:1267` | the predecessor's input, byte for byte | not validated (D043-003) |

Not found on `develop`: a separate whole-run retry or replay, a script test run (it writes `script_test_runs`), a gRPC
start (`crates/api-grpc` is worker-only), internal apps (027) and cross-namespace services (030). When one of those is
built, the type of D043-001 makes it go through the port.

### 2. The schema travels with the definition

The IR codec (`crates/infra-ir/src/lib.rs:154`, `parse_stored`, and its private twin) puts the canonical bytes of
`inputSchema` into the parsed `WorkflowDefinition` as an opaque byte string (`InputSchema`, domain, no `serde_json`).
`WorkflowDefinition::footprint` counts it, so the `DefinitionCache` byte budget
(`crates/application/src/execution.rs:1951`) still bounds memory. Every creator already holds the definition when it
needs the schema: rows 1 to 8 and 13 through `start_with`'s `get_version_by_id` and `get_or_parse` (`runs.rs:1402`,
`:1408`), rows 9 and 10 at `events.rs:416`, row 11 at `passes.rs:5224`, row 12 through the run transaction's
`tx.parts()` before it commits (`passes.rs:832`). No creator reads the database for the schema (NFR-043-PERF).

### 3. The port, the type and the cache

```
// crates/application/src/run_input.rs (pseudocode)
trait RunInputValidator: Send + Sync {
    fn check(&self, version: WorkflowVersionId, definition: &WorkflowDefinition,
             input: Vec<u8>, creator: Creator) -> Result<CheckedRunInput, InputRefusal>;
}
struct CheckedRunInput { bytes: Vec<u8>, outcome: Checked }   // fields private
enum Checked { NoSchema, Valid, Reported }                       // Reported: report mode only
enum InputRefusal { Invalid { field: Pointer, rule: Rule }, SchemaUnusable }
enum Creator { Api, Repeat, Schedule, Trigger, Child, Continuation, AdHoc }  // the metric label
```

`NewRun.input` and the input of `ContinuationSelection::new` (`crates/application/src/chain_progress.rs`) become
`CheckedRunInput`. A test constructor exists, and `scripts/check-architecture.py` fails when it is named outside
`tests/` or a `#[cfg(test)]` module (FR-043-016). The existing `RunInputChecker` (`crates/application/src/blobs.rs:284`)
is removed; the blob start goes through the port.

The adapter in `crates/infra-schema` (`InputSchemas`) compiles with the options of `compile`
(`crates/infra-schema/src/lib.rs:74`) and caches the compiled validator per version id. The cache is the
classification cache (`crates/infra-schema/src/classification/cache.rs`) made generic over its key and value, with its
own bounds: 256 entries and 64 MiB of estimated compiled bytes, single flight per version, at most four compiles at
once, a remembered failure per version. A compile never runs inside a database transaction: every creator calls the
port before it opens one (row 12 after `tx.commit()` at `passes.rs:832`).

The refusal is built value-free (D043-007): the first violation's keyword mapped to a closed `Rule`, and the instance
pointer cut before the first segment that is neither an array index nor a name declared under some `properties` of the
schema (the set is computed once per compiled version). A `required` violation appends the missing name, which the
schema holds. Nothing else of the error is read: the `jsonschema` error kinds carry values and unexpected keys.

### 4. What each creator does with a refusal

- **Rows 1 to 5, 8, 13 (a caller waits).** `AppError::InvalidRunInput { reason, field, rule }` (D043-014), mapped in
  `crates/api-http/src/error.rs` beside `InvalidStart` (`:854`) to `422 VALIDATION_FAILED` with the three details, and a
  remediation line beside `SCHEDULE_INPUT_INVALID` (`:187`). Before answering, `start_with` replays an existing run of
  the same key and fingerprint (D043-004). The batch executor's `error_code` (`batch_executor.rs:134`) and the reset
  completer's `failure_of` (`reset_completion.rs:220`) list the new variant.
- **Rows 6 and 7 (schedule pass).** A new arm in `schedule_firing.rs` `start` (`:1174`) before the `NotAllowed` arm
  (`:1203`) records the action through the `record_held` path (`:1333`) with `Outcome::InputInvalid` and the reason,
  then pauses the schedule when `pause_on_failure` is set; the backfill arm (`:556`) records it and stops the backfill.
  A migration widens `schedule_actions_outcome_valid`
  (`crates/infra-postgres/migrations/20261230000000_schedules.up.sql:176`) with `INPUT_INVALID`; its down migration
  refuses while a row holds it. The OpenAPI enum (`specs/001-platform-foundation/contracts/openapi.yaml:8080`) and the
  Studio model (`frontend/apps/studio/src/features/schedules/model.ts:342`) gain the value.
- **Row 14 (schedule write).** `target_check` (`crates/application/src/schedules.rs:731`) returns the target version's
  definition with `Found`, so `check` (`:1240`) can validate the input with `nominalTimeField` filled (D043-008). The
  write path is not on a budget; the version read it adds is acceptable there.
- **Rows 9 and 10 (triggers).** In `events.rs` `start` (`:407`), after `get_or_parse`: on a refusal, `record_denial`
  with `AuditAction::RunInputRefused` (`run.input_refused`, resource the workflow version, reason the closed code),
  `trigger_events_total{outcome="input_invalid"}`, and `Ok(Started::No)`, as the database denial does (`:512-528`).
- **Row 11 (child).** In `start_child` after `get_or_parse` (`passes.rs:5224`): `finish_with(...,
  ChildOutcome::Failed(FailureCode::child_input_invalid()))`, as for `CHILD_NOT_FOUND` (`:5217`).
- **Row 12 (continuation).** After the evaluation (`passes.rs:838-866`): a refusal goes to a twin of
  `finalize_continuation_expression` (`:982`) with `FailureCode::continuation_input_invalid()` and its own metric.

### 5. The mode setting (O043-001, recommended answer)

`run_input_validation = enforce | report`, in the configuration of `apps/control-plane` and `apps/execution-engine`,
read once at start and logged. In `report` the port returns `Checked::Reported` instead of a refusal, counts it and the
creator writes the same audit record as a trigger refusal, then creates the run. If the owner answers (b) or (c), this
section changes before the build.

### 6. Telemetry

`run_input_validations_total{creator, outcome}` with `outcome` in `accepted`, `no_schema`, `refused`,
`schema_unusable`, `reported`; `run_input_schema_cache_total{result}` with `hit`, `miss`, `evicted`,
`failure_remembered`. A span attribute `sagat.run_input.outcome` and, on a refusal, `sagat.run_input.rule`. Logs at
`debug` name the version id, the creator and the rule. None carries a value, a field, a tenant or a namespace label
(constitution V and VI).

### 7. Publish

`validated_document_in` (`crates/infra-ir/src/lib.rs:326`) compiles every `inputSchema`, annotated or not, and returns
an `IrIssue` at `/inputSchema` when it does not compile. The stored-document path (`validated_stored_document_in`) does
not, so a version already stored still parses. A `DefinitionWarning` is added when a version has an `inputSchema` and a
trigger whose kind is not `manual`. The versioning diff already reports a tighter `inputSchema` as breaking
(`crates/infra-ir/src/diff.rs:178`); with this feature that report becomes true.

### 8. Rollout

Upgrade order (OD-2026-11-14-3): the migration first, then the engines (rows 6, 7, 9 to 12 validate from then on), then
the control planes (rows 1 to 5, 8, 13, 14). During the window an old control plane admits what a new one refuses; no
state format changes, so nothing has to be repaired. Rollback: downgrade the binaries; the down migration refuses
while an `INPUT_INVALID` action exists, so an operator who rolls back after one was recorded deletes or keeps the
schema change consciously.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Triggers on versions whose schema describes a manual input stop starting runs, silently for the caller (G043-004) | Audit row and metric per refusal; the publish warning from 0.3.0; the inventory query in `docs/runbooks/run-input-validation.md` (versions with an `inputSchema` and a non-manual trigger); the `report` lever (O043-001); the release notes name the case first |
| A published schema that does not compile blocks its version (O043-002) | Same inventory lists them (they fail the compile at once); the remembered failure keeps it to one compile per process; fixed by publishing a new version |
| A pointer leaks a key the caller chose | The cut rule of D043-007 and the sentinel test of FR-043-014, including a map key under `additionalProperties` and `patternProperties` |
| Existing tests and fixtures start runs whose input does not match their schema | T043-19 sweeps them first and lists each change; a fixture is fixed, never the check loosened |
| A schedule loops on an input that can never pass | `INPUT_INVALID` is recorded and the time is not owed (D043-009), unlike today's `NotAllowed` arm |
| Compiling a large schema on the API path | Once per version per process (SC-043-004), never under a transaction, at most four at once; compile bounds of the classification cache |
| An old control plane meets an `INPUT_INVALID` action during the upgrade (G043-001) | The listing passes unknown outcomes through by design; the strict outcome filter is stated in the release notes; T043-10 tests the listing |
| `report` mode becomes permanent | Logged at every start of a process, counted, and removed no earlier than 0.4.0 by a recorded owner decision |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | The check runs before the creation transaction and writes nothing; a schedule refusal is a decision committed with the schedule's state under its fence (`record_held`); a continuation refusal ends the predecessor through the existing fenced path |
| II. Modular domain, ports and adapters | Pass | The domain holds opaque schema bytes; the port is in `application`, `jsonschema` stays in `infra-schema`; no new dependency edge (`scripts/check-architecture.py`) |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect is added; a refused run has none |
| IV. Compatibility is versioned and pinned | Pass with conditions | Published versions are not changed; the check uses the pinned version's own schema. The behaviour change is the owner's (OD-2026-11-14-2); contracts change additively (a reason, an outcome value, a failure code); see Complexity Tracking |
| V. Code, tenant boundary and secrets are isolated | Pass | No tenant code runs; linear regex and bounded compile; no grant (NFR-043-SEC); values never in errors or telemetry (FR-043-014) |
| VI. Observability and audit are part of the feature | Pass | Metrics with closed labels, span attributes, audit of every asynchronous refusal (Technical Plan 6) |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | A trigger refusal is deterministic for the same event and version; a redelivery repeats the same answer (G043-002) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Per-creator tests on real PostgreSQL, the crash and duplicate cases of the Verification Plan, six-SDK conformance; gaps registered |
| Delivery Constraints | Pass | No new technology; single region; the upgrade order of OD-2026-11-14-3 |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| `CheckedRunInput` and an architecture rule | Thirteen creating paths in three crates; the next one must not skip the check | A code review rule: how the gap of G1955 came to exist |
| The schema bytes in `WorkflowDefinition` | Continue-as-new holds the definition, not the document; no statement may be added | Reading the document on a cache miss: an extra statement on the continuation path |
| A second compiled-validator cache | Compile once per version; never under a lock | Compiling per start, as the blob check does today: cost on every start |
| A schedule action outcome and a migration | A refused action must be visible and must not loop | Reusing `ADMISSION_RETRY` (`NotAllowed`): retries forever |
| Two failure codes (`CHILD_INPUT_INVALID`, `CONTINUATION_INPUT_INVALID`) | The parent and the chain must say why, without the value | `EXPRESSION_ERROR`: says the expression failed, which it did not |
| A refusal where an input used to be accepted (principle IV) | The owner's decision OD-2026-11-14-2; published versions stay as they are, only the admission of new runs changes, and the contracts change additively | Deferring validation, as recommended before the decision: the owner refused it |
| The `report` mode (if O043-001 is (a)) | A lever for an upgrade that breaks a caller | No lever: an operator's only recourse would be a downgrade |

## Alternatives Considered

- **Validate in `PgRunStore::create`.** One place for rows 1 to 11 and 13, but in the infra layer, inside the creation
  transaction (a compile under the run's locks), and continue-as-new does not pass there. Rejected.
- **A check written in each creator, no shared type.** What exists for the blob start. It drifts: the blob check
  compiles per call and words its own error. Rejected.
- **Validation in the SDKs only.** Six implementations of a JSON Schema dialect, and triggers, schedules, children
  and continuations have no SDK in the path. Rejected.
- **Validation at publish only** (check that the schema compiles, not the inputs). Leaves G1955 open. Kept as part of
  this design (FR-043-012), not instead of it.
- **Defer, as recommended before OD-2026-11-14-2.** The owner chose otherwise.

## Non-Functional Requirements

- **NFR-043-PERF**, **NFR-043-SEC**, **NFR-043-COMPAT** as stated under Requirements; SC-043-003 and SC-043-004 are their
  tests.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Each task is one pull request unless it says otherwise, with its tests seen failing first. Tasks 1 to 3 come first;
4 to 12 can run in parallel after them; 14 to 19 close the feature.

- [ ] T043-1 Carry the canonical `inputSchema` bytes in the parsed `WorkflowDefinition`, counted by its footprint.
  Files: `crates/domain/src/workflow.rs` (an `InputSchema` byte string, accessor, footprint);
  `crates/infra-ir/src/lib.rs` (extract on `parse`, `parse_stored` and the private twins); unit tests in
  `crates/infra-ir/tests/cache.rs`.
  Refs: FR-043-001, NFR-043-PERF
  Owner-gate: OD-2026-11-14-2
- [ ] T043-2 Add the `RunInputValidator` port, `CheckedRunInput`, `InputRefusal`, and the `InputSchemas` adapter with
  its per-version cache, the value-free refusal and the dialect tests; remove `RunInputChecker`.
  Files: `crates/application/src/run_input.rs` (new), `crates/application/src/lib.rs`,
  `crates/application/src/blobs.rs`; `crates/infra-schema/src/lib.rs`, `crates/infra-schema/src/run_input.rs` (new),
  `crates/infra-schema/src/classification/cache.rs` (generic over key and value); tests
  `crates/infra-schema/tests/run_input.rs` (dialect, cut rule, a proptest that no generated value appears in a refusal).
  Refs: FR-043-002, FR-043-014, NFR-043-PERF, NFR-043-SEC, SC-043-004
  Owner-gate: OD-2026-11-14-2
- [ ] T043-3 Make `NewRun.input` and the continuation input a `CheckedRunInput`; add the architecture rule and its
  negative fixture; wire the adapter in both composition roots.
  Files: `crates/application/src/runs.rs` (`NewRun`), `crates/application/src/chain_progress.rs`,
  `scripts/check-architecture.py`, `scripts/tests/` fixture, `apps/control-plane/src/lib.rs` (`:746`),
  `apps/execution-engine/src/lib.rs`; every test that builds a `NewRun` uses the test constructor.
  Refs: FR-043-016
  Owner-gate: OD-2026-11-14-2
- [ ] T043-4 Validate in `start_with` (rows 1 to 5): the error variant, the replay before refusal, the problem
  mapping, the remediation line, and the batch and reset mappings.
  Files: `crates/application/src/runs.rs`, `crates/application/src/error.rs`, `crates/api-http/src/error.rs`,
  `crates/application/src/batch_executor.rs`, `crates/application/src/reset_completion.rs`,
  `specs/043-run-input-validation/contracts/errors.md` (new), `docs/quickstarts/errors.md`; tests in
  `tests/integration/durable-execution/`, `tests/contract/rest-durable-execution/`,
  `tests/integration/schedules-bulk-operations/` (with-start, rerun, reset, batch).
  Refs: FR-043-001, FR-043-003, FR-043-004, FR-043-005, FR-043-010
  Owner-gate: OD-2026-11-14-2, O043-003 (open)
- [ ] T043-5 Validate ad hoc and private starts (row 13).
  Files: `crates/application/src/runs/ad_hoc.rs`; tests in `tests/integration/database-connections/`.
  Refs: FR-043-001
  Owner-gate: OD-2026-11-14-2
- [ ] T043-6 Validate triggered starts (rows 9 and 10): audit action, metric outcome, `Started::No`.
  Files: `crates/scheduler/src/events.rs`, `crates/domain/src/audit.rs` (`RunInputRefused`); tests in
  `tests/integration/events-webhooks-messaging/`, `tests/fault/events-webhooks-messaging/` (crash after the audit,
  redelivery).
  Refs: FR-043-001, FR-043-007
  Owner-gate: OD-2026-11-14-2
- [ ] T043-7 Validate child starts (row 11) with `CHILD_INPUT_INVALID`.
  Files: `crates/scheduler/src/passes.rs`, `crates/domain/src/execution/event.rs`; tests in
  `tests/integration/workflow-primitives/`.
  Refs: FR-043-001, FR-043-008
  Owner-gate: OD-2026-11-14-2
- [ ] T043-8 Validate evaluated continue-as-new (row 12) with `CONTINUATION_INPUT_INVALID`.
  Files: `crates/scheduler/src/passes.rs`, `crates/domain/src/execution/event.rs`; tests in
  `tests/integration/workflow-primitives/chain_progress.rs`, `tests/fault/workflow-primitives/` (restart between the
  evaluation and the failure).
  Refs: FR-043-001, FR-043-009
  Owner-gate: OD-2026-11-14-2
- [ ] T043-9 Validate schedule writes (row 14): `target_check` returns the definition; `SCHEDULE_INPUT_INVALID` with
  a pointer under `/action/input`; `NOMINAL_TIME_FIELD_INVALID`.
  Files: `crates/application/src/schedules.rs`, `crates/infra-postgres/src/schedules.rs` (the `target_check` query),
  `crates/api-http/src/error.rs` (remediation of `NOMINAL_TIME_FIELD_INVALID`); tests in
  `crates/application/tests/schedule_document.rs`, `tests/integration/schedules-bulk-operations/`.
  Refs: FR-043-006
  Owner-gate: OD-2026-11-14-2
- [ ] T043-10 Record `INPUT_INVALID` schedule actions (rows 6 to 8): migration, outcome, pause by policy, backfill
  stop, trigger-now `422`; OpenAPI enum and Studio label; find out G043-001.
  Files: `crates/infra-postgres/migrations/<next timestamp>_schedule_action_input_invalid.{up,down}.sql` (take the
  timestamp after grepping the other branches), `crates/domain/src/schedules.rs`, `crates/application/src/schedule_firing.rs`,
  `specs/001-platform-foundation/contracts/openapi.yaml`, `frontend/apps/studio/src/api/schema.d.ts` (regenerated),
  `frontend/apps/studio/src/features/schedules/model.ts`; tests in `tests/integration/schedules-bulk-operations/`,
  `tests/fault/schedules-bulk-operations/` (crash between the decision and the pause), the runtime-role run.
  Refs: FR-043-006, NFR-043-COMPAT
  Owner-gate: OD-2026-11-14-2
- [ ] T043-11 Surface the refusal in the CLI and the Studio start form (`reason`, `field`, `rule`).
  Files: `apps/cli/src/workflow.rs`, `apps/cli/tests/`, `frontend/apps/studio/src/` (the start dialog and its test).
  Refs: FR-043-015
  Owner-gate: OD-2026-11-14-2
- [ ] T043-12 Handle a stored schema that does not compile as O043-002 decides, with the remembered failure.
  Files: `crates/infra-schema/src/run_input.rs`, `crates/application/src/run_input.rs`; tests in
  `tests/integration/durable-execution/`.
  Refs: FR-043-011, SC-043-004
  Owner-gate: OD-2026-11-14-2, O043-002 (open)
- [ ] T043-13 Build the protection O043-001 decides (recommended: the `enforce` / `report` setting).
  Files: `apps/control-plane/src/lib.rs`, `apps/execution-engine/src/lib.rs`, their configuration docs,
  `crates/application/src/run_input.rs`; tests in `tests/integration/durable-execution/`.
  Refs: FR-043-013
  Owner-gate: OD-2026-11-14-2, O043-001 (open)
- [ ] T043-14 Refuse a new version whose `inputSchema` does not compile; warn on a schema with a non-manual trigger.
  Files: `crates/infra-ir/src/lib.rs`, `crates/infra-ir/src/carrier_schema_check.rs`,
  `crates/application/src/definitions.rs` (`DefinitionWarning`); tests in `crates/infra-ir/tests/`,
  `tests/integration/gitops-versioning/`, `apps/cli/tests/`.
  Refs: FR-043-012
  Owner-gate: OD-2026-11-14-2
- [ ] T043-15 Emit the metrics and span attributes, and run the sentinel test across every creator.
  Files: `crates/infra-schema/src/run_input.rs`, the creators of T043-4 to T043-10; tests in
  `tests/integration/payload-classification/`, `tests/integration/observability-operations/` (labels are closed sets).
  Refs: FR-043-014, SC-043-002
  Owner-gate: OD-2026-11-14-2
- [ ] T043-16 Budget a start, a triggered start, a child start and a continuation with and without an `inputSchema`.
  Files: `tests/integration/claim-transition-efficiency/work_budget.rs`.
  Refs: NFR-043-PERF, SC-043-003
- [ ] T043-17 Add the conformance scenario `run-input-validation` and pass it in the six SDKs.
  Files: `tests/contract/sdk-conformance/scenarios/run-input-validation.json` (new), the six SDK conformance runners
  under `sdks/`, `sdks/SPEC.md` (the reasons and the outcome), `sdks/RELEASE_NOTES.md`.
  Refs: FR-043-015, SC-043-005
  Owner-gate: OD-2026-11-14-2
- [ ] T043-18 Write the runbook with the inventory query, the release notes, the traceability rows, and close G1955 and
  G2081 of feature 018 by reference.
  Files: `docs/runbooks/run-input-validation.md` (new), `docs/releases/unreleased.md` (or the 0.3.0 notes where they
  live), `docs/traceability.md`, `specs/018-schedules-bulk-operations/research.md` (a closing entry citing this
  design), `specs/043-run-input-validation/research.md` (closures), `docs/reference` regenerated after reading the
  diff (`contract_developer_experience`).
  Refs: FR-043-013, FR-043-006
  Owner-gate: OD-2026-11-14-2, O043-001 (open)
- [ ] T043-19 Sweep the tests, fixtures and examples whose inputs do not match their schemas, before T043-4 lands.
  Files: whatever the sweep finds under `tests/`, `examples/`, `sdks/*/examples`; each change listed in the pull
  request.
  Refs: SC-043-001

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`python3 scripts/check-architecture.py`, `python3 scripts/check-agent-config.py` and
`python3 scripts/check-design-docs.py`, the unit tests of the crates it changes, and these targets as its rows need:

- `integration_durable_execution` (rows 1, 2, 13; FR-043-004, FR-043-011, FR-043-013), also with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
- `contract_rest_durable_execution` (the problem shape of FR-043-003);
- `integration_schedules_bulk_operations`, `fault_schedules_bulk_operations`, `contract_schedules_bulk_operations`
  (rows 3 to 8 and 14; the migration as the runtime role);
- `integration_events_webhooks_messaging`, `fault_events_webhooks_messaging` (rows 9, 10; duplicate delivery and a
  crash after the audit);
- `integration_workflow_primitives`, `fault_workflow_primitives` (rows 11, 12; restart between evaluation and failure);
- `integration_database_connections` (row 13);
- `integration_gitops_versioning` (FR-043-012);
- `integration_payload_classification` (the sentinel, FR-043-014), `integration_observability_operations` (labels);
- `integration_claim_transition_efficiency` (SC-043-003), which passes unmodified except for the new lines of T043-16;
- `SAGAT_SDK_SCENARIO=run-input-validation cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored`;
- the per-SDK checks of `sdks/AGENTS.md`, `(cd frontend/apps/studio && npm run api && npm run check:api && npm test)`,
  and the CLI tests;
- `SAGAT_DOCS_BLESS=1 cargo test -p sagat-tests --test contract_developer_experience` only after reading the diff, then
  again without the switch.

Failure cases (constitution VIII): a crash between the check and the creation (nothing to recover: the check wrote
nothing); a redelivered event after a crash (the same refusal, G043-002); a restart with a cold cache (one compile, no
statement); a schedule refusal under a lost shard lease (the decision is refused by its fence, as `record_held` is);
two creators compiling the same version at once (single flight, one compile by counter). A test that fails is a defect
to explain, not to retry.
