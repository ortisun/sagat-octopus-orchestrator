# Run input validation

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-16

--------------------------------------------------------------------------------

## Overview

From release 0.3.0 the input of every run is checked against the `inputSchema` of the workflow version the run will
pin, before the run is created. One gate in `crates/application` does the check for each of the thirteen paths that
create a run: a start through the API (and so the six SDKs, the CLI and the Studio), an uploaded-input start,
Signal-with-Start and Update-with-Start, a rerun or batch rerun, a reset or pending reset, a schedule's action,
backfill and trigger-now, an event, webhook or message trigger, a cron trigger, a child run, a continue-as-new and an
ad hoc database execution. An input that does not match is refused where it used to be accepted. A caller who waits
gets `422 VALIDATION_FAILED` with a closed reason, a JSON pointer and the rule that failed, never the value. A creator
that nobody waits for records a permanent, value-free outcome: a trigger records a delivery without a run and audits
it once, a schedule records an `INPUT_INVALID` action, a child fails its parent step, a continuation fails its run.

The `inputSchema` is a documented subset of JSON Schema, validated by a validator of the platform whose work counter
stops at a budget (OD-2026-11-14-14); a schema outside the subset is refused at publication and, if already stored,
refuses every start of its version. `control-plane input-schema-inventory` lists the stored versions it affects.

The draft history: the first was read by the architecture, security and correctness reviewers on 2026-11-14,
and the owner then answered its questions (`docs/owner-decisions.md`):

- OD-2026-11-14-2: every start is validated in 0.3.0 (the decision this design implements).
- OD-2026-11-14-8 (O043-001): `enforce` by default; a deployment may run in `report` while it fixes its callers, and
  the mode goes through the instance registry so that replicas cannot answer differently once the operator requires it.
- OD-2026-11-14-9 (O043-004, against the recommendation): children and continuations of runs created before the
  upgrade are validated too, an accepted deviation from constitution principle IV.
- OD-2026-11-14-10 (against the recommendation): a Signal-with-Start or Update-with-Start whose holder is open is
  still refused when its start input does not match.
- OD-2026-11-14-11 (O043-002, O043-003): a schema that does not compile refuses every start of its version
  (`RUN_INPUT_SCHEMA_UNUSABLE`), permanently for the creators nobody waits for; a rerun or reset of a run from before
  0.3.0 is validated like any start.
- OD-2026-11-14-14 ("Subconjunto suportado"), after four review rounds of a static bound on `jsonschema`: the subset
  and the platform's own validator with a work counter. `research.md` dates each of the ten drafts that followed.
- OD-2026-11-14-15 ("Aceitar e construir"): ADR-0090 accepted and T043-1 to T043-24 to be built before this design is
  approved. The build ran in three phases, each reviewed: phase 1 the validator, publication and the inventory (T043-1,
  T043-2, T043-13, T043-24, branch `feature/043-validator`); phase 2 the gate and the creators (T043-3 to T043-12);
  phase 3 the remaining tests, the contracts and the SDKs. What the build learned is folded into this text, each change
  citing its decision.
- OD-2026-11-15-3 ("Atualizar e voltar a fixar"): the six start operations of the OpenAPI describe their `422`, and the
  pinned baselines of features 016 and 035 were re-pinned once after reading the diff (D043-149).

The registers are in `specs/043-run-input-validation/research.md` (D043-001 to D043-163, G043-001 to G043-053,
O043-001 to O043-006). The rule is ADR-0090, refined by ADR-0092 (Proposed, for the owner).

## The Problem

The IR declares `inputSchema` (`specs/001-platform-foundation/contracts/workflow.schema.json:47`) and the platform
enforces it almost nowhere. Only a start with an uploaded input checks it (`crates/application/src/runs.rs:1673`); a
plain JSON input, a triggered run, a schedule's run, a child and a continuation are created with whatever they carry,
as long as it is a JSON object of at most 256 KiB (`runs.rs:1333-1337`).

Feature 018 could not close its own input check because of this (G1955, G2081 in
`specs/018-schedules-bulk-operations/research.md`): a schedule that validated would be stricter than a person starting
the same workflow, and `NOMINAL_TIME_FIELD_INVALID` (`specs/018-schedules-bulk-operations/contracts/errors.md:16`) is
never answered.

What makes it more than a check in `start_with`:

1. **Thirteen creating paths, and many with nobody waiting.** Triggers, schedule actions, backfills, children,
   continuations, batch items and pending resets run in the engine; a refusal needs an outcome someone can see, and
   must not be retried when it cannot pass. A schedule action refused today with `NotAllowed` is deferred and retried
   forever (`crates/application/src/schedule_firing.rs:1203`), and an error variant that no arm names falls to
   `Err(other)` (`:1214`) and is fired again every tick.
2. **The schema is not in the parsed definition.** `WorkflowDefinition` (`crates/domain/src/workflow.rs:1228`) has no
   input schema; continue-as-new holds the definition and not the document.
3. **The hot path must not pay for it.** The design 038 budget (`tests/integration/claim-transition-efficiency/work_budget.rs`)
   holds what a lease and a completion cost; a start must not add a statement either.
4. **Published schemas were never checked to compile or to stay within bounds.** At publish only a schema with an
   `x-sagat-classification` annotation is compiled (`crates/infra-ir/src/carrier_schema_check.rs:12-23`), and the
   compile used for scripts (`crates/infra-schema/src/lib.rs:74`) has a 1 MiB pattern limit and no depth, pattern-count
   or expansion bound. A schema is author data that the control plane and the engine will now evaluate on every start.
   Four review rounds showed that the cost of validating an arbitrary schema with `jsonschema` cannot be bounded
   beforehand by a static walk (`unevaluated*` re-runs branches, `referencing` decodes pointers differently from the
   walk, value-sized keywords).
5. **The dialect is not what it seems.** Under drafts 4, 6 and 7 `jsonschema` 0.58.6 treats `format` as an assertion
   by default (`compiler.rs:66-68`, `:538-541`) and compiles `contentMediaType` and `contentEncoding` as checks
   (`keywords/mod.rs:473-480`). The subset gives each of its keywords one meaning in draft-07 and 2020-12 instead.
6. **Existing callers and runs may not match.** A definition whose schema its callers never satisfied, a version with
   an event trigger whose schema describes a manual input rather than the event envelope `{event, data}`
   (`crates/application/src/schedule_triggers.rs:113`), and a run started before the upgrade that will create a child
   or continue: all start failing the day the release is installed.
7. **The value must not leak.** The run input is a classified payload. The existing `infra-schema::validate`
   (`crates/infra-schema/src/lib.rs:90`) formats instance paths and `Debug` text of the error kind, both of which can
   carry what the caller sent.

## Requirements

### Functional

- **FR-043-001** Every path that creates a run MUST check the run's input against the `inputSchema` of the exact
  version the run pins, before the transaction that creates the run opens, through the gate of Technical Plan 3. The
  thirteen paths, and what each does on each kind of refusal, are rows 1 to 13 of Technical Plan 1. This includes the
  children and continuations of runs created before the upgrade (OD-2026-11-14-9). *Acceptance:* for each of rows 1 to
  13 an integration test starts with an input the schema refuses and observes the row's outcome and no new `runs` row,
  then with a matching input and observes the run; rows 11 and 12 also with a parent or predecessor created before the
  check was on.
- **FR-043-002** A version without `inputSchema` MUST accept any input that passes today's checks (object, size). A
  version with one MUST be validated by the platform's validator over the subset of Technical Plan 4
  (OD-2026-11-14-14): the same meaning with `$schema` absent, draft-07 or 2020-12; `format` an annotation; references
  only local pointers, resolved by the validator; `x-sagat-classification` inert. A schema outside the subset is
  unusable. *Acceptance:* the oracle property test of T043-2 (same verdicts as `jsonschema` over generated schemas of
  the subset, in the three dialect choices), and a vocabulary test that refuses each excluded keyword with its name and
  pointer.
- **FR-043-003** A synchronous refusal MUST answer `422 VALIDATION_FAILED`, `retryable: false`, with `details.reason`
  `RUN_INPUT_INVALID` or `RUN_INPUT_SCHEMA_UNUSABLE`, and for `RUN_INPUT_INVALID` `details.rule` (from the closed enum of
  D043-049) and `details.field` (D043-049's pointer). A start whose input the caller did not send answers `field` `/` and
  `rule` `other` whatever the caller may read, so neither describes a value the caller may not see: a rerun, a batch
  rerun, a reset and a pending reset (D043-121), a schedule's trigger-now (D043-144) and a private rerun of row 13
  (D043-148). The graded-disclosure check that would show the field to a caller who may read the source input is not
  built (G043-040, O043-006). Nothing is created and no idempotency record is written. *Acceptance:* `contract_rest_durable_execution`
  asserts the problem shape for `POST /runs`, the uploaded-input start, Signal-with-Start, Update-with-Start,
  `RERUN_AS_NEW`, reset and the ad hoc execution.
- **FR-043-004** A refused start whose idempotency key already holds a run with the same fingerprint MUST replay that
  run (D043-004). A retry after a promotion of another active version computes another fingerprint, because the
  fingerprint holds the version id (`runs.rs:1538`), and gets the existing `409` for a reused key, as it does today. Ad hoc
  starts replay through their own lookup (D043-029). *Acceptance:* integration tests for the replay, the `409` after a
  promotion, and the ad hoc replay.
- **FR-043-005** Signal-with-Start, Update-with-Start and a start with `USE_EXISTING` MUST be refused when the start
  input does not match, also when an open holder would answer and no run would be created (OD-2026-11-14-10).
  *Acceptance:* with an open holder, an invalid input is refused and the holder receives no signal and no Update.
- **FR-043-006** Schedules. Create and update MUST refuse an input the target version refuses (`SCHEDULE_INPUT_INVALID`,
  `details.field` under `/action/input`), checked with `nominalTimeField` filled with the next nominal time as the first
  action will fill it, and a `nominalTimeField` that is not a declared top-level string property of a version that has
  an `inputSchema` (`NOMINAL_TIME_FIELD_INVALID`); the declaration is read as the validator applies the schema, through
  local `$ref`s and `allOf` members (D043-158). An action MUST be checked before `decide`, so a refused action is never
  buffered and trigger-now answers `422` instead of `BUFFERED`; an action buffered while valid is checked again at its
  release (D043-126). An exhausted schedule is not checked: `decide` answers it `Exhausted` (D043-142). A refused
  action is recorded with outcome `INPUT_INVALID` and the reason (`SCHEDULE_INPUT_INVALID`, `NOMINAL_TIME_FIELD_INVALID`
  or `RUN_INPUT_SCHEMA_UNUSABLE`, D043-123), the time acted on and not owed, no remaining action used up; for a scheduled
  or triggered action the auto-pause (when `pause_on_failure` is set) and its audit, and for a backfill step the
  backfill's end as `CANCELLED` without a pause (D043-134), commit in that same decision (D043-026). A trigger-now
  refused at its start after its check passed (a version promoted in between) returns the refusal and records nothing
  (D043-146). *Acceptance:* `integration_schedules_bulk_operations` for write, action, backfill, trigger-now and
  `BUFFERED` to `INPUT_INVALID`.
- **FR-043-007** A trigger whose run input is refused MUST start nothing, record the delivery to that trigger with no
  run and, only when that record is new, one value-free audit record `run.input_refused`, in one transaction; count
  `trigger_events_total{outcome="input_invalid"}` or `{outcome="input_schema_unusable"}` once per dispatch (D043-129);
  and still serve the event's other consumers. A redelivery of the event finds the delivery and repeats neither
  (D043-027). A cron tick has no delivery: its audit is written in the transaction that moves the trigger past the
  tick, only by the dispatcher whose move found the trigger still at it, so once (D043-156). *Acceptance:*
  `integration_events_webhooks_messaging` with two triggers, one refusing, and a wait; `fault_events_webhooks_messaging`
  makes the other trigger fail so the event is retried, and observes one audit row.
- **FR-043-008** A child run whose input is refused MUST NOT be created; the parent step fails with
  `CHILD_INPUT_INVALID`, or `RUN_INPUT_SCHEMA_UNUSABLE` for a schema that does not compile. *Acceptance:*
  `integration_workflow_primitives`.
- **FR-043-009** A continue-as-new whose evaluated input is refused MUST create no successor; the predecessor ends
  `FAILED` with `CONTINUATION_INPUT_INVALID` or `RUN_INPUT_SCHEMA_UNUSABLE`. *Acceptance:* `integration_workflow_primitives`.
- **FR-043-010** A batch `RERUN` item refused for its input ends `FAILED` with `VALIDATION_FAILED`; a pending reset ends
  `ADMISSION_REFUSED`; neither is tried again (D043-013). *Acceptance:* `integration_schedules_bulk_operations`.
- **FR-043-011** A stored `inputSchema` that fails the pre-check of Technical Plan 4 MUST refuse every start of its
  version with `RUN_INPUT_SCHEMA_UNUSABLE` (OD-2026-11-14-11), in `enforce` mode. The failure is remembered: one
  pre-check per process, tenant and schema while the entry stays cached, except on row 13, which checks on each start.
  *Acceptance:* versions stored (as the owner role, since publication refuses them after FR-043-012) with an excluded
  keyword, a remote `$ref`, a `$ref` cycle, a nested `$id`, a fractional `multipleOf`, a pattern with `\d`, a schema
  deeper than 32 and one with 33 patterns are each refused on every row, by counter one pre-check each on rows 1 to 12
  and 14.
- **FR-043-012** Publication (and `sagat validate`) MUST run the same pre-check on every new `inputSchema`, annotated or
  not: in `enforce` mode a violation refuses the publication with a typed issue that names the keyword and its pointer;
  in `report` mode it is a warning with the same fields (D043-051). An `inputSchema` over 64 KiB or nested deeper than 32
  is refused first by feature 016's classification bound, `CLASSIFICATION_TOO_COMPLEX`, in both modes (D043-112,
  G043-020). A version with an `inputSchema` and a non-manual trigger gets a warning that the schema applies to the event
  envelope. *Acceptance:* `integration_gitops_versioning`,
  `crates/infra-ir/tests/` and the CLI tests.
- **FR-043-013** The mode (OD-2026-11-14-8). `SAGAT_RUN_INPUT_VALIDATION` is parsed strictly: `enforce` or `report`;
  unset or empty is `enforce`; anything else stops the process naming the variable. In `report` every check runs, a
  refusal is counted (`outcome="reported"`) and audited, and the run is created; a start that an open holder answers
  creates no run, and its refusal is audited with the holder's answer (ADR-0092 decision 5, D043-157). A process lists the capability
  `run-input-validation-v1` in the instance registry only in `enforce`. When the operator records it as a requirement
  for the control plane and the execution engine, a 0.2.x binary and a process in `report` are refused by the
  registry (ADR-0050 decision 9, ADR-0082), so replicas cannot answer differently; going back to `report` means removing
  the requirement first. No manifest, chart or compose file this repository ships sets `report`. *Acceptance:* the
  configuration tests of both apps (strict parse, default, the capability listed only in `enforce`), registry tests
  (recording the requirement is refused while a live process runs in `report`; once recorded, a process started in
  `report` stays up with its gate closed), and `no_shipped_manifest_sets_run_input_validation_to_report`.
- **FR-043-014** The value of an input MUST NOT appear in an error, a log line, a span, an audit record or a metric
  label, on any of rows 1 to 14. The validator never formats an input value. Metric labels are closed
  sets (`creator`, `outcome`), with no tenant, namespace, workflow, version, field or rule. *Acceptance:*
  `integration_payload_classification` starts each row with a sentinel value, and a sentinel key under
  `additionalProperties` (the only place a caller's key can reach the subset), in an input that fails at them, and asserts the sentinels are in none
  of the response, the captured logs and spans, the audit rows and the metrics text.
- **FR-043-015** The six SDKs MUST expose `field` and `rule` on their error type beside `reason`; the CLI and the Studio
  show them; the Studio and the SDK schedule views show the `INPUT_INVALID` outcome; `CHILD_INPUT_INVALID` reaches the
  parent's child node, and `CONTINUATION_INPUT_INVALID` and `RUN_INPUT_SCHEMA_UNUSABLE` reach the run view through the
  REST `Run`'s optional `failureCode`, read from the run's stop (`runs.stop_code`), unchanged (D043-162). *Acceptance:*
  the conformance scenario `run-input-validation` in the six languages (the chain's run view shows `failureCode`
  `CONTINUATION_INPUT_INVALID`); Studio and CLI tests; a unit test of each SDK's run parsing.
- **FR-043-016** The architecture check MUST fail on: an `INSERT INTO runs` (the name bare, quoted or qualified by
  `public`) outside its three allowed places (`PgRunStore::create` in `crates/infra-postgres/src/execution.rs`, and the
  legacy and evaluated continue-as-new writes in `crates/infra-postgres/src/chain_progress.rs`, counted per function so
  that a moved line is not a failure), or in any up migration; a call of `.continue_as_new(` (the legacy path) outside
  `tests/`, `crates/*/tests/` and `#[cfg(test)]`, except in the one forwarder, the `continue_as_new` of `PgRunStore`'s
  `ChainProgressStore` implementation in `crates/infra-postgres/src/chain_progress.rs`, exempt by file and function
  (D043-122, D043-160); the `test-support` feature of `sagat-application` enabled anywhere but a `[dev-dependencies]`
  entry. `NewRun.input`,
  `ContinuationSelection` and `EvaluatedChainHandoffCommand.input` (`crates/application/src/chain_progress.rs:246`) hold a
  `CheckedRunInput`, whose fields and only non-test constructor are private to `crates/application/src/run_input.rs`; a
  fourth rule fails on a struct expression or an associated function call of `CheckedRunInput` in any other file
  outside test paths, and on a `Default` or `Deserialize` derive or impl for it anywhere (D043-045); a fifth rule fails on
  `jsonschema` (a dependency path, a `use` or a qualified path) under `crates/infra-schema/src/run_input/` (D043-074);
  a sixth fails when `tokio` is a dependency of `sagat-application` with a default feature or a feature other than `rt`,
  or is named outside `crates/application/src/collection_source.rs` and test modules (ADR-0092 decision 1, D043-154);
  a seventh fails when a composition root under `apps/*/src` builds a `RunService`, `Scheduler`, `EventDispatcher` or
  `ScheduleService` without `.with_run_inputs` in its builder chain (ADR-0092 decision 2, D043-161). *Acceptance:* a
  negative fixture for each of the seven rules in `scripts/tests/`.
- **FR-043-017** A `CheckedRunInput` carries the version id it was checked against. `PgRunStore::create` and
  `continue_evaluated` MUST refuse one whose version is not the run's (`StoreError::Internal`, logged without the value).
  *Acceptance:* a unit test of each store against a mismatched input.
- **FR-043-018** `control-plane input-schema-inventory` MUST list every stored version whose `inputSchema` fails the
  pre-check, with its ids, the keyword and its pointer, and nothing of the schema or of any input; list separately, for
  information, the versions within the subset that have an `inputSchema` and a non-manual trigger; and exit 0 when none
  would be refused, 1 when one would, 2 when the command failed (D043-052, D043-072). *Acceptance:* an integration test
  stores one version inside the subset with an event trigger and one with `patternProperties`, runs the command, finds
  the second as refused and the first as informational, and sees exit status 1; with only the first stored, exit 0;
  against an unreachable database, exit 2.

### Non-functional

- **NFR-043-PERF** In `enforce` mode validation adds no SQL statement to a start, a triggered start, a child start or a
  continuation. In `report` mode a reported refusal writes one audit row, which adds a statement to that start. A
  schema is compiled at most once per process and tenant while its entry stays cached. Compiling and validating never
  run inside a database transaction, and run through `block_in_place` off the async scheduler.
- **NFR-043-BOUND** A compile is bounded by the pre-check (64 KiB of schema, depth 32, an application path of at most
  128, names of at most 1 KiB, an input-independent count within the budget, at most 32 patterns of 16 KiB and 512 NFA
  states each, 256 KiB of programs in all), and what it keeps weighs at most a tenant's share of the cache (D043-105,
  D043-159); a validation by the
  validator's own counter of `RUN_INPUT_WORK_UNITS` (1,000,000) and an input depth of 64, with a 5 s backstop; at most four
  validations run at once per process, shared among tenants by the work-conserving queue of D043-068 within the same
  wait.
- **NFR-043-SEC** No new grant, role membership or `SECURITY DEFINER` function for the runtime role. No tenant code runs
  in the control plane or the engine; no reference is resolved; a compiled schema is shared only within its tenant.
- **NFR-043-COMPAT** The upgrade order of OD-2026-11-14-3 (migrate, engines, control planes) holds; until the operator
  records `run-input-validation-v1`, old and new binaries, and processes in either mode, run side by side (G043-001,
  G043-010).

### Success criteria

- **SC-043-001** Rows 1 to 14 of Technical Plan 1 each have the test of FR-043-001 or FR-043-006, passing on real
  PostgreSQL, for both `RUN_INPUT_INVALID` and `RUN_INPUT_SCHEMA_UNUSABLE` where the row has both.
- **SC-043-002** The sentinel test of FR-043-014 passes for every row.
- **SC-043-003** In `integration_claim_transition_efficiency`, in `enforce` mode, a start, a triggered start, a child
  start and a continuation of a version with an `inputSchema` cost exactly the statements, transactions and rows written
  of the same shape without one.
- **SC-043-004** The cache counters show one compile for 1,000 starts of one version in one process, one for two
  versions of one tenant with byte-identical schemas, two for the same schema in two tenants, and one remembered failure
  for 1,000 starts of a version whose schema does not compile.
- **SC-043-005** `sdk_conformance` with `SAGAT_SDK_SCENARIO=run-input-validation` passes in TypeScript, Go, Python, C#,
  Java and Rust.

### User stories

- **US01** As a caller, I am told at once, with the field and the rule, that my input does not fit the workflow, and
  nothing is started.
- **US02** As an operator, I can see why a trigger, a schedule, a child or a continuation did not start a run, from a
  delivery and an audit row, an action record or a failure code, without reading the input.
- **US03** As an operator upgrading to 0.3.0, I can list the versions at risk, run in `report` while I fix my callers,
  and then require `enforce` across every replica.

## Goals

- One rule for every run creator, enforced by the type system and seven architecture checks rather than by
  convention.
- A refusal that a person or an operator can act on, with no input value in it, that is never retried when it cannot
  pass.
- No cost on the statement budget of design 038 in `enforce` mode.
- Bounded work for a schema the platform did not write.
- An upgrade an operator can prepare for and then lock.

## Non-Goals

- Validating signals, Updates, worker results or activity outputs against a schema (G043-008).
- Asserting `format` or `content*`, resolving references, or a dialect of our own.
- A validation-only endpoint. The Studio form of feature 026 already renders the schema.
- Re-checking stored inputs (schedules, pending resets) at the upgrade (G043-007).
- Editing published versions: a version is immutable (constitution IV). A schema that does not fit is fixed by
  publishing a new version.
- Fixing the instance paths in the script schema errors of feature 005 (G043-009).
- Changing AsyncAPI: no message shape changes; `events.baseline.json` is its test.

## Open Questions

- Whether the inventory command should also try a synthetic envelope against each schema in the subset (G043-004).
- What `RUN_INPUT_WORK_UNITS` costs in time on the CI machine and in production, and whether 1,000,000 is the right
  budget (G043-006). On the development machine a full budget costs at most 7.5 ms in a release build (G043-017); the
  counter, not the clock, is the bound on the work, and 5 s is a backstop.

## Owner decisions needed

The answers are OD-2026-11-14-2, -8, -9, -10, -11, -14, -15 and OD-2026-11-15-3 (`docs/owner-decisions.md`), and each
task that builds one cites it in an `Owner-gate:` line. Open for the owner, neither blocking this design (each states
what is built and its follow-up), both with options and a recommendation in `research.md`:

- O043-005: an `inputSchema` with tuple positions (`prefixItems`), which feature 016 classifies and feature 035 reads for
  the levels of an Iceberg write, is outside the subset, so such a version publishes in 0.3.0 only in `report` mode.
  Recommended: accept for 0.3.0 and add tuples to the subset in a later release.
- O043-006: repeats, trigger-now and the private rerun answer `/` and `other` to every caller (G043-040). Recommended:
  accept for 0.3.0; build the graded-disclosure check of D043-037 only if callers ask for it.
- ADR-0092 is `Proposed`: the owner accepts or refuses it.

## Critical User Journeys (CUJs)

### CUJ 1: A caller sends the wrong input

*   **Given**: a version whose `inputSchema` requires `orderId` as a string.
*   **When**: a caller starts it through the TypeScript SDK with `{"orderId": 42}`.
*   **Then**: the call fails with `VALIDATION_FAILED`, `reason` `RUN_INPUT_INVALID`, `field` `/orderId`, `rule`
    `type`; no run exists; `42` appears nowhere.

### CUJ 2: A schedule whose input no longer fits

*   **Given**: a schedule targeting the active version, with `pause_on_failure`, and a new version published that
    requires a field the schedule does not send.
*   **When**: the next time falls due.
*   **Then**: one decision records the action `INPUT_INVALID` with `SCHEDULE_INPUT_INVALID`, pauses the schedule and
    writes the auto-pause audit; no run starts; the next tick does nothing more.

### CUJ 3: An event trigger on a version written for manual starts

*   **Given**: a version with an `inputSchema` `{"required": ["orderId"]}` and an event trigger.
*   **When**: an event arrives, and is redelivered after another consumer fails.
*   **Then**: no run starts; one delivery without a run and one audit row `run.input_refused` exist; the metric counts
    `input_invalid` once per dispatch; a wait for the same event is still woken.

### CUJ 4: An old run reaches a child

*   **Given**: a run started on 0.2.x whose `child` node evaluates an input the child's version refuses.
*   **When**: the 0.3.0 engine starts the child.
*   **Then**: no child run exists; the parent step fails with `CHILD_INPUT_INVALID`; its `onError` route, if any, runs.
    The release notes said this can happen (OD-2026-11-14-9).

### CUJ 5: The upgrade breaks a caller that cannot be fixed today

*   **Given**: 0.3.0 installed, the requirement `run-input-validation-v1` not yet recorded, and a production caller
    that sends an input its version refuses.
*   **When**: the operator sets `SAGAT_RUN_INPUT_VALIDATION=report` on every replica and restarts them.
*   **Then**: the starts are admitted again; every attempt is counted `reported`, and each created run (or each answer
    of an open holder) is audited once. The final outcomes recorded
    before the switch (an `INPUT_INVALID` action, a failed child step, a trigger's delivery without a run) stay as they
    were recorded; `report` changes only what happens from then on. When the caller is fixed, the
    operator restarts every replica in `enforce` and records the requirement. Recording is refused while any live
    replica still runs in `report` (ADR-0082 decision 2); once it is recorded, a replica restarted in `report` stays up
    with its gate closed.

## Technical Plan (Architecture)

### 1. The creators

Rows 1 to 13 are the thirteen paths that create a run, and all of them are checked (D043-003, OD-2026-11-14-9).
Row 14 is not a creator: the schedule write uses the same check. The unnumbered legacy row is reachable from tests only
and the architecture check keeps it there (FR-043-016). Line numbers are on `develop` at `c2e9341f` and drift as the
code moves; the symbol beside each is what to search for.

| # | Creator | Entry | Where the run is created | Input | `Invalid` | `SchemaUnusable` |
|---|---|---|---|---|---|---|
| 1 | `POST /runs` (SDKs, CLI `apps/cli/src/workflow.rs`, Studio) | `crates/api-http/src/routes/execution.rs:505` → `start_with_options` `crates/application/src/runs.rs:989` | `start_with` `runs.rs:1313` → `store.create` `runs.rs:1725` | caller | `422 RUN_INPUT_INVALID` | `422 RUN_INPUT_SCHEMA_UNUSABLE` |
| 2 | Uploaded-input start | `execution.rs:489` → `start_with_blob` `runs.rs:1150` | as 1; the check at `runs.rs:1673` is replaced by the gate | the blob descriptor | as 1 | as 1 |
| 3 | Signal-with-Start, Update-with-Start | `execution.rs:857`, `:993` → `runs.rs:1010`, `:1057` | as 1 | caller | as 1, also with an open holder | as 1 |
| 4 | `RERUN_AS_NEW`, batch `RERUN` | `execution.rs:1612` → `rerun_item` `runs.rs:1232`; batch `crates/application/src/runs/batch_items.rs:134` | as 1 | the stored input of the ended run | as 1 with `field` `/` and `rule` `other` (D043-121); batch item `FAILED VALIDATION_FAILED` | as 1; batch item as for `Invalid` |
| 5 | Reset, pending reset | `crates/api-http/src/routes/resets.rs:158` → `reset.rs:550`; pass `crates/application/src/runs/pending_reset.rs:424` | `create_from_prefix` `crates/application/src/runs/reset.rs:696` → `start_with` | the stored input of the source | as 4; pending reset `ADMISSION_REFUSED` | as for `Invalid` |
| 6 | Schedule action | `crates/scheduler/src/schedules.rs:278` → `fire` `crates/application/src/schedule_firing.rs:611` → `start` `:1174` | `ScheduledStarts::start_scheduled` `runs.rs:3156` → `start_with` | the schedule's input with `nominalTimeField` (`schedule_firing.rs:1545`) | action `INPUT_INVALID` (`SCHEDULE_INPUT_INVALID` or `NOMINAL_TIME_FIELD_INVALID`), the schedule paused when `pauseOnFailure` | action `INPUT_INVALID` (`RUN_INPUT_SCHEMA_UNUSABLE`), as for `Invalid` |
| 7 | Schedule backfill | `crates/scheduler/src/schedules.rs:168` → `backfill_step` `schedule_firing.rs:405` | as 6 | as 6 | as 6, the backfill ends `CANCELLED` and the schedule is not paused (D043-134) | as 6, the backfill ends `CANCELLED` |
| 8 | Schedule trigger-now | `crates/api-http/src/routes/schedules.rs:489` → `trigger` `schedule_firing.rs:201` → `trigger_once` `:240` | as 6 | as 6 | `422 SCHEDULE_INPUT_INVALID` with `field` `/` and `rule` `other` (D043-144) before `decide`, nothing recorded | `422 RUN_INPUT_SCHEMA_UNUSABLE`, nothing recorded |
| 9 | Event, webhook, message trigger | `crates/scheduler/src/events.rs:294` `dispatch` → `start_run` `:386` → `start` `:407` | `store.create` `events.rs:486` | the event envelope | delivery without a run + one audit + metric | as for `Invalid`, its own reason and metric |
| 10 | Cron trigger (legacy pass until the handover) | `cron_pass` `events.rs:748` → `start` `:809` | as 9 | `{event, data: {scheduledAt, firedAt}}` | as 9 (the firing key stands for the delivery) | as 9 |
| 11 | Child run | `crates/scheduler/src/passes.rs:5142` `start_child` | `store.create` `passes.rs:5302` | the parent node's evaluated input | parent step `CHILD_INPUT_INVALID` | parent step `RUN_INPUT_SCHEMA_UNUSABLE` |
| 12 | Continue-as-new (evaluated) | `advance_continuation_in_view` `passes.rs:775`, input at `:838` | `continue_evaluated` `crates/infra-postgres/src/chain_progress.rs:1494` | the evaluated expression | predecessor `FAILED CONTINUATION_INPUT_INVALID` | predecessor `FAILED RUN_INPUT_SCHEMA_UNUSABLE` |
| 13 | Ad hoc database execution, connection probe, private rerun | `crates/api-http/src/routes/database_executions.rs:103`, `:141` → `crates/application/src/runs/ad_hoc.rs:365` | `store.create` `ad_hoc.rs:523` | caller, or `{}`; a private rerun the stored input | as 1 when the document declares a schema; a private rerun `field` `/` and `rule` `other` (D043-148) | as 1 |
| 14 | Schedule create and update (not a creator) | `crates/application/src/schedules.rs:1301`, `:1560` → `check` `:1240` | none | the schedule's input with `nominalTimeField` filled with the next nominal time (D043-158) | `422 SCHEDULE_INPUT_INVALID`, `NOMINAL_TIME_FIELD_INVALID` | `422 RUN_INPUT_SCHEMA_UNUSABLE` |
| — | Continue-as-new (legacy, tests only) | `chain_progress.rs:1149` | `chain_progress.rs:1267` | the predecessor's input, byte for byte | not checked | not checked |

The refusals of the creators that nobody waits for (the batch item of row 4, the pending reset of row 5, rows 6, 7
and 9 to 12, all run by the engine) are final: nothing tries again an input or a schema the version refuses
(OD-2026-11-14-11). An input past the work budget is final the first time too. The transient cases are a busy or
starved machine, never the input: no permit within the wait (`Busy`, which is also what a service built without a gate
answers, D043-120), a validation stopped by the backstop, and a compile that panicked (`Interrupted`, D043-106). A
caller gets a retryable `503`; the engine's creators try again later. An event that waits so is waiting for room, as
for a full quota: it uses up none of its attempts, and only an event that has waited for a day (`GIVE_UP_AFTER` of
`crates/scheduler/src/events.rs`) ends `DEAD`, so only a validator starved for a day drops a trigger start (G043-054). Not found on `develop`: a whole-run retry or replay, a script test run (it writes
`script_test_runs`), a gRPC start (`crates/api-grpc` is worker-only), internal apps (027) and cross-namespace services
(030). When one is built, the type of FR-043-016 makes it go through the gate.

### 2. The schema travels with the definition

The IR codec (`crates/infra-ir/src/lib.rs:154`, `parse_stored`, and its private twin) puts the canonical bytes of
`inputSchema` into the parsed `WorkflowDefinition` as an `InputSchema`: an `Arc<[u8]>` and a `[u8; 32]` digest. The
codec in `infra-ir` computes the SHA-256 once at parse; the domain only stores the bytes and the digest, with no
`serde_json` and no hashing crate (`sha2` is not in `DOMAIN_ALLOWED`, `scripts/check-architecture.py:31`). `WorkflowDefinition::footprint` counts the bytes, so the
`DefinitionCache` byte budget (`crates/application/src/execution.rs:1951`) still bounds memory. Every creator holds the
definition when it needs the schema: rows 1 to 8, 13 and 14 through `get_version_by_id` and `get_or_parse`
(`runs.rs:1402`, `:1408`; for row 14, `target_check`), rows 9 and 10 at `events.rs:416`, row 11 at `passes.rs:5224`,
row 12 through the run transaction's `tx.parts()` before it commits (`passes.rs:832`). No creator on a budgeted path
reads the database for the schema. Two paths off the budget do: the schedule write of row 14 reads the target version's
document (`ScheduleStore::target_version`, one statement, D043-125), and the check before `decide` of rows 6 to 8
resolves the version again as a start does (`ScheduledStarts::check_scheduled`, D043-126, G043-041).

### 3. The gate, the validator and the type

Two pieces, so that the port is narrow and the policy is in one place (D043-017):

```
// crates/application/src/run_input.rs — the port, implemented in crates/infra-schema, which re-exports its types (D043-127)
trait SchemaValidator: Send + Sync {          // object-safe: held as Arc<dyn SchemaValidator> (D043-076)
    // async; the adapter owns the permits, the wait, the off-worker step and the backstop (D043-071)
    fn check<'a>(&'a self, key: SchemaKey, schema: &'a InputSchema, input: &'a [u8])
        -> Pin<Box<dyn Future<Output = Result<(), SchemaRefusal>> + Send + 'a>>;
}
struct SchemaKey { tenant: TenantId, digest: [u8; 32], shared: bool }   // built by the gate; shared = creator != AdHoc
enum SchemaRefusal { Invalid { segments: Vec<PathSegment>, rule: Rule }, Unusable(Unusable),   // final
                     TooDeep, WorkLimit,                                                       // final
                     Busy, Backstop, Interrupted }     // transient: no permit, the clock, a compile that panicked

// crates/application/src/run_input.rs — the concrete gate
struct RunInputGate { validator: Arc<dyn SchemaValidator>, mode: Mode }   // no runtime, no timer, no permit; not generic
impl RunInputGate {
    fn unwired() -> Arc<Self>;          // what a service holds until it is given a gate: Busy for every schema (D043-120)
    async fn check(&self, tenant, version: WorkflowVersionId, definition: &WorkflowDefinition,
                   input: Vec<u8>, creator: Creator, field_policy: FieldPolicy)
        -> Result<CheckedRunInput, InputRefusal>;
    async fn check_schema(..., schema: Option<&InputSchema>, ...) -> ...;   // a continuation holds only the schema
    async fn recheck(...) -> ...;        // a schedule's start, checked and counted before its decision (D043-147, D043-152)
}
struct CheckedRunInput { version: WorkflowVersionId, bytes: Vec<u8>, outcome: Checked }  // fields and constructor private to run_input.rs
enum Checked { NoSchema, Valid, Reported(InputRefusal) }
enum InputRefusal { Invalid { field: Pointer, rule: Rule }, SchemaUnusable, Unavailable }
enum Creator { Api, Repeat, Schedule, Trigger, Child, Continuation, AdHoc }      // the metric label
enum FieldPolicy { Show, Root }      // Root: `/` and `other`, for every start whose input the caller did not send
```

- **The gate** owns the mode (Technical Plan 6), the metrics (7) and the only non-test constructor of
  `CheckedRunInput`. The type's fields and its constructor are private to `crates/application/src/run_input.rs`, so no
  other module of `application` can build one, and the architecture check fails on a struct expression or an associated
  function call of `CheckedRunInput` anywhere else, and on a `Default` or `Deserialize` for it (FR-043-016, D043-032,
  D043-045). A test constructor is behind a `test-support` cargo feature of `sagat-application`; the architecture check
  fails when that feature is enabled outside `[dev-dependencies]`. The gate builds the `SchemaKey` from the tenant, the
  digest the `InputSchema` carries and the creator (`shared` is false for `Creator::AdHoc`, D043-041). The digest is the
  codec's: `InputSchema::new` is public, since the domain cannot hash (`sha2` is not in `DOMAIN_ALLOWED`), and a digest
  that does not match its bytes can only mis-key the cache within its own tenant, because the tenant is in the key.
- **A service without a gate.** `RunService`, the `Scheduler`, the `EventDispatcher` and the `ScheduleService` hold
  `RunInputGate::unwired()` until a composition root gives them one with `.with_run_inputs`. Its port answers `Busy` for
  every schema, so a start of a version with an `inputSchema` is refused as transient (`503`, or tried again by the
  engine) and is never admitted unchecked; a version without one passes as it would through a wired gate (D043-120). A
  composition root that forgets the gate would leave an engine creator retrying forever, so the seventh rule of
  FR-043-016 fails on it (ADR-0092 decision 2, D043-161). A `ScheduleService` without a gate checks a write only at
  action time, which is the authority (D043-008, D043-125).
- **The type** carries the version id; `PgRunStore::create` (`crates/infra-postgres/src/execution.rs:1108`) compares it
  with `NewRun.info.version_id`, and `continue_evaluated` with the predecessor's version (FR-043-017). The existing
  `RunInputChecker` (`crates/application/src/blobs.rs:284`) is removed.
- **The bound on the work is the validator's own counter** (D043-047). The validator of Technical Plan 4 charges units as
  it runs and stops at `RUN_INPUT_WORK_UNITS` (1,000,000). A budget miss is deterministic for a schema and an input, so
  it is final the first time, on every row: `RUN_INPUT_INVALID` with `rule` `workLimit` and `field` `/`. No static
  estimate of the work is needed or made: the counter is exact because it counts what the validator does.
- **Where the runtime lives** (D043-071, ADR-0092). The gate is plain async code over the port and names no runtime.
  The permits, the wait, the off-worker step and the backstop are inside the adapter, `crates/infra-schema`, which has
  tokio (`sync`, `time`, `rt-multi-thread`) as a normal dependency, as other adapters have; the adapter has its own
  `off_worker`, a twin of the private helper of `crates/infra-postgres/src/payload_disclosure.rs:561`, and there is no new
  edge into `application`. `sagat-application` itself depends on tokio with no default features and the `rt` feature
  alone, for one blocking parse of design 042 in `collection_source.rs`, the only file of the crate that may name tokio
  outside test modules (ADR-0092 decision 1, D043-150, D043-154); moving that parse behind a port would restore the
  stricter rule (G043-052).
- **Threads and permits** (D043-050, D043-068, D043-140), in the adapter. Four permits per process
  (`RUN_INPUT_PERMITS`), one pool however many services hold the gate (`RunInputValidator::process()`), shared
  work-conservingly among tenants: a free permit goes to the waiting tenant that holds the fewest, FIFO among equals, so
  a tenant is granted a third or a fourth permit only while no other tenant waits. A tenant alone on the process can use
  all four; when a second arrives it is served first as permits come back, and nothing held is taken away (each
  validation is bounded by the counter, so it returns soon). A waiter waits at most `RUN_INPUT_PERMIT_WAIT` (5 s); its
  guard leaves the queue, or returns its permit, when its future is dropped. The queue holds one entry per caller that
  waits, so it is bounded by the callers in flight and each entry by the wait. The validator runs off the async workers
  and holds its permit until it returns. Appendix A has the details the bound rests on.
- **The backstop** (D043-050). The counter also reads the clock every 4,096 units; past `RUN_INPUT_BACKSTOP` (5 s) of
  work it stops with `SchemaRefusal::Backstop`. `Busy`, `Backstop` and `Interrupted` become `InputRefusal::Unavailable`
  in the gate: a caller gets a retryable `503`, and the engine's creators treat it as a lack of room and try again later,
  as they do for a full quota. All three mean a busy or starved machine, not an input, and are never remembered or
  cached. A compile that panics for a given schema would panic again, so such a schema is tried again on every start
  (G043-053).
- **Input depth.** Before validating, the input is parsed with a depth limit of 64. A deeper input, and one deeper than
  `serde_json` parses (its recursion limit is 128), is `Invalid` with `rule` `maxDepth` and `field` `/`, never an
  internal error. A version without a schema keeps today's checks only.

### 4. The validator

`InputSchemas` in `crates/infra-schema/src/run_input/` is a validator of the platform for a documented subset of JSON
Schema (OD-2026-11-14-14, D043-046 to D043-049). `jsonschema` is not on this path; it is the oracle of a test only.

**The subset.** The same meaning when `$schema` is absent, draft-07 (`http://json-schema.org/draft-07/schema#`) or
2020-12 (`https://json-schema.org/draft/2020-12/schema`); any other `$schema` is refused.

| Group | Keywords | Notes that keep the drafts equal |
|---|---|---|
| Structure | `type` (a name or a list of names), `properties`, `required`, `items` (one schema), `additionalProperties` (boolean or schema) | `items` as an array (draft-07 tuples) and `prefixItems` are refused; `integer` accepts a number with no fractional part, as both drafts do |
| Values | `enum`, `const` | equality of JSON values, numbers compared by value |
| Numbers | `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf` | the exclusive bounds are numbers (the boolean form of draft-04 is refused); `multipleOf` is a positive integer (a fractional divisor is refused: floating-point remainders differ between implementations, G043-015) |
| Lengths | `minLength`, `maxLength`, `minItems`, `maxItems`, `minProperties`, `maxProperties` | string length in Unicode code points |
| Pattern | `pattern` | parsed with `regex-syntax` and checked node by node against an allowlist (D043-055): literal characters, escaped metacharacters, `\n`, `\r`, `\t`, `\xHH`, bracket classes of literals and ranges (negated too; a class set operation `&&`, `--` or `~~` and a nested bracket are refused, because ECMA-262 reads them as literals; a `]` as the first item of a class, as in `[]a]`, and `[^]` are refused, because ECMA-262 reads `[]` as an empty class and `[^]` as any character, D043-070), `.`, `^`, `$`, alternation, groups and non-capturing groups `(?:...)`, and the repetitions `*`, `+`, `?`, `{n}`, `{n,}`, `{n,m}` (greedy or lazy, bounds at most 100); everything else is refused, among it `\d`, `\D`, `\w`, `\W`, `\s`, `\S`, `\b`, `\B` (Unicode classes in the `regex` crate, ASCII in ECMA-262), inside brackets too, `\p{...}`, POSIX classes `[[:alpha:]]`, flags `(?i)`, `\A`, `\z`, named groups `(?P<...>` and `(?<...>`, backreferences and lookaround. `.` is compiled as "any character but `\n`, `\r`, U+2028 and U+2029", its ECMA-262 meaning. Unanchored search, as both drafts specify. Characters are code points, as in the `jsonschema` crate (G043-016) |
| Combinators | `allOf`, `anyOf`, `oneOf` | each a non-empty array of schemas |
| References | `$ref` to a JSON pointer into the document (`#/...`), decoded by the validator itself (D043-056), whose target is a schema position the vocabulary walk recorded; `$defs` and `definitions` as containers | a target that is not a schema position (`#/enum/0`, `#/default`, `#/examples/0`, an `x-sagat-classification` value, a `properties` map itself) is refused; no cycle; a `$ref` with sibling keywords other than annotations is refused (draft-07 ignores them, 2020-12 applies them) |
| Ignored | `title`, `description`, `default`, `examples`, `$comment`, `format` (an annotation); `$schema` and `$id` at the root of the `inputSchema` only (anywhere else they are refused) | the walk does not descend into the values of `enum`, `const`, `default`, `examples` and `x-sagat-classification`: they are data, not schemas |
| Platform | `x-sagat-classification` | the only `x-` keyword the platform reads in a schema (`crates/infra-ir/src/carrier_schema_check.rs:8`); inert to validation, as `annotation_is_inert.rs` checks |

**Out of the subset** (D043-046): every other keyword, among them `patternProperties`, `uniqueItems`, `not`,
`if`/`then`/`else`, `contains`, `dependencies`, `dependentRequired`, `dependentSchemas`, `unevaluatedProperties`,
`unevaluatedItems`, `additionalItems`, `propertyNames`, `prefixItems`, `$dynamicRef`, `$recursiveRef`, `$anchor`,
`$dynamicAnchor`, a nested `$id` or `id`, a remote `$ref`, and any `x-` keyword but the one above. `patternProperties`,
`uniqueItems` and `not` could be bounded (a charge per key and pattern, n·log n, one application), but each adds a
semantic the property test must cover and none is needed by the platform's own documents; they are left out of 0.3.0 and
can be added later without breaking a published schema.

**The pre-check** (D043-048, D043-054 to D043-057, D043-062, D043-063, D043-066, D043-069, D043-070, D043-101, D043-102,
D043-104), before anything is compiled, at publication, at start (cached) and in the inventory:

1. At most 64 KiB of schema and a JSON depth of 32; a value too deep inside a data keyword is reported with that keyword
   and its pointer, never with the keys of the data (D043-104).
2. A vocabulary walk from the root records every schema position: the root, each value of `properties`, `$defs` and
   `definitions`, the value of `items` and of an object `additionalProperties`, and each member of `allOf`, `anyOf` and
   `oneOf`. At a schema position every key must be in the vocabulary. The keys of `properties`, `$defs` and
   `definitions` are names, not keywords, so a property called `id` or `$ref` is legal. The walk does not descend into
   data values (`enum`, `const`, `default`, `examples`, `x-sagat-classification`).
3. Every `$ref` decodes to a recorded schema position (D043-056); the schema is compiled into an arena of nodes, one per
   schema position, and a `$ref` becomes the index of its target node, never an inlined copy, so a DAG of references
   stays the size of the document.
4. A depth-first search over the application edges (references and every child schema) refuses a cycle, and stores
   with each node its longest application path and a lower bound, in units of the counter, of what any input pays at
   that node (D043-069; Appendix A). The bound counts only charges that do not depend on the input's type: the
   application itself, its `$ref` target's bound, one unit per `type` name, one per `enum` member, one for `const`, the
   sum of the bounds of `allOf` branches, the minimum of the bounds of `anyOf` branches (evaluation may stop at the first
   match), and for `oneOf` its cheapest branch plus two units for each other branch, which can fail at its first keyword
   (D043-101). A path longer than 128 is refused (`INPUT_SCHEMA_TOO_DEEP`), so the validator's own recursion is bounded
   by the schema; a root whose bound passes `RUN_INPUT_WORK_UNITS` is refused too (`INPUT_SCHEMA_TOO_COSTLY`), because
   every input would end in `workLimit`: a ×2 `allOf` tree of depth 10 whose leaves point to one `enum` of 1,000 members
   (over 1,024,000 units) cannot be published. Both are listed by the inventory.
5. Each pattern passes the syntax allowlist and, compiled, stays within a 16 KiB program and 512 NFA states; at most
   32 patterns, and at most 256 KiB of compiled programs for the whole schema.
6. A name of `properties` or `required` is at most 1 KiB (`INPUT_SCHEMA_NAME_TOO_LONG`, D043-102): a lookup compares
   names and is charged by their length.

The first violation is `SchemaRefusal::Unusable` with the keyword and its JSON pointer in the schema (author
vocabulary, never input), truncated to 512 characters (D043-104). There is no estimate of the work.

**The counter** (D043-047, D043-058). Every loop step and every subschema application costs at least one unit, so work
is proportional to units whatever the keyword holds:

- one unit per subschema application, an empty schema `{}` included, and per `$ref` followed;
- one unit per name of `required`, per name of a `type` list, per branch of `allOf`, `anyOf` and `oneOf` (plus the
  branch's own application);
- for each instance member, its `properties` lookup charged `log2(n) × (1 + length / 64)` units, the comparisons it
  makes and the bytes each compares (D043-102), and one unit for its `additionalProperties` decision; one per array
  element for `items`;
- for `enum`, one unit per member plus the smaller of the member's size and the value's size; for `const`, the smaller
  of the two sizes; measuring the input stops before it passes the cap, and its steps are charged (D043-100);
- for `multipleOf` on an integral float, the remainder is computed exactly in steps logarithmic in the exponent and
  charged so (D043-108);
- for a length check, the length in code points; for a `pattern` match, (length in bytes + 1) × NFA states, charged
  before the match runs (D043-063). The PikeVM and the backtracker do work proportional to the states per byte, and the
  lazy DFA may never be built, so no bit-parallel divisor is assumed. A divisor smaller than the states is used only if
  the T043-2 measurement sets it, recorded with k in G043-017. With this charge the state cap is lowered to 512: a
  64-character string against a 512-state pattern costs about 33,000 units, and a 256 KiB string against any pattern of
  at least 4 states stops at the budget. Within one subschema the length checks run before `pattern`, so a long string fails on its
  length first; authors pair `pattern` with `maxLength`.

A charge that would pass the budget stops the whole validation before the step, with `workLimit`: it is never the
failure of one branch, so `oneOf: [cheap match, costly match]` is refused with `workLimit`, never accepted because the
costly branch "failed". An adversarial test per keyword
counts the validator's primitive operations (lookups, comparisons, regex steps) under `test-support` and asserts they
stay within a constant multiple of the units charged.

**Determinism** (D043-059). No iteration depends on a hash seed, so every replica reports the same first violation,
`field` and `rule` (Appendix A).

**Cache** (D043-064, D043-105, D043-159). The classification cache (`crates/infra-schema/src/classification/cache.rs`),
made generic over key and value, holds the compiled form (resolved references, compiled patterns), keyed by
`(tenant, digest)`: identical schemas of one tenant share an entry, two tenants never do. 256 entries and 64 MiB, each
entry weighed with its compiled programs, the runtime caches its patterns can hold for the four permits' matchers and
the heap of its literals and names. One tenant holds at most a quarter of the cache, by count and by weight: an entry of
a tenant at its share evicts that tenant's own oldest. Every schema the pre-check admits weighs no more than that share
(the heaviest, 32 patterns at the most states 256 KiB of programs allow and an `enum` filling the rest of 64 KiB,
weighs 7.4 MB of 16.8 MB), so it is compiled once and kept (ADR-0092 decision 3); the engine settings that make that so
are in Appendix A. Single flight per key, and a remembered `Unusable` per key (1,024 at most). Ad hoc keys
(`shared: false`, row 13: a new version id per call at `ad_hoc.rs:459`) are compiled for the call and not cached, under
a permit like every other key (G043-031).

**The refusal** (D043-049). The validator reports its first violation as a closed `Rule` named by the keyword that
failed (the subset's keywords, `workLimit`, `maxDepth`, `other`), with the instance pointer it built: a segment for a
member matched through `properties` and for an index matched through `items`, nothing for a member reached through
`additionalProperties`, whose key the caller chose. A `required` violation appends the missing name, which the schema
holds. The validator never formats an input value.

**The oracle test** (T043-2, D043-060). A property test generates schemas of the subset and inputs, and checks that the
platform's validator and `jsonschema` 0.58.6 (formats off, the refusing retriever) give the same verdict, valid or not,
in each of the three dialect choices. Cases where the platform answers `workLimit`, `maxDepth` or the backstop are
skipped and the skip ratio is reported; the test fails above 5%. Numbers are generated as i64, u64 above `i64::MAX`,
integral floats, `-0.0`, and integers beyond u64. The validator compares `enum` and `const` by mathematical value, never
with `serde_json`'s `PartialEq` (for which `1 != 1.0`). Integers beyond u64 become f64 when parsed, because the server
does not enable `arbitrary_precision`, and both validators read the same f64, so they round them the same way. It is a
regression check of the subset's semantics, not a proof of equivalence. Adversarial cases (wide
`anyOf` over deep inputs, chains of `$ref` through `allOf`, long strings against patterns) must stop at the budget
within a bounded time on the CI machine.

### 5. What each creator does with a refusal

- **Rows 1 to 5, 8, 13 (a caller waits).** `AppError::InvalidRunInput { reason, field, rule }` (D043-014), mapped in
  `crates/api-http/src/error.rs` beside `InvalidStart` (`:854`) to `422 VALIDATION_FAILED`, with remediation lines beside
  `SCHEDULE_INPUT_INVALID` (`:187`); `InputRefusal::Unavailable` (no permits within 5 s, the
  backstop) maps to `AppError::Unavailable`, and a budget miss to `InvalidRunInput` with `workLimit`. Before answering a
  refusal, final or transient, `start_with` looks the key up and answers a run of the same key and fingerprint as any
  replay is answered (D043-004, D043-141, D043-151): what a with-start delivered or applied, "already acted" for a
  schedule, "already completed" for a pending reset; `start_private` has its own lookup (D043-029). Rows 4 and 5, the
  trigger-now of row 8 and the private rerun of row 13 use `FieldPolicy::Root`: `field` `/` and `rule` `other`, whatever
  the caller may read (D043-121, D043-144, D043-148; G043-040). `error_code` (`batch_executor.rs:134`) and `failure_of`
  (`reset_completion.rs:220`) name the new variant.
- **Rows 6 to 8 (schedules).** `trigger_once` (`:240`) and the scheduled path check the input before `decide` through
  `ScheduledStarts::check_scheduled`, which resolves the version and its definition as a start does (D043-126); the start
  then checks again, against the version resolved then, with `RunInputGate::recheck`, which counts only a refusal
  (D043-147, D043-152). An exhausted schedule is not checked (D043-142). A refused trigger-now answers `422` and records
  nothing, also when the refusal comes at its start (D043-146). A refused scheduled or triggered action records, in one
  `ScheduleDecision`, the action (`Outcome::InputInvalid`, the reason of D043-123), the auto-pause and its
  `ScheduleAutoPaused` audit when `pause_on_failure` is set; a refused backfill step records the action and the
  backfill's end as `CANCELLED`, as `revoke_authority` does (`schedule_firing.rs:1464`), and does not pause the schedule
  (D043-134). The arm sits before `NotAllowed` (`:1203`), so nothing falls to `Err(other)` (`:1214`). A `BUFFERED` action
  is checked again at its release and may become `INPUT_INVALID`; the waiting row is settled where it stands
  (`settle_waiting`, D043-126). Two migrations add `INPUT_INVALID` to the action outcomes and `run-input-validation-v1`
  to the registry's capability lists and its requirement history (D043-131): `20270118000000` drops each old CHECK in
  the `ALTER TABLE` that adds its successor `NOT VALID` (a superset, named `*_known`), and `20270118000100` validates
  them; a static test keeps every later CHECK added `NOT VALID` (D043-145, D043-153). Their down migrations refuse while
  a row holds a new value.
- **Row 14 (schedule write).** `ScheduleService::with_run_inputs` gives the write check the gate and a definition cache
  (D043-125); `check_input` (`crates/application/src/schedules.rs`) reads the target version's document
  (`ScheduleStore::target_version`), fills `nominalTimeField` with the next nominal time, as the first action will
  (D043-008, D043-158), runs the gate, and then requires the field to be a declared top-level string property, read
  through local `$ref`s and `allOf` members. The write check is never stricter than the action check but in the one
  rule feature 018's contract states: the field must be declared. The write path is not on a budget.
- **Rows 9 and 10 (triggers).** In `events.rs` `start` (`:407`), after `get_or_parse`: one store call writes, in one
  transaction, a delivery to `trigger:<id>` with no run (`event_deliveries.run_id` is nullable,
  `crates/infra-postgres/migrations/20261012000000_events.up.sql:89`) and, only when that row is inserted, the audit
  record `run.input_refused` (resource the workflow version, the reason `<reason> rule=<rule> via trigger <id>`, D043-128);
  then the metric, once per dispatch (D043-129), and `Ok(Started::No)`. A dispatch that waits for room in the validator
  is recorded on the event as "the run input could not be checked now; will retry". A retry
  of the event after another consumer failed (`events.rs:304-334`) finds the delivery and writes nothing. A cron firing
  has no inbox event; its refusal audit names the tick's key (`tick_identity`, `events.rs:801`) in its reason (the
  audit's own key; there is no miss memory), and
  the audit is written in the transaction that moves the trigger past the tick, only by the dispatcher whose move
  found the trigger still at that tick, so the tick is audited once however many dispatchers reach it and wherever one
  stops (D043-156, closing G043-002).
- **Row 11 (child).** In `start_child` after `get_or_parse` (`passes.rs:5224`), on a refusal the engine first looks up
  the child's idempotency record (scope `child:{parent}`, key `child-start:{node}`, `passes.rs:5302-5306`): a child that a
  peer, a 0.2.x engine or an engine in `report` already created is answered as `Applied::Replayed` is today, and the
  step is not failed (D043-038). The lookup is `RunStore::replay_start` under the child's own idempotency, as
  `system:engine`: one statement on the refusal path only (D043-132). Otherwise `finish_with(...,
  ChildOutcome::Failed(code))`, as for `CHILD_NOT_FOUND` (`:5217`).
- **Row 12 (continuation).** After the evaluation (`passes.rs:838-866`), with `check_schema` on the schema read under the
  run's lock: a refused input ends the predecessor through the fenced path of `finalize_continuation_expression`
  (`:982`), given the failure code, and counts `scheduler_continuation_input_refusals_total`; an input that cannot be
  checked now is `ContinuationAdvanceError::InputUnavailable`, counted as a wait
  (`scheduler_continuation_waits_total{blocker="input_check"}`), and left for the next pass (D043-133). The failure
  code is the run's stop code, which the REST `Run` shows as `failureCode` (D043-162).

### 6. The mode and the registry

`SAGAT_RUN_INPUT_VALIDATION` is read once at start by `apps/control-plane` and `apps/execution-engine`, strictly (the
ADR-0081 pattern): `enforce`, `report`, unset or empty meaning `enforce`, anything else a start-up error naming the
variable. The process logs its mode. Only in `enforce` does it list `run-input-validation-v1`
(`apps/execution-engine/src/capabilities.rs:23` and the control plane's twin); the capability is added to
`Capability` (`crates/application/src/instances.rs:108`) and to the two closed lists of the registry
(`crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql:42-54`) in the same migration as the
action outcome, and to the registry's data model (`specs/018-schedules-bulk-operations/data-model.md`). The operator
records the requirement for `CONTROL_PLANE` and `EXECUTION_ENGINE` once every live process of those kinds lists the
capability: ADR-0082 decision 2 verifies every live instance and refuses to record a requirement while one lacks it, so
the requirement cannot be recorded while a replica runs in `report` or a 0.2.x binary is live. From then on a process
started in `report`, or a 0.2.x binary, stays up with a closed gate (ADR-0082). Going back to `report` is: remove the
requirement, then restart in `report`. In `report` mode a refusal becomes `Checked::Reported`, is counted, and its
audit record, `run.input_refused` on the run with the reason `reported: <reason> rule=<rule>` (D043-128), is written in
the transaction that creates the run (one statement on that start), so a retry or a replay of the same start does not
audit twice (D043-039); for a continuation, in the hand-off transaction (D043-143). A start that an open holder answers
creates no run: its audit is written with the receipt a Signal-with-Start or an Update-with-Start keeps, so once across
its retries, and for a plain start answered by its holder, which keeps no receipt, in a statement of its own with each
answer (ADR-0092 decision 5, D043-157).

Because the engines are upgraded first and default to `enforce`, an operator who wants `report` for the upgrade sets
`SAGAT_RUN_INPUT_VALIDATION=report` in the manifests of every engine and control plane **before** migrating, after
running `control-plane input-schema-inventory`; otherwise the first upgraded engine records final outcomes (rows 4 to 12)
before anyone can choose (D043-040).

### 7. Telemetry

`run_input_validations_total{creator, outcome}` with `outcome` in `accepted`, `no_schema`, `refused`,
`schema_unusable`, `reported`, `deadline` (a transient refusal), counted once per start (a recheck counts only a
refusal, D043-147, D043-152); `run_input_schema_cache_total{result}` with `hit`, `miss`, `evicted`,
`failure_remembered`, `unshared`; `trigger_events_total{outcome}` gains `input_invalid` and `input_schema_unusable`
(D043-129); `scheduler_continuation_input_refusals_total` and `scheduler_continuation_waits_total{blocker="input_check"}`
(D043-133). A `run_input.check` span with the attribute `sagat.run_input.outcome` and, on a refusal,
`sagat.run_input.rule`.
Logs at `debug` name the version id, the creator and the rule. None carries a value, a field, a tenant or a namespace
label (constitution V and VI).

### 8. Publish and the inventory

`validated_document_in` (`crates/infra-ir/src/lib.rs:326`) runs the pre-check of Technical Plan 4 on every
`inputSchema`, annotated or not. The codec takes the mode as a parameter (D043-073): the control plane passes the mode of
its process (`SAGAT_RUN_INPUT_VALIDATION`), and `sagat validate` takes `--input-validation enforce|report`, `enforce` by
default. In `enforce` mode a violation refuses the publication with an `IrIssue` that names the
keyword and its pointer (`/inputSchema/...`), as a closed issue `INPUT_SCHEMA_KEYWORD_UNSUPPORTED`,
`INPUT_SCHEMA_REFERENCE_INVALID`, `INPUT_SCHEMA_PATTERN_UNSUPPORTED`, `INPUT_SCHEMA_TOO_LARGE`, `INPUT_SCHEMA_TOO_DEEP`,
`INPUT_SCHEMA_TOO_COSTLY` or `INPUT_SCHEMA_NAME_TOO_LONG` (D043-065, D043-102); in `report` mode it is a
`DefinitionWarning` with the same fields and the version is published (D043-051). An `inputSchema` over 64 KiB or
nested deeper than 32 never reaches the pre-check at publication: feature 016's classification bound reads every
carrier first and answers `CLASSIFICATION_TOO_COMPLEX`, in both modes; the start gate and the inventory answer the
validator's own codes (D043-112, G043-020). `sagat validate` runs the same check locally. The stored-document
path (`validated_stored_document_in`, `:344`) does not, so a version already stored still parses and is refused at start
instead (FR-043-011). A `DefinitionWarning` is also added when a version has an `inputSchema` and a non-manual trigger.
The versioning diff already reports a tighter `inputSchema` as breaking (`crates/infra-ir/src/diff.rs:178`).

**The inventory** (D043-052) is a command of the control plane, `control-plane input-schema-inventory`, beside
`migrate` and `schedule-trigger-handover` (`apps/control-plane/src/cli.rs:57-62`). It runs with the database
settings of the control plane process (`config.database`, the runtime role where a deployment sets one, D043-163): it
only reads, and needs nothing but `SELECT` on `workflow_versions`, which the runtime role holds. It reads `workflow_versions` page by page (its cursor starts after the nil uuid,
in one statement each page), runs the same pre-check on each stored `inputSchema` and prints one line per version that
would be refused: tenant, namespace, workflow and version ids, the keyword and its pointer. The keyword and the pointer
are author text, so they are printed in double quotes with `"` and `\` escaped (D043-111). In a second, informational section it lists the versions within the subset that have an `inputSchema` and a
non-manual trigger, whose triggered runs may be refused (G043-004); they do not change the exit status. It prints no
schema text beyond those keywords and pointers, no input, and writes nothing. It exits 0 when no version would be
refused, 1 when at least one would, and 2 when the command itself failed (it could not connect, a read failed, or a
stored document is not a JSON object, which it still reports beside what it could read), so an upgrade or CI script
can tell "found" from "failed" (D043-072, D043-111). The release notes and the
runbook tell the operator to run it before the upgrade.

### 9. Contracts

- `specs/001-platform-foundation/contracts/openapi.yaml`: the action outcome enum (`:8080`) gains `INPUT_INVALID`; the
  shared `Error` schema carries design 043: the list of `details.reason` refinements (`:4905-4915`) gains
  `RUN_INPUT_INVALID` and `RUN_INPUT_SCHEMA_UNUSABLE` (`VALIDATION_FAILED`, 422, `retryable: false`), `details.rule` is a
  closed enum, and `details.field` is described as a pointer that never holds a caller's key (D043-135). The `422` of
  the six start operations (`startRun`, `signalWithStart`, `updateWithStart`, `operateRun` for the rerun, `resetRun`,
  `startDatabaseExecution`) names the two reasons, and the pinned baselines of features 016 and 035 were re-pinned once
  after reading their diff (OD-2026-11-15-3, D043-149); the schedule writes, trigger-now and the publication are
  described by the shared `Error` schema and `errors.md`, since their operations stay pinned. The `Run` schema gains the
  optional `failureCode`, read from the run's stop (FR-043-015, D043-162); the pinned baselines digest operations only,
  so it moves none of them.
- `specs/043-run-input-validation/contracts/errors.md` (new) with the reasons, the failure codes, the publication issue
  codes and the remediation lines. It notes that `INPUT_INVALID` has two meanings: the schedule action outcome of design
  043, and the worker step failure code of `sdks/SPEC.md`, which is older and unrelated. The `SCHEDULE_INPUT_INVALID` row
  of `specs/018-schedules-bulk-operations/contracts/errors.md:15` points to it and gains `RUN_INPUT_SCHEMA_UNUSABLE` at
  action time.
- `specs/016-payload-classification/contracts/README.md` (section 1.3, rule 6) says that an `inputSchema` carrier is
  also held to design 043's subset: `prefixItems` and the other applicators that rule lets appear in a carrier make an
  `inputSchema` unusable for starts and are refused at publication in `enforce`; output and state carriers are
  unchanged (O043-005).
- AsyncAPI is unchanged; `events.baseline.json` is its test.
- `sdks/SPEC.md` and the six SDK error types gain `field` and `rule`, and the six SDK `Run` types `failureCode`;
  `sdks/error-catalog.json` keeps its codes.

### 10. Rollout

Before the upgrade the operator runs `control-plane input-schema-inventory` and, if the answer warrants it, sets `report` everywhere
(Technical Plan 6). Upgrade order (OD-2026-11-14-3): the migration first; then the engines (the batch items of row 4,
the pending resets of row 5, rows 6, 7 and 9 to 12 are checked from then on); then the control planes (the requests of
rows 1 to 5, rows 8, 13 and 14); then, once every live process runs 0.3.0 in `enforce`, the requirement
`run-input-validation-v1`. Until it is recorded, an old control plane admits what a new one refuses, and replicas may
run different modes (G043-010). Runs in flight are checked when they create a child or continue (OD-2026-11-14-9).
Rollback: remove the requirement, then downgrade the binaries; a 0.2.x binary runs on the 0.3.0 schema (the new CHECKs
are supersets), so the binaries can roll back without the down migrations. Those refuse while a schedule action holds
`INPUT_INVALID`, while a registry row lists `run-input-validation-v1`, or while the requirement history names it
(D043-131).

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Triggers on versions whose schema describes a manual input stop starting runs (G043-004) | A delivery and an audit row per refused event, the metric, the publish warning, the release notes, `report` mode while callers are fixed |
| A run started before the upgrade fails at a child or a continuation (OD-2026-11-14-9) | Stated in the release notes with the command that lists the versions at risk; the failure code names the cause; accepted by the owner as a deviation from principle IV |
| A schema the platform did not write costs CPU or memory | The subset and its pre-check, the weighed cache whose tenant share holds the heaviest admitted schema, the validator's own work counter, the input depth and nesting limits, four permits, a 5 s backstop (NFR-043-BOUND, D043-159) |
| An `inputSchema` with tuple positions (`prefixItems`), which features 016 and 035 read, is refused in `enforce` (O043-005) | Stated in 016's contract and the release notes; listed by the inventory; `report` mode publishes it; the owner decides whether tuples join the subset |
| A composition root builds a creator without the gate, which then refuses every start of a version with a schema as busy | The seventh architecture rule fails on it (ADR-0092 decision 2); the unwired gate refuses, it never admits unchecked (D043-120) |
| A schema whose compile panics is tried again on every start (G043-053) | A panic is transient (D043-106) and logged without the schema; a compiler bug, not an input, so it is reported and fixed in the validator |
| Published schemas that use a keyword outside the subset (recursion, `patternProperties`, `if`, `not`, ...) become unusable (OD-2026-11-14-14) | `control-plane input-schema-inventory` lists them before the upgrade (FR-043-018); `report` mode while they are republished (G043-014) |
| The platform's validator and the drafts disagree on a case | The subset excludes the known disagreements (fractional `multipleOf`, `\d`-style classes, `$ref` siblings, tuple `items`); the oracle property test against `jsonschema` (T043-2) is a regression check, not a proof (G043-015, G043-016) |
| One tenant's burst occupies the four permits | A free permit goes to the waiting tenant that holds the fewest; a tenant may use all four while no other waits, and a waiting tenant is served first (D043-068); each validation is bounded by the counter, so a permit is held for a bounded time; a caller who cannot get one in 5 s gets a retryable `503`, never a final refusal (D043-050, D043-068) |
| The first upgraded engine records final outcomes before the operator chose `report` | The runbook runs the inventory and sets `report` in every manifest before migrating (D043-040) |
| A pointer or a rule name leaks what the caller sent | Segments only where `properties` or `items` matched; the rule named by the validator's own keyword; no input value ever formatted; `/` for repeats; the sentinel test with sentinel keys |
| A refusal loops | Rows 6 to 12 record a final outcome; the trigger delivery stops the re-audit; the schedule arm sits before `NotAllowed` and `Err(other)` |
| Replicas answer differently | The capability is listed only in `enforce`; the operator's requirement closes the gate of any other process (G043-010 until it is recorded) |
| `report` mode becomes permanent | Logged at every start, counted, and the release notes ask the operator to record the requirement |
| Existing tests and fixtures start runs whose input does not match their schema | T043-20 sweeps them first and lists each change; a fixture is fixed, never the check loosened |
| An old control plane meets an `INPUT_INVALID` action during the upgrade (G043-001) | The listing passes unknown outcomes through by design; the strict outcome filter is in the release notes; T043-11 tests the listing |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | The check runs before the creation transaction and writes nothing; a schedule refusal, its pause, its audit and the backfill's end are one decision under the schedule's fence; a trigger refusal is one transaction (delivery, then audit); a continuation refusal ends the predecessor through the existing fenced path |
| II. Modular domain, ports and adapters | Pass | The domain holds opaque bytes and a digest computed by `infra-ir`; the port and the gate are in `application`, and the gate names no runtime; the permits, the wait, `block_in_place` and the backstop are in the adapter `infra-schema` behind the port, which has tokio as a normal dependency (D043-071). `application` has tokio `rt` alone for design 042's blocking parse in one file, a divergence from ADR-0090 recorded in ADR-0092 (Proposed) and held to that file by a rule; no new edge into `application` or the domain (`scripts/check-architecture.py`), and a rule keeps `jsonschema` out of the validator module (D043-074) |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect is added; a refused run has none |
| IV. Compatibility is versioned and pinned | Pass with conditions | Published versions are not changed. Three deviations, each accepted by the owner: an input that was accepted is refused (OD-2026-11-14-2); runs in progress can fail at the step that creates a child or continues (OD-2026-11-14-9); a document that publishes today can be refused by 0.3.0 publication, because the subset narrows `inputSchema` (OD-2026-11-14-14). All three in Complexity Tracking |
| V. Code, tenant boundary and secrets are isolated | Pass | No tenant code runs; a schema is data validated by the platform's own bounded validator; no reference resolved outside the document; a compiled schema is shared only within its tenant; bounded compile and validation off the async workers; no grant (NFR-043-SEC); values never in errors or telemetry (FR-043-014) |
| VI. Observability and audit are part of the feature | Pass | Metrics with closed labels, span attributes, an audit of every asynchronous refusal and of every reported one (Technical Plan 7) |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | A trigger refusal is a delivery record: a redelivery finds it and repeats nothing (D043-027) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Per-row tests on real PostgreSQL for both refusals, the crash and duplicate cases of the Verification Plan, six-SDK conformance; gaps registered |
| Delivery Constraints | Pass | No new technology; single region; the upgrade order of OD-2026-11-14-3; the registry of ADR-0082 |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A refusal where an input used to be accepted (principle IV) | The owner's decision OD-2026-11-14-2; published versions stay as they are, only the admission of new runs changes | Deferring validation, as first recommended: the owner refused it |
| A document that publishes today can be refused tomorrow (principle IV: contracts versioned, a change MUST NOT break) | OD-2026-11-14-14: the subset narrows what `inputSchema` may contain, so a schema accepted by 0.2.x publication is refused by 0.3.0 publication in `enforce`, and a stored one refuses its starts. Recorded in `workflow.schema.json` and `sdks/SPEC.md` (T043-14); listed by the inventory | Accepting any JSON Schema with the library: rejected after four reviews (OD-2026-11-14-14) |
| Children and continuations of runs in progress are checked (principle IV: a change MUST NOT break runs in progress) | OD-2026-11-14-9, against the recommendation: a child or a successor is a new run, and the owner wants every new run checked | Exempting runs created before the upgrade (the recommendation): the owner refused it. The release notes say an old run can fail there |
| `CheckedRunInput`, a `test-support` feature and seven architecture rules | Thirteen creating paths in three crates; the next one must not skip the check, the test constructor must not reach a build, and a composition root must not leave a creator unwired | A code review rule: how G1955 came to exist |
| `tokio` `rt` in `sagat-application`, for one file (ADR-0092) | Design 042, merged first, parses a blob source's array in a blocking task there | Moving that parse behind a port of an adapter (G043-052): the follow-up of design 042, not done in the middle of a release |
| A narrow port and a gate in `application` | The policy (mode, metrics, the type) must not live in the adapter; the adapter sees only the domain identifiers it needs (`TenantId` in `SchemaKey`) and never builds `CheckedRunInput` | One trait that does everything: the adapter would own the mode and build the type |
| The schema bytes and digest in `WorkflowDefinition` | Continue-as-new holds the definition, not the document; no statement may be added | Reading the document on a cache miss: an extra statement on the continuation path |
| A second compiled-validator cache keyed by tenant and digest | Compile once per schema; never under a lock; never shared across tenants | Compiling per start, as the blob check does today; keying by version, which ad hoc floods |
| A validator of the platform for a documented subset, with a work counter (OD-2026-11-14-14) | The bound on the work holds by construction: the counter counts what the validator does and stops at a budget. Four review rounds found work that `jsonschema` does and a static walk did not charge (`unevaluated*`, percent-encoded pointers, value-sized keywords) | `jsonschema` with a static walk as the bound: rejected after four reviews; validation in an isolated process with a timeout: rejected for latency and operations |
| A blocking section with four permits shared work-conservingly among tenants, a 5 s wait and a 5 s backstop | Validation is CPU work on schemas and inputs the platform did not write; the counter bounds each run, and the fair queue keeps one tenant from holding all four while another waits | A plain semaphore with a timed acquire, relying on the existing rate limits and pass fairness: one tenant's burst would hold the four permits while another tenant's starts time out, and the rate limits are per namespace, not per permit; the miss memory and the two clocks of the third draft (removed, D043-050); refusing a tenant at its cap at once (D043-053, which broke a single-tenant deployment), and a fixed cap of two per tenant (D043-061, which left permits idle); replaced by D043-068 |
| A schedule action outcome and a migration | A refused action must be visible and must not loop | Reusing `ADMISSION_RETRY` (`NotAllowed`): retries forever |
| A capability and a strict variable | Replicas must not answer differently once the operator decides | A per-process flag alone: nothing stops one replica in `report` beside others in `enforce` |
| Three failure codes (`CHILD_INPUT_INVALID`, `CONTINUATION_INPUT_INVALID`, `RUN_INPUT_SCHEMA_UNUSABLE`) | The parent and the chain must say why, without the value | `EXPRESSION_ERROR`: says the expression failed, which it did not |

## Alternatives Considered

- **`jsonschema` with a static walk as the bound on its work.** Four review rounds found cases the walk did not
  charge or resolved differently (`unevaluated*` re-running branches, percent-encoded pointers, value-sized keywords,
  recursion through `dependencies`). Rejected for the subset with a validator of the platform (OD-2026-11-14-14).
- **Validation in an isolated process with a timeout.** It bounds the damage without bounding the work, adds a round
  trip to every start and a process to operate. Rejected for latency and operations.
- **The subset's pre-check, then `jsonschema` on the checked schema.** The pre-check would keep the keywords whose work
  escaped the static walk out, but `jsonschema`'s own work per keyword is still not counted, the dialect differences
  (`format`, `content*`) would remain, and the bound would rest on reading the library rather than on a counter.
  Rejected for the platform's validator, whose counter counts what it does (OD-2026-11-14-14); `jsonschema` stays its
  oracle.
- **A wider subset now** (`patternProperties`, `uniqueItems`, `not`). Each could be bounded by the counter, but each adds
  semantics to cover; left out of 0.3.0 (D043-046), addable later without breaking a schema.
- **Validate in `PgRunStore::create`.** One place for most rows, but in the infra layer, inside the creation
  transaction (a compile under the run's locks), and continue-as-new does not pass there. Rejected; the store only
  compares the version id of an input already checked (FR-043-017).
- **A check written in each creator, no shared type.** What exists for the blob start. It drifts. Rejected.
- **Validation in the SDKs only.** Six implementations of a dialect, and the engine's creators have no SDK in their
  path. Rejected.
- **Cache keyed by version id.** Simple, but an ad hoc start makes a version per call (`ad_hoc.rs:459`) and would fill
  the cache, and identical schemas of successive versions would compile again. Rejected for `(tenant, digest)`.
- **`report` by default in 0.3.0.** Contradicts OD-2026-11-14-2. Rejected.
- **A per-namespace mode.** Six SDK surfaces, the Studio and an audit for a transition aid. Not chosen (OD-2026-11-14-8).
- **Defer, as recommended before OD-2026-11-14-2.** The owner chose otherwise.

## Non-Functional Requirements

- **NFR-043-PERF**, **NFR-043-BOUND**, **NFR-043-SEC** and **NFR-043-COMPAT** as stated under Requirements; SC-043-003
  and SC-043-004 are the tests of the first two.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Each task is one pull request unless it says otherwise, with its tests seen failing first. T043-1 to T043-4 and T043-20
come first; T043-21 to T043-24 land with the tasks they test; T043-5 to T043-13 can run in parallel after them; T043-14 to T043-19 close the feature.

- [ ] T043-1 Carry `InputSchema` (an `Arc<[u8]>` of canonical bytes and its SHA-256) in the parsed `WorkflowDefinition`,
  counted by its footprint.
  Files: `crates/domain/src/workflow.rs`; `crates/infra-ir/src/lib.rs` (on `parse`, `parse_stored` and the private
  twins); tests in `crates/infra-ir/tests/cache.rs`.
  Refs: FR-043-001, NFR-043-PERF
  Owner-gate: OD-2026-11-14-2
- [ ] T043-2 Build `InputSchemas`, the platform's validator of the subset: the pre-check (vocabulary, local pointers,
  `$ref` cycles, sizes, pattern syntax) with its typed `Unusable { keyword, at }`, the compiled form, the work counter
  and its budget, the application nesting limit, the refusal with its closed `Rule` and its pointer, and the
  `(tenant, digest)` cache.
  Files: `crates/infra-schema/src/run_input/` (new: `vocabulary.rs`, `compile.rs`, `validate.rs`, `counter.rs`),
  `crates/infra-schema/src/lib.rs`, `crates/infra-schema/src/classification/cache.rs` (generic over key and value; the
  classification walk and its behaviour unchanged), `crates/infra-schema/Cargo.toml` (`jsonschema` stays, for the
  scripts and as a dev oracle); tests `crates/infra-schema/tests/run_input.rs`: the oracle property test (generated
  schemas of the subset and inputs; the same verdict as `jsonschema` 0.58.6 with formats off and the refusing
  retriever, for `$schema` absent, draft-07 and 2020-12); a vocabulary test that refuses every excluded keyword of
  Technical Plan 4 with its name and pointer, and every subset rule (tuple `items`, boolean exclusive bounds, a
  fractional `multipleOf`, `$ref` with siblings, a nested `$id` or `$schema`, an unknown `$schema`, a property named `id`
  or `$ref` accepted); the pattern allowlist (`\d`, `\D`, `\w`, `\W`, `\s`, `\S`, `\b`, `\B`, inside brackets too,
  `(?i)`, `\A`, `\z`, `(?P<`, `(?<`, `[[:alpha:]]`, `\p{L}` refused; `.` against U+2028); the reference targets
  (`#/enum/0`, `#/default`, `#/examples/0`, an `x-sagat-classification` value, `#/properties` refused; `#/$defs/a`
  accepted); the pointer decoding (`%25`, `~0`, `~1`, `~01` accepted with their meaning; a bad `%`, `~2`, non-UTF-8 and
  `#/allOf/01` refused); a cycle found by the marked DFS through `allOf` and through `properties`; a `$ref` DAG with ×2
  fan-out compiled to a document-sized arena; an application path of 129 refused; the adversarial cases, one per keyword
  (`required`, `properties`, `additionalProperties`, `enum`, `const`, `type`, `allOf`, `anyOf`, `oneOf`, `items`, the
  length checks, `pattern`), each asserting primitive operations ≤ k × units and a stop at `workLimit`, among them a
  `$ref` DAG with ×2 fan-out ending at a `required` of 6,000 names and a pattern that defeats the lazy DFA (such as
  `[ab]*a[ab]{12}`) against a 256 KiB string, timed against its `(bytes + 1) × states` charge to measure the divisor and
  k recorded in G043-017; `oneOf: [cheap match, costly match]` refused with `workLimit`, not accepted; a class set
  operation, a nested bracket, `[]a]` and `[^]` refused; a ×2 `allOf` tree of depth 10 over an `enum` of 1,000 members
  refused at publication with `INPUT_SCHEMA_TOO_COSTLY`, an `anyOf` of one cheap and one costly branch accepted, and
  a bound that saturates instead of overflowing; the oracle property test with its skip ratio and its number generators; a
  determinism test (two validators built from the same bytes report the same `field` and `rule`); a proptest that no
  generated value or key appears in a refusal.
  Refs: FR-043-002, FR-043-011, FR-043-014, NFR-043-BOUND, NFR-043-SEC, SC-043-004
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-11, OD-2026-11-14-14
- [ ] T043-3 Build, in `application`, the `RunInputGate` (its `unwired()` default, `check`, `check_schema`, `recheck`),
  `CheckedRunInput` (version id, fields and constructor private to `run_input.rs`, a `test-support` constructor and a
  `test-support` validator double), the mode, the transient refusals mapped to `Unavailable`, the input depth limit,
  the field policy and the metrics; and, in the adapter (D043-071), the work-conserving fair queue of D043-068 (fewest
  held first, a third or fourth permit for a tenant only while no other waits, a 5 s wait, guards that leave the queue
  or return the permit when the future is dropped), `off_worker` (`block_in_place`, or in place on a current-thread
  runtime) and the backstop.
  Files: `crates/application/src/run_input.rs` (new), `crates/application/src/lib.rs`, `crates/application/Cargo.toml`
  (`[features] test-support`; tokio stays a dev-dependency), `crates/application/src/blobs.rs` (remove `RunInputChecker`),
  `crates/infra-schema/src/run_input/permits.rs` (new), `crates/infra-schema/Cargo.toml` (tokio `sync`, `time`,
  `rt-multi-thread`); unit tests in both modules.
  Refs: FR-043-001, FR-043-013, FR-043-014, NFR-043-PERF, NFR-043-BOUND
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-8, OD-2026-11-14-14
- [ ] T043-4 Make `NewRun.input`, `ContinuationSelection` and `EvaluatedChainHandoffCommand.input` a `CheckedRunInput`;
  refuse a version mismatch in `create` and `continue_evaluated`, with a store test of each (FR-043-017); add the four
  architecture rules with negative fixtures, a fifth that rejects a `jsonschema` dependency, `use` or path under
  `crates/infra-schema/src/run_input/` (D043-074), a sixth that holds `tokio` in `sagat-application` to `rt` alone and to
  `collection_source.rs` (ADR-0092 decision 1), and a seventh that fails on a creator a composition root builds without
  the gate (ADR-0092 decision 2), each with its negative fixture; wire the gate, held as `Arc<RunInputGate>` over an
  `Arc<dyn SchemaValidator>`, in both composition roots.
  Files: `crates/application/src/runs.rs` (`NewRun`), `crates/application/src/chain_progress.rs`,
  `crates/infra-postgres/src/execution.rs`, `crates/infra-postgres/src/chain_progress.rs`, `scripts/check-architecture.py`,
  `scripts/tests/` fixtures, `apps/control-plane/src/lib.rs` (`:746`), `apps/execution-engine/src/lib.rs` (`:531`, the
  batch executor's run service, among others), every `Cargo.toml` under `tests/` and `crates/*/` that needs
  `test-support` (dev-dependencies only); store tests in `tests/integration/durable-execution/` and
  `tests/integration/workflow-primitives/chain_progress.rs`.
  Refs: FR-043-016, FR-043-017
  Owner-gate: OD-2026-11-14-2
- [ ] T043-5 Check rows 1 to 5 in `start_with`: the error variant, the replay before refusal, the `409` after a
  promotion, the refusal with an open holder, the field policy of repeats under graded disclosure (`/` and `other`
  when the caller's view withholds anything), and the batch and reset mappings.
  Files: `crates/application/src/runs.rs`, `crates/application/src/runs/reset.rs`, `crates/application/src/error.rs`,
  `crates/api-http/src/error.rs`, `crates/application/src/batch_executor.rs`, `crates/application/src/reset_completion.rs`;
  tests in `tests/integration/durable-execution/`, `tests/contract/rest-durable-execution/`,
  `tests/integration/schedules-bulk-operations/` (with-start, rerun, reset, batch).
  Refs: FR-043-001, FR-043-003, FR-043-004, FR-043-005, FR-043-010, FR-043-011
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-10, OD-2026-11-14-11
- [ ] T043-6 Check row 13 (ad hoc and private starts) with its own replay, outside the shared cache, under the same
  permits as every other row.
  Files: `crates/application/src/runs/ad_hoc.rs`; tests in `tests/integration/database-connections/`.
  Refs: FR-043-001, FR-043-004
  Owner-gate: OD-2026-11-14-2
- [ ] T043-7 Check rows 9 and 10: the delivery without a run, the audit written once, the metric, `Started::No`.
  Files: `crates/scheduler/src/events.rs`, `crates/application/src/dispatch.rs` (a `DeliveryRef` without a run and the
  store call), `crates/infra-postgres/src/execution.rs` (that call), `crates/domain/src/audit.rs` (`RunInputRefused`);
  tests in `tests/integration/events-webhooks-messaging/`
  (`a_cron_trigger_whose_input_is_refused_starts_nothing_and_audits_the_tick` for row 10),
  `tests/fault/events-webhooks-messaging/` (a retry after another consumer fails, a crash between the two writes,
  `two_engines_on_one_refused_cron_tick_audit_it_once_with_the_tick_key` and
  `a_cron_refusal_whose_writer_stops_before_the_trigger_moves_is_audited_once_by_the_next_round` for G043-002,
  D043-156).
  Refs: FR-043-001, FR-043-007
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-11
- [ ] T043-8 Check row 11 with `CHILD_INPUT_INVALID` and `RUN_INPUT_SCHEMA_UNUSABLE`, also for a parent created before the
  check was on; look up `child:{parent}` / `child-start:{node}` before failing the step, with a test where the child
  already exists.
  Files: `crates/scheduler/src/passes.rs`, `crates/domain/src/execution/event.rs`; tests in
  `tests/integration/workflow-primitives/`.
  Refs: FR-043-001, FR-043-008
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-9, OD-2026-11-14-11
- [ ] T043-9 Check row 12 with `CONTINUATION_INPUT_INVALID` and `RUN_INPUT_SCHEMA_UNUSABLE`, also for a chain started
  before the check was on.
  Files: `crates/scheduler/src/passes.rs`, `crates/domain/src/execution/event.rs`; tests in
  `tests/integration/workflow-primitives/chain_progress.rs`, `tests/fault/workflow-primitives/` (a restart between the
  evaluation and the failure).
  Refs: FR-043-001, FR-043-009
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-9, OD-2026-11-14-11
- [ ] T043-10 Check row 14: `target_check` returns the definition; `SCHEDULE_INPUT_INVALID` under `/action/input`;
  `NOMINAL_TIME_FIELD_INVALID`; `RUN_INPUT_SCHEMA_UNUSABLE`.
  Files: `crates/application/src/schedules.rs`, `crates/infra-postgres/src/schedules.rs` (the `target_check` query),
  `crates/api-http/src/error.rs` (remediation of `NOMINAL_TIME_FIELD_INVALID`); tests in
  `crates/application/tests/schedule_document.rs`, `tests/integration/schedules-bulk-operations/`.
  Refs: FR-043-006
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-11
- [ ] T043-11 Check rows 6 to 8: the migration (`NOT VALID`, then `VALIDATE`), the check before `decide`, one decision
  with the action, the auto-pause, its audit and a backfill `CANCELLED`, trigger-now `422`, `BUFFERED` to
  `INPUT_INVALID`, both refusals; test the listing of an unknown outcome (G043-001).
  Files: `crates/infra-postgres/migrations/<next timestamp>_run_input_validation.{up,down}.sql` (the action outcome and
  the capability lists; take the timestamp after grepping the other branches), `crates/domain/src/schedules.rs`,
  `crates/application/src/schedule_firing.rs`, `frontend/apps/studio/src/features/schedules/model.ts`; tests in
  `tests/integration/schedules-bulk-operations/`, `tests/fault/schedules-bulk-operations/` (a crash before the decision
  commits), the run as the runtime role.
  Refs: FR-043-006, NFR-043-COMPAT
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-11
- [ ] T043-12 Build the mode: strict `SAGAT_RUN_INPUT_VALIDATION`, the capability `run-input-validation-v1` listed only
  in `enforce`, the manifest test, and the registry tests (recording refused while a live process runs in `report`; a
  process started in `report` after recording stays up with its gate closed).
  Files: `crates/application/src/instances.rs`, `apps/execution-engine/src/capabilities.rs`,
  `apps/execution-engine/src/settings.rs`, the control plane's settings and capabilities, `apps/execution-engine/tests/config.rs`
  and `apps/control-plane/tests/config.rs` (`no_shipped_manifest_sets_run_input_validation_to_report`),
  `apps/execution-engine/tests/registry.rs`, `specs/018-schedules-bulk-operations/data-model.md` (the capability).
  Refs: FR-043-013, NFR-043-COMPAT
  Owner-gate: OD-2026-11-14-8
- [ ] T043-13 Run the pre-check at publication and in `sagat validate`, the mode passed as a parameter (the process's
  mode in the control plane; `--input-validation`, `enforce` by default, in the CLI): a typed issue naming the keyword
  and its pointer in `enforce`, a warning in `report`; warn on a schema with a non-manual trigger.
  Files: `crates/infra-ir/src/lib.rs`, `crates/infra-ir/src/carrier_schema_check.rs`,
  `crates/application/src/definitions.rs` (`DefinitionWarning`, the issue codes); tests in `crates/infra-ir/tests/`,
  `tests/integration/gitops-versioning/` (both modes), `apps/cli/tests/`.
  Refs: FR-043-012
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-14
- [ ] T043-14 Update the contracts: OpenAPI (outcome, reasons, the `422` of the six start operations under
  OD-2026-11-15-3 with the re-pinned baselines, the `details.rule` enum, `RUN_INPUT_SCHEMA_UNUSABLE`, the `Run`'s
  `failureCode`), the note on `inputSchema` carriers in 016's contract, the new `errors.md` with the publication issue codes `INPUT_SCHEMA_KEYWORD_UNSUPPORTED`,
  `INPUT_SCHEMA_REFERENCE_INVALID`, `INPUT_SCHEMA_PATTERN_UNSUPPORTED`, `INPUT_SCHEMA_TOO_LARGE`, `INPUT_SCHEMA_TOO_DEEP`,
  `INPUT_SCHEMA_TOO_COSTLY` and `INPUT_SCHEMA_NAME_TOO_LONG` (and the same closed list in the OpenAPI description of the
  publication's `422`, which also says that a schema over 64 KiB or 32 deep is answered `CLASSIFICATION_TOO_COMPLEX`,
  D043-112), the 018
  `errors.md` row, the remediation lines; record the subset in the IR schema (`inputSchema` at
  `workflow.schema.json:47`, today `{"type": "object"}`, gets a description and a reference to the subset's keyword
  list) and in `sdks/SPEC.md` (the `.inputSchema(schema)` builder, `:501`); confirm AsyncAPI is unchanged.
  Files: `specs/001-platform-foundation/contracts/openapi.yaml`, `specs/001-platform-foundation/contracts/workflow.schema.json`,
  `sdks/SPEC.md`, `frontend/apps/studio/src/api/schema.d.ts`
  (regenerated), `specs/043-run-input-validation/contracts/errors.md` (new),
  `specs/018-schedules-bulk-operations/contracts/errors.md`, `docs/quickstarts/errors.md`,
  `specs/016-payload-classification/contracts/README.md`, the pinned baselines of `contract_payload_classification` and
  `contract_iceberg_writes`; tests `contract_rest_durable_execution`, `contract_schedules_bulk_operations`,
  `contract_asyncapi_events_webhooks_messaging`, `contract_payload_classification`.
  Refs: FR-043-003, FR-043-006, FR-043-015
  Owner-gate: OD-2026-11-14-2, OD-2026-11-15-3
- [ ] T043-15 Give the six SDK error types `field` and `rule`, and the six SDK `Run` types the optional `failureCode`;
  add the conformance scenario `run-input-validation`, failure codes included (the chain's run view shows
  `CONTINUATION_INPUT_INVALID`); regenerate the goldens whose run views gain `failureCode`, each diff read first.
  Files: `sdks/SPEC.md`, the error type and the `Run` type of each SDK under `sdks/`,
  `tests/contract/sdk-conformance/scenarios/run-input-validation.json` (new), the six conformance runners, the goldens
  under `tests/contract/sdk-conformance/expected/`, `sdks/RELEASE_NOTES.md`.
  Refs: FR-043-015, SC-043-005
  Owner-gate: OD-2026-11-14-2
- [ ] T043-16 Surface the refusal in the CLI and the Studio start form (`reason`, `field`, `rule`).
  Files: `apps/cli/src/workflow.rs`, `apps/cli/tests/`, `frontend/apps/studio/src/` (the start dialog and its test).
  Refs: FR-043-015
  Owner-gate: OD-2026-11-14-2
- [ ] T043-17 Run the sentinel test across rows 1 to 14 (row 10 carries no caller value: its input is the platform's
  tick) and check the metric labels.
  Files: tests in `tests/integration/payload-classification/`, `tests/integration/observability-operations/`.
  Refs: FR-043-014, SC-043-002
  Owner-gate: OD-2026-11-14-2
- [ ] T043-18 Budget a start, a triggered start, a child start and a continuation with and without an `inputSchema`,
  in `enforce` mode.
  Files: `tests/integration/claim-transition-efficiency/work_budget.rs`.
  Refs: NFR-043-PERF, SC-043-003
  Owner-gate: OD-2026-11-14-2
- [ ] T043-19 Write the runbook and the release notes (the warning, the subset, the inventory command and the choice of `report`
  before migrating, old runs failing at a child or a continuation, recording the requirement once every process is in
  `enforce`, and removing it before going back), the traceability rows, and close G1955 and G2081
  of feature 018 by reference.
  Files: `docs/runbooks/run-input-validation.md` (new), `docs/releases/unreleased.md` (or the 0.3.0 notes where they
  live), `docs/traceability.md`, `specs/018-schedules-bulk-operations/research.md`,
  `specs/043-run-input-validation/research.md`, `docs/reference` regenerated after reading the diff.
  Refs: FR-043-013, FR-043-006, FR-043-001, FR-043-018
  Owner-gate: OD-2026-11-14-8, OD-2026-11-14-9, OD-2026-11-14-14
- [ ] T043-20 Sweep the tests, fixtures and examples whose inputs do not match their schemas, before T043-5 lands.
  Files: whatever the sweep finds under `tests/`, `crates/*/tests/`, `examples/` and the SDK examples; each change
  listed in the pull request.
  Refs: SC-043-001
  Owner-gate: OD-2026-11-14-2
- [ ] T043-21 Test the budget and the backstop on every row: an input past the budget is final the first time with
  `workLimit` (an `INPUT_INVALID` action, a failed child step, a trigger delivery without a run, a batch item `FAILED`);
  with the `test-support` validator double held by a barrier, a tenant alone on the process holding all four permits at
  four concurrent starts (work-conserving), a second tenant that arrives then served before the first tenant's next
  waiter, no permit taken from a holder, a future dropped
  while it waits that leaves the queue and one dropped after its grant that returns its permit (the cancellation
  tests), permits not obtained
  within 5 s and a backstop stop are transient (`503`, a batch item deferred, a pending reset left pending, a schedule time owed, an event retried, a child
  or a continuation left for the next pass), never cached, never final; no more than four validations at once.
  Files: tests in `tests/integration/durable-execution/`, `tests/integration/schedules-bulk-operations/`,
  `tests/integration/events-webhooks-messaging/`, `tests/integration/workflow-primitives/`, and unit tests in
  `crates/application/src/run_input.rs`.
  Refs: NFR-043-BOUND, FR-043-001
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-11
- [ ] T043-22 Test `report` mode on one creator per process: `POST /runs` in the control plane and an event trigger in
  the engine; the run is created, the refusal counted, and its audit written in the creation transaction, once across a
  retry and a replay.
  Files: tests in `tests/integration/durable-execution/`, `tests/integration/events-webhooks-messaging/`.
  Refs: FR-043-013
  Owner-gate: OD-2026-11-14-8
- [ ] T043-23 Check that every `Role` holding `run:start` also holds `definition:read`, so `details.field` names only
  what the caller may read (keeps G043-003 closed; a regression check).
  Files: `crates/domain/src/identity.rs` (unit test).
  Refs: FR-043-003, FR-043-014
  Owner-gate: OD-2026-11-14-2
- [ ] T043-24 Build `control-plane input-schema-inventory` and its test: one stored version inside the subset and one
  with `patternProperties`; the second is listed as refused with its ids, the keyword and its pointer, the first (with an
  event trigger) as informational, no schema text; exit 1, exit 0 with only the first, exit 2 against an unreachable
  database.
  Files: `apps/control-plane/src/cli.rs`, `apps/control-plane/src/inventory.rs` (new),
  `crates/infra-postgres/src/definitions.rs` (a paged read of stored versions for the owner role),
  `apps/control-plane/tests/inventory.rs` (new), `docs/runbooks/run-input-validation.md`.
  Refs: FR-043-018
  Owner-gate: OD-2026-11-14-14
#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`python3 scripts/check-architecture.py`, `python3 scripts/check-agent-config.py` and
`python3 scripts/check-design-docs.py`, the unit tests of the crates it changes, and these targets as its rows need:

- `integration_durable_execution` (rows 1, 2; FR-043-004, FR-043-011, FR-043-013), also with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
- `contract_rest_durable_execution` (the problem shape of FR-043-003);
- `integration_schedules_bulk_operations`, `fault_schedules_bulk_operations`, `contract_schedules_bulk_operations`
  (rows 3 to 8 and 14; the migration as the runtime role);
- `integration_events_webhooks_messaging`, `fault_events_webhooks_messaging`, `contract_asyncapi_events_webhooks_messaging`
  (rows 9, 10; a redelivery, a crash between the delivery and the audit, AsyncAPI unchanged), the integration suite also
  with `SAGAT_TEST_AS_RUNTIME_ROLE=1` (the delivery without a run);
- `integration_workflow_primitives`, `fault_workflow_primitives` (rows 11, 12, pre-upgrade parents included), the
  integration suite also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
- `integration_database_connections` (row 13);
- `integration_gitops_versioning` (FR-043-012);
- `integration_payload_classification` (the sentinel), `integration_observability_operations` (labels);
- `integration_claim_transition_efficiency` (SC-043-003), which passes unmodified except for the new lines of T043-18;
- the configuration and registry tests of `apps/control-plane` and `apps/execution-engine` (FR-043-013);
- `SAGAT_SDK_SCENARIO=run-input-validation cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored`;
- the per-SDK checks of `sdks/AGENTS.md`, `(cd frontend/apps/studio && npm run api && npm run check:api && npm test)`,
  and the CLI tests;
- `SAGAT_DOCS_BLESS=1 cargo test -p sagat-tests --test contract_developer_experience` only after reading the diff, then
  again without the switch.

Failure cases (constitution VIII): a crash between the check and the creation (the check wrote nothing); a redelivered
event (the delivery row stops a second audit); a crash between the trigger's delivery and its audit (one transaction,
so neither or both); a restart with a cold cache (one compile, no statement); a schedule refusal under a lost shard
lease (the decision is refused by its fence); two creators compiling the same schema at once (single flight, one
compile by counter); a budget miss (T043-21: final the first time); no permits within 5 s, a dropped waiting future, or
the backstop (T043-21: transient, never remembered). A test that fails is a
defect to explain, not to retry.

## Appendix A: details the bound rests on

Informative. These are how the adapter holds the bounds of Technical Plans 3 and 4; they change with the code, and the
tests named in T043-2 and T043-3 hold them, so a change here does not change the architecture.

- **The pre-check's search** (D043-069). Depth-first, with white, grey and black marks over the application edges: a
  grey node met again is a cycle. When a node turns black the search stores its longest application path and its lower
  bound, so the pass stays linear in the arena; every sum is a `saturating_add`, so it cannot overflow. A `$ref` is the
  index of its target node, never an inlined copy, so a DAG of references stays the size of the document.
- **Off the async workers** (D043-071). The adapter's `off_worker` runs the validator through `block_in_place` on a
  multi-thread runtime, and in place on a current-thread runtime (where `block_in_place` would panic, and where there is
  no other worker to protect) or outside a runtime. The cache's single-flight `Condvar` is waited on only there. The work
  is bounded by the counter, so nothing has to be cancelled and no thread outlives its caller. A tenant's bookkeeping
  entry in the permit queue leaves the map when it holds nothing and waits for nothing, so nothing in use is evicted.
- **The regex engines** (D043-064, D043-159). Each pattern is built with every runtime engine bounded: the forward and
  the reverse lazy DFA with a cache capacity of 16 KiB each, the bounded backtracker off (its visited set is 256 KiB by
  default), the one-pass DFA off, and the PikeVM cache measured once with `Cache::memory_usage()` after a warm-up match.
  A lazy DFA whose minimum cache does not fit in 16 KiB is not built, and the pattern runs on the PikeVM, whose work the
  charge of `(bytes + 1) × states` already prices. The regex keeps a pool of caches, one for its owner thread and one per
  other thread that matches at once, at most four here under the permits, so an entry weighs its programs plus
  (1 + 4) × that per-pattern cache size; a compile-time assertion of `pattern.rs` holds the four matchers to at least
  `RUN_INPUT_PERMITS` (D043-107).
- **Determinism** (D043-059). The compiled form keeps document order in vectors and sorted keys in its lookup tables; no
  iteration depends on `RandomState`. Instance members are visited in `serde_json::Map` order (a `BTreeMap`: the
  workspace does not enable `preserve_order`).
