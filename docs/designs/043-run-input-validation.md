# Run input validation

*   **Author:** Claude Code, for the owner
*   **Status:** Approved
*   **Date:** 2026-11-16
*   **Reviewed-commit:** 4b6eb1a649b281660c4622e20d0b348322a7d8a4
*   **EGM-hash:** 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
*   **Claude-Code:** 2.1.325
*   **Reports:** docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-egm-review.md sha256:374a7801168180a0c72246d7cc8dfbb95e1c02a9a885bde87c27e244fe728e29; docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-architecture.md sha256:e571280931fa6eff1fe39d045e99b0fe8e6cd2703bb9dca7bf72088f1ce4e28f; docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-security.md sha256:e4773b83d089c81467664f3f84adaeb9da653d2677718f879f690f04265776cb; docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-correctness.md sha256:e307f3b1a56630f3ffa5adc005218fb8680efd6b590367e691e67c68f2ed8d7d
*   **Open findings:** Blocker 0, Major 0
*   **Accepted:** 7; OD-2026-11-16-100, OD-2026-11-16-101, OD-2026-11-16-102, OD-2026-11-16-103, OD-2026-11-16-104, OD-2026-11-16-105, OD-2026-11-16-106

--------------------------------------------------------------------------------

## Overview

From release 0.3.0 the input of every run is checked against the `inputSchema` of the workflow version the run will
pin, before the run is created. One gate in `crates/application` does the check for each of the thirteen paths that
create a run, listed in Technical Plan 1, from `POST /runs` (and so the six SDKs, the CLI and the Studio) to the ad hoc
database execution. An input that does not match is refused where it used to be accepted. A caller who waits gets
`422 VALIDATION_FAILED` with a closed reason, a JSON pointer and the rule that failed, never the value. A creator that
nobody waits for records a permanent, value-free outcome: a trigger records a delivery without a run and audits it
once, a schedule records an `INPUT_INVALID` action, a child fails its parent step, a continuation fails its run.

The `inputSchema` is a documented subset of JSON Schema
(`specs/043-run-input-validation/contracts/input-schema-subset.md`), validated by the platform's validator, whose work
counter stops at a budget (OD-2026-11-14-14); a schema outside the subset is refused at publication and, if already
stored, refuses every start of its version. `control-plane input-schema-inventory` lists the stored versions it affects.
The check has two modes (FR-043-013): `enforce`, the default, refuses as described; `report` runs every check, counts
and audits each final refusal, and creates the run anyway, so an operator can fix callers before locking the upgrade.

The rule is [ADR-0090](../adr/ADR-0090-every-run-input-is-checked-against-its-pinned-version-schema-before-the-run-is-created.md),
refined while it was built by
[ADR-0092](../adr/ADR-0092-the-application-names-a-runtime-only-for-the-blob-source-parse-and-the-run-input-rules-refined-in-the-build.md)
(accepted, OD-2026-11-16-4). The owner's decisions are listed under Owner decisions needed. The owner asked for the design to be built
before it is approved (OD-2026-11-14-15), so the tasks describe what was built and each is ticked with its evidence at
approval. The registers, the history of the drafts and the build, and the validator's internals are in
`specs/043-run-input-validation/research.md` (decisions D043-001 to D043-188, gaps G043-001 to G043-058, owner questions O043-001 to O043-008). The
principles cited by numeral are those of [the constitution](../constitution.md).

### Terms

- **Holder.** The open run that holds a business identifier (feature 018). A start with the conflict policy
  `USE_EXISTING`, and every Signal-with-Start and Update-with-Start, is answered by the holder when there is one: the
  holder gets the signal or the Update, and no run is created.
- **Repeat.** A start whose input is the stored input of an earlier run: a rerun (`RERUN_AS_NEW`), a batch `RERUN` item,
  a reset and a pending reset (rows 4 and 5). Its creator label is `repeat`.
- **A schedule's time** (feature 018, `specs/018-schedules-bulk-operations/data-model.md`). `decide` turns a due nominal
  time into one action: `STARTED`, `BUFFERED` (an overlap policy held it, to be released later) or another outcome. A
  time is **owed** while it waits to be decided again (a transient failure), and **acted on** once its action is
  recorded; an exhausted schedule has no actions left to use. Each decision commits under the schedule's fence.
- **The requirement and the closed gate** (ADR-0082). The operator records that every control plane and engine must list
  a capability (`platform_requirement_activate`, with the owner role, `docs/runbooks/instance-registry.md`); the
  registry refuses that while a live process lacks it. A process that lacks a recorded capability stays up and serves
  nothing but its liveness probe (`503`, `BINARY_CAPABILITY_MISSING`): ADR-0082 calls this its closed gate, which is not
  the `RunInputGate` of this design.
- **The validator.** `RunInputValidator` (`crates/infra-schema/src/run_input/`), the platform's own validator of the
  subset: the pre-check, the compiled schemas (`InputSchemas`, its cache) and the permits. This design calls it "the
  validator" and names `jsonschema`, the library, by its name.
- **Permit; the transient refusals.** The validator runs at most four checks at once per process, each on a permit. A
  caller waits at most 5 s for one; with none it is refused `Busy` ("no permit"). `Busy`, the backstop (a validation
  stopped at 5 s) and `Interrupted` (a panic of the validator) are the transient refusals: nothing was judged, so a
  caller gets a retryable `503` and the engine tries again later. Every other refusal is final.
- **Blob descriptor** (row 2). A start with an uploaded input (`inputBlobRef`) is checked against the object the
  platform builds for the upload, `{"$blob": {"id", "size", "sha256", "contentType"}}`; the uploaded bytes stay opaque
  and are never read by the check.
- **Reset, pending reset, `CANCEL_SOURCE`, completion** (row 5). A reset creates a new run from a prefix of an ended
  run's journal, with the source's stored input. A reset of a run still open with the source policy `CANCEL_SOURCE`
  is a pending reset: its request cancels the source, and its completion, a pass of the engine once the source has
  ended, creates the new run.
- **Ad hoc execution, connection probe, private rerun** (row 13). An ad hoc execution runs a database statement as a
  one-off run of a document the caller sends with the request (`POST /database-executions`); a connection probe is the
  platform's own such run that tests a connection; a private rerun repeats an ad hoc run. All three are created by
  `start_private` (`crates/application/src/runs/ad_hoc.rs`), which makes a version per call.

## The Problem

The IR declares `inputSchema` (`specs/001-platform-foundation/contracts/workflow.schema.json`) and the platform
enforces it on one path only. Only a start with an uploaded input checks it (`crates/application/src/runs.rs`); a
plain JSON input, a triggered run, a schedule's run, a child and a continuation are created with whatever they carry,
as long as it is a JSON object of at most 256 KiB (`runs.rs`).

Feature 018 could not close its own input check because of this (G1955, G2081 in
`specs/018-schedules-bulk-operations/research.md`): a schedule that validated would be stricter than a person starting
the same workflow, and `NOMINAL_TIME_FIELD_INVALID` (`specs/018-schedules-bulk-operations/contracts/errors.md`) is
never answered.

The constraints beyond a check in `start_with`:

1. **Thirteen creating paths, and many with nobody waiting.** Triggers, schedule actions, backfills, children,
   continuations, batch items and pending resets run in the engine; a refusal needs an outcome someone can see, and
   must not be retried when it cannot pass. A schedule action refused today with `NotAllowed` is deferred and retried
   forever (`crates/application/src/schedule_firing.rs`), and an error variant that no arm names falls to
   `Err(other)` and is fired again every tick.
2. **The schema is not in the parsed definition.** `WorkflowDefinition` (`crates/domain/src/workflow.rs`) has no
   input schema; continue-as-new holds the definition and not the document.
3. **The hot path must not pay for it.** The design 038 budget (`tests/integration/claim-transition-efficiency/work_budget.rs`)
   holds what a lease and a completion cost; a start must not add a statement either.
4. **Published schemas were never checked to compile or to stay within bounds.** At publish only a schema with an
   `x-sagat-classification` annotation is compiled (`crates/infra-ir/src/carrier_schema_check.rs`), and the
   compile used for scripts (`crates/infra-schema/src/lib.rs`) has a 1 MiB pattern limit and no depth, pattern-count
   or expansion bound. A schema is author data that the control plane and the engine will now evaluate on every start.
   Four review rounds showed that the cost of validating an arbitrary schema with `jsonschema` cannot be bounded
   beforehand by a static walk (`unevaluated*` re-runs branches, `referencing` decodes pointers differently from the
   walk, value-sized keywords).
5. **`jsonschema` reads the drafts differently.** Under drafts 4, 6 and 7 `jsonschema` 0.58.6 treats `format` as an assertion
   by default (`src/compiler.rs` of the `jsonschema` 0.58.6 crate) and compiles `contentMediaType` and `contentEncoding`
   as checks (its `src/keywords/mod.rs`). The subset gives each of its keywords one meaning in draft-07 and 2020-12 instead.
6. **Existing callers and runs may not match.** A definition whose schema its callers never satisfied, a version with
   an event trigger whose schema describes a manual input rather than the event envelope `{event, data}`
   (`crates/application/src/schedule_triggers.rs`), and a run started before the upgrade that will create a child
   or continue: all are refused from the first start after the upgrade.
7. **The value must not leak.** The run input is a classified payload. The existing `infra-schema::validate`
   (`crates/infra-schema/src/lib.rs`) formats instance paths and `Debug` text of the error kind, both of which can
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
  `RUN_INPUT_INVALID` or `RUN_INPUT_SCHEMA_UNUSABLE` (on a schedule's trigger-now, `SCHEDULE_INPUT_INVALID`,
  `NOMINAL_TIME_FIELD_INVALID` or `RUN_INPUT_SCHEMA_UNUSABLE`), and for an input refusal `details.rule` (one of the closed
  list of rules in `specs/043-run-input-validation/contracts/errors.md`, D043-049) and `details.field` (a JSON pointer,
  D043-049). A start whose input the caller did not send answers `field`
  `/` and `rule` `other`, regardless of the caller's read permission on the source, so the answer reveals nothing about
  an input the caller did not send: a repeat (D043-121), a trigger-now (D043-144) and a private rerun of row 13
  (D043-148). The graded-disclosure check that would show the field to a caller who may read the source input is not
  built (G043-040, O043-006). Nothing is created and no idempotency record is written. *Acceptance:* the problem shape
  in `crates/api-http/tests/run_input_errors.rs`, and the refusals of each start route in the row tests of SC-043-001.
- **FR-043-004** A refused start whose idempotency key already holds a run with the same fingerprint MUST replay that
  run (D043-004). A retry after a promotion of another active version computes another fingerprint, because the
  fingerprint holds the version id (`runs.rs`), and gets the existing `409` for a reused key, as it does today. Ad hoc
  starts replay through their own lookup (D043-029). *Acceptance:* integration tests for the replay, the `409` after a
  promotion, and the ad hoc replay.
- **FR-043-005** Signal-with-Start, Update-with-Start and a start with `USE_EXISTING` MUST be refused when the start
  input does not match, also when an open holder would answer and no run would be created (OD-2026-11-14-10).
  *Acceptance:* with an open holder, an invalid input is refused and the holder receives no signal and no Update.
- **FR-043-006** Schedules.
  1. A schedule create or update MUST refuse an input the target version refuses (`SCHEDULE_INPUT_INVALID`,
     `details.field` under `/action/input`), checked with `nominalTimeField` filled with the next nominal time, as the
     first action will fill it.
  2. It MUST refuse a `nominalTimeField` that is not a declared top-level string property of a target version that has
     an `inputSchema` (`NOMINAL_TIME_FIELD_INVALID`). The declaration is read as the validator applies the schema,
     through local `$ref`s and `allOf` members (D043-158).
  3. An action MUST be checked before `decide`, so a refused action is never buffered and trigger-now answers `422`
     instead of `BUFFERED`. An action buffered while valid MUST be checked again at its release (D043-126).
  4. An exhausted schedule is not checked: `decide` answers it `Exhausted` (D043-142).
  5. A refused action MUST be recorded with outcome `INPUT_INVALID` and its reason (`SCHEDULE_INPUT_INVALID`,
     `NOMINAL_TIME_FIELD_INVALID` or `RUN_INPUT_SCHEMA_UNUSABLE`, D043-123); its time is acted on, not owed, and no
     remaining action is used up.
  6. The decision that records it also commits (D043-026): for a scheduled action, and for a trigger-now action that was
     buffered and is refused at its release (kind `TRIGGERED`), the auto-pause when `pauseOnFailure` is set, with its
     audit; for a backfill step, the backfill's end as `CANCELLED`, without a pause (D043-134).
  7. A trigger-now refused before `decide` answers `422` and records nothing; one refused at its start after its check
     passed (a version promoted in between) returns the refusal and records nothing either (D043-146).

  *Acceptance:* `integration_schedules_bulk_operations` for write, action, backfill, trigger-now and `BUFFERED` to
  `INPUT_INVALID`.
- **FR-043-007** Triggers.
  1. A trigger whose run input is refused MUST start nothing and record the delivery to that trigger with no run.
  2. Only when that record is new, it MUST write one value-free audit record `run.input_refused`, in the same
     transaction (D043-027).
  3. It MUST count `trigger_events_total{outcome="input_invalid"}` or `{outcome="input_schema_unusable"}` on every
     dispatch that refuses, a redelivery's included (D043-129).
  4. It MUST still serve the event's other consumers. A redelivery of the event finds the delivery and records nothing
     again.
  5. A cron tick has no delivery: its audit is written in the transaction that moves the trigger past the tick, only by
     the dispatcher whose move found the trigger still at it, so once (D043-156).
  6. A tick whose key already holds a run (started by a peer in `report`, or by 0.2.x, that stopped before it moved the
     trigger) is not a refusal and is not audited (D043-165).

  *Acceptance:* `integration_events_webhooks_messaging` with two triggers, one refusing, and a wait, and a redelivery of
  the inbox event; `fault_events_webhooks_messaging` for two engines on one tick, a writer that stops, a crash between
  the delivery and its audit, and a peer's run.
- **FR-043-008** A child run whose input is refused MUST NOT be created; the parent step fails with
  `CHILD_INPUT_INVALID`, or `RUN_INPUT_SCHEMA_UNUSABLE` for a schema that does not compile. *Acceptance:*
  `integration_workflow_primitives`.
- **FR-043-009** A continue-as-new whose evaluated input is refused MUST create no successor; the predecessor ends
  `FAILED` with `CONTINUATION_INPUT_INVALID` or `RUN_INPUT_SCHEMA_UNUSABLE`. *Acceptance:* `integration_workflow_primitives`.
- **FR-043-010** A batch `RERUN` item refused for its input ends `FAILED` with `VALIDATION_FAILED`; a pending reset ends
  `ADMISSION_REFUSED`; neither is tried again (D043-013). *Acceptance:* `integration_schedules_bulk_operations`.
- **FR-043-011** A stored schema outside the subset.
  1. In `enforce` mode a stored `inputSchema` that fails the pre-check of Technical Plan 4 MUST refuse every start of
     its version with `RUN_INPUT_SCHEMA_UNUSABLE` (OD-2026-11-14-11).
  2. The failure is remembered: one pre-check per process, tenant and schema while the entry stays cached.
  3. Row 13 is the exception: its document is the caller's own and is parsed in the process's mode, as a publication is.
     In `enforce` a schema outside the subset is refused at the parse with the publication's issue code, naming the
     keyword (`INVALID_DEFINITION`); in `report` the gate reports it and the run is created.

  *Acceptance:* versions stored with a schema outside the subset (published in `report` mode, since publication refuses
  them in `enforce`) are refused on rows 1, 3, 4, 5, 6, 8, 9, 11, 12 and 14, and the row 13 parse in both modes (the row
  tests of SC-043-001); `the_cache_compiles_once_per_tenant_and_schema_and_remembers_a_failure_once`
  (`integration_run_input_cache`) counts one pre-check for 1,000 starts; the vocabulary test of T043-2 refuses each
  excluded keyword, a remote `$ref`, a cycle, a nested `$id`, a fractional `multipleOf`, `\d`, depth 33 and 33 patterns.
- **FR-043-012** Publication (and `sagat validate`) MUST run the same pre-check on every new `inputSchema`, annotated or
  not: in `enforce` mode a violation refuses the publication with a typed issue that names the keyword and its pointer;
  in `report` mode it is a warning with the same fields (D043-051). An `inputSchema` over 64 KiB or nested deeper than 32
  is refused first by feature 016's classification bound, `CLASSIFICATION_TOO_COMPLEX`, in both modes (D043-112,
  G043-020). A version with an `inputSchema` and a non-manual trigger gets a warning that the schema applies to the event
  envelope. *Acceptance:* `integration_gitops_versioning`,
  `crates/infra-ir/tests/` and the CLI tests.
- **FR-043-013** The mode (OD-2026-11-14-8).
  1. `SAGAT_RUN_INPUT_VALIDATION` is parsed strictly: `enforce` or `report`; unset or empty is `enforce`; anything else
     stops the process naming the variable.
  2. In `report` every check runs; a final refusal is counted (`outcome="reported"`) and audited, and the run is created.
  3. In `report` a start that an open holder answers creates no run; its refusal is audited with the holder's answer
     (ADR-0092 decision 5, D043-157).
  4. A transient refusal (Terms) is refused in both modes, since nothing was checked.
  5. A schedule write (row 14) creates no run: in `report` its refusal is counted and the write is stored, with no audit.
  6. A process lists the capability `run-input-validation-v1` in the instance registry only in `enforce`. Once the
     operator records it as a requirement for the control plane and the execution engine, a 0.2.x binary and a process
     in `report` keep their closed gate (ADR-0082), so replicas cannot answer differently; going back to `report` means
     removing the requirement first.
  7. No manifest, chart or compose file this repository ships sets `report`.

  *Acceptance:* the configuration tests of both apps (strict parse, default, the capability listed only in `enforce`),
  the registry test (recording the requirement is refused while a live process runs in `report`; once recorded, a
  process started in `report` stays up with its gate closed), `no_shipped_manifest_sets_run_input_validation_to_report`,
  and the `report` tests of T043-22.
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
- **FR-043-016** `NewRun.input`, `ContinuationSelection` and `EvaluatedChainHandoffCommand.input` hold a
  `CheckedRunInput`, whose fields and only non-test constructor are private to `crates/application/src/run_input.rs`.
  The architecture check (`scripts/check-architecture.py`) MUST fail on:
  1. an `INSERT INTO runs` (the name bare, quoted or qualified by `public`) outside its three places (`create` in
     `crates/infra-postgres/src/execution.rs`; `continue_as_new` and `continue_evaluated` in
     `crates/infra-postgres/src/chain_progress.rs`), counted per function so that a moved line is not a failure, or in
     any up migration;
  2. a call of `.continue_as_new(` (the legacy path) outside `tests/`, `crates/*/tests/` and `#[cfg(test)]`, except in
     the one forwarder, `continue_as_new` in `crates/infra-postgres/src/chain_progress.rs`, exempt by file and function
     (D043-122, D043-160);
  3. the `test-support` feature of `sagat-application` enabled anywhere but a `[dev-dependencies]` entry;
  4. a struct expression or an associated function call of `CheckedRunInput` in another file outside test paths, and a
     `Default` or `Deserialize` derive or impl for it anywhere (D043-045);
  5. `jsonschema` (a dependency path, a `use` or a qualified path) under `crates/infra-schema/src/run_input/` (D043-074);
  6. `tokio` as a dependency of `sagat-application` with a default feature or a feature other than `rt`, or named
     outside `crates/application/src/collection_source.rs` and test modules (ADR-0092 decision 1, D043-154);
  7. a `RunService`, `Scheduler`, `EventDispatcher` or `ScheduleService` that a composition root under `apps/*/src`
     builds without `.with_run_inputs` in its builder chain (ADR-0092 decision 2, D043-161).
  *Acceptance:* a negative fixture for each rule in `scripts/tests/test_check_architecture.py`.
- **FR-043-017** A `CheckedRunInput` carries the version id it was checked against. `PgRunStore::create` and
  `continue_evaluated` MUST refuse one whose version is not the run's (`StoreError::Internal`, logged without the value).
  *Acceptance:* a unit test of each store against a mismatched input.
- **FR-043-018** `control-plane input-schema-inventory` MUST list every stored version whose `inputSchema` fails the
  pre-check, with its ids, the keyword and its pointer, and nothing of the schema or of any input; list separately, for
  information, the versions within the subset that have an `inputSchema` and a non-manual trigger; and exit 0 when none
  would be refused, 1 when one would, 2 when the command failed or could not judge a version (a stored document that is
  not JSON, a schema that made the validator panic; D043-052, D043-072, D043-187). A version outside the subset that
  nothing starts any more (superseded: not its workflow's active version, pinned by no schedule that is not deleted, no
  open run, no run created in the last 30 days) MUST be shown only as a number in the summary line and MUST NOT change
  the exit status; `--all` lists it (OD-2026-11-16-5, D043-179, D043-185). A rerun or a reset of a run of it created
  before those 30 days is not foreseen: in `enforce` it is refused `422 RUN_INPUT_SCHEMA_UNUSABLE` when asked for, as
  rows 4 and 5 refuse every start of a version outside the subset (`an_unusable_schema_refuses_with_start_rerun_and_trigger_now`,
  `row_5_a_reset_of_a_version_whose_schema_is_unusable_is_refused_schema_unusable`). The schedules are feature 018's, which
  the v0.2.0 release does not have: on its database, read before the migration, the inventory reads no schedule
  (D043-187). *Acceptance:* an integration test stores one version inside the subset with an event trigger and one with
  `patternProperties`, runs the command, finds the second as refused and the first as informational, and sees exit
  status 1; with only the first stored, exit 0; against an unreachable database, exit 2;
  `a_superseded_version_nothing_starts_does_not_fail_the_inventory_and_all_lists_it` (a quiet superseded version is
  only in the summary's number, and listed by `--all`; one with an open run, or with a run created in the last 30 days,
  is refused and makes the exit 1), `a_version_a_live_schedule_pins_is_never_superseded` and
  `the_inventory_runs_on_a_database_of_0_2_0_before_the_migration`, each run as the runtime role.

### Non-functional

- **NFR-043-PERF** In `enforce` mode validation adds no SQL statement to a start, a triggered start, a child start or a
  continuation. In `report` mode a reported refusal writes one audit row, which adds a statement to that start. A
  schema is compiled at most once per process and tenant while its entry stays cached. Compiling and validating never
  run inside a database transaction (row 12 reads the schema under the run's lock and checks after it is released),
  and run off the async workers (`block_in_place`); so does a pre-check outside a start, on a permit of the starts' pool
  (Technical Plan 3, D043-177).
- **NFR-043-BOUND** The work of a schema the platform did not write is bounded by numbers fixed in the build, each in
  the contract `specs/043-run-input-validation/contracts/input-schema-subset.md`: a compile by the pre-check's limits (at
  most 64 KiB of schema, a nesting depth of 32 and 32 patterns); a validation by 1,000,000 work units, an input depth of
  64 and a 5 s backstop; at most four validations and pre-checks at once per process, each waiting at most 5 s for its
  permit; and the compiled form of every admitted schema within one tenant's share of the cache, 16 MiB (D043-105,
  D043-159).
- **NFR-043-SEC** No new grant, role membership or `SECURITY DEFINER` function for the runtime role. No tenant code runs
  in the control plane or the engine; no remote reference is resolved (only local `#/...` pointers within the
  document); a compiled schema is shared only within its tenant.
- **NFR-043-COMPAT** The upgrade order of OD-2026-11-14-3 (migrate, engines, control planes) holds; until the operator
  records `run-input-validation-v1`, old and new binaries, and processes in either mode, run side by side (G043-001,
  G043-010).

### Success criteria

- **SC-043-001** Rows 1 to 14 of Technical Plan 1 each have the test of FR-043-001 or FR-043-006, passing on real
  PostgreSQL, for both `RUN_INPUT_INVALID` and `RUN_INPUT_SCHEMA_UNUSABLE` where the row has both, but row 2's
  `RUN_INPUT_SCHEMA_UNUSABLE`: an uploaded-input start reaches the gate through row 1's path once its descriptor is
  built, and row 1's test covers that refusal (G043-056).
- **SC-043-002** The sentinel test of FR-043-014 passes for every row that carries a caller's value (all but row 10,
  whose input is the platform's tick).
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
- Asserting `format` or `content*`, resolving remote references, or a dialect of our own.
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

Answered (`docs/owner-decisions.md`); each task that builds one cites it in an `Owner-gate:` line:

- OD-2026-11-14-2: every start is validated in 0.3.0.
- OD-2026-11-14-8 (O043-001): `enforce` by default, `report` while callers are fixed, the mode through the registry.
- OD-2026-11-14-9 (O043-004): children and continuations of runs created before the upgrade are validated too, an
  accepted deviation from principle IV.
- OD-2026-11-14-10: a Signal-with-Start or Update-with-Start is refused for its start input even when a holder is open.
- OD-2026-11-14-11 (O043-002, O043-003): a schema outside the subset refuses every start of its version, permanently for
  the creators nobody waits for; a repeat of a run from before 0.3.0 is validated like any start.
- OD-2026-11-14-14: the supported subset and the platform's validator with a work counter.
- OD-2026-11-14-15: ADR-0090 accepted; T043-1 to T043-24 built before approval.
- OD-2026-11-15-3: the start operations of the OpenAPI describe their `422`; the 016 and 035 baselines re-pinned once.
- OD-2026-11-16-3: the approval path after the second review round: the findings marked for a fix were fixed and
  re-read, and every other Major was accepted as it stands, one row each (OD-2026-11-16-100 upward).
- OD-2026-11-16-4 (O043-007): ADR-0092 accepted; moving design 042's parse behind a port stays G043-052 (T043-25).
- OD-2026-11-16-5: the inventory does not count a superseded version that nothing starts; `--all` lists it.
- OD-2026-11-16-6: the per-tenant share of the cache stays, with its limit documented (Technical Plan 3, Capacity;
  T043-28 measures it).

Open, each with its options and a recommendation in `research.md`. None blocks what is built, because the design states
the built behaviour and what each answer would change:

- O043-005: an `inputSchema` with tuple positions (`prefixItems`), which feature 016 classifies and feature 035 reads for
  the levels of an Iceberg write, is outside the subset. In `enforce` such a version is refused at publication and every
  start of a stored one is refused; once the requirement is recorded, `report` is no longer available, so until tuples
  join the subset such a workflow can start only with its schema rewritten without them (an `items` schema with
  `minItems` and `maxItems`, whose positions 035 then reads as one level, the strictest). Recommended: accept for 0.3.0
  and design tuples into the subset in a later release.
- O043-006: repeats, trigger-now and the private rerun answer `/` and `other` to every caller (G043-040). Recommended:
  accept for 0.3.0; build the graded-disclosure check of D043-037 only if callers ask for it.

## Critical User Journeys (CUJs)

### CUJ 1: A caller sends the wrong input

*   **Given**: a version whose `inputSchema` requires `orderId` as a string.
*   **When**: a caller starts it through the TypeScript SDK with `{"orderId": 42}`.
*   **Then**: the call fails with `VALIDATION_FAILED`, `reason` `RUN_INPUT_INVALID`, `field` `/orderId`, `rule`
    `type`; no run exists; `42` appears nowhere.

### CUJ 2: A schedule whose input no longer fits

*   **Given**: a schedule targeting the active version, with `pauseOnFailure`, and a new version published that
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
*   **Then**: the starts refused for their input or their schema are admitted again; every such attempt is counted
    `reported`, and each created run, or each answer of an open holder, is audited once. A validator with no permit still
    answers `503`: `report` admits what was checked and refused, never what could not be checked. The final outcomes
    recorded before the switch (an `INPUT_INVALID` action, a failed child step, a trigger's delivery without a run) stay
    as they were. When the caller is fixed, the operator runs `control-plane input-schema-inventory` again (versions
    published in `report` may be outside the subset) and expects exit 0, which a superseded version that nothing starts
    does not prevent (`--all` lists those, OD-2026-11-16-5), restarts every replica in `enforce`, and
    records the requirement. Recording is refused while any live replica still runs in `report` (ADR-0082 decision 2);
    once it is recorded, a replica restarted in `report` stays up with its gate closed.

## Technical Plan (Architecture)

### 1. The creators

Rows 1 to 13 are the thirteen paths that create a run, and all of them are checked (D043-003, OD-2026-11-14-9).
Row 14 is not a creator: the schedule write uses the same check. The unnumbered legacy row is reachable from tests only
and the architecture check keeps it there (FR-043-016). Each entry names a file and the function to search for in it.

| # | Creator | Entry | Where the run is created | Input | `Invalid` | `SchemaUnusable` |
|---|---|---|---|---|---|---|
| 1 | `POST /runs` (SDKs, CLI `apps/cli/src/workflow.rs`, Studio) | `crates/api-http/src/routes/execution.rs` → `start_with_options` `crates/application/src/runs.rs` | `start_with` `runs.rs` → `store.create` `runs.rs` | caller | `422 RUN_INPUT_INVALID` | `422 RUN_INPUT_SCHEMA_UNUSABLE` |
| 2 | Uploaded-input start | `execution.rs` → `start_with_blob` `runs.rs` | as 1; the check at `runs.rs` is replaced by the gate | the blob descriptor (Terms) | as 1 | as 1 |
| 3 | Signal-with-Start, Update-with-Start | `execution.rs` → `runs.rs` | as 1 | caller | as 1, also with an open holder | as 1 |
| 4 | `RERUN_AS_NEW`, batch `RERUN` | `execution.rs` → `rerun_item` `runs.rs`; batch `crates/application/src/runs/batch_items.rs` | as 1 | the stored input of the ended run | as 1 with `field` `/` and `rule` `other` (D043-121); batch item `FAILED VALIDATION_FAILED` | as 1; batch item as for `Invalid` |
| 5 | Reset, pending reset | `crates/api-http/src/routes/resets.rs` → `reset.rs`; pass `crates/application/src/runs/pending_reset.rs` | `create_from_prefix` `crates/application/src/runs/reset.rs` → `start_with` | the stored input of the source | as 4; pending reset `ADMISSION_REFUSED` | as for `Invalid` |
| 6 | Schedule action | `crates/scheduler/src/schedules.rs` → `fire` `crates/application/src/schedule_firing.rs` → `start` | `ScheduledStarts::start_scheduled` `runs.rs` → `start_with` | the schedule's input with `nominalTimeField` (`schedule_firing.rs`) | action `INPUT_INVALID` (`SCHEDULE_INPUT_INVALID` or `NOMINAL_TIME_FIELD_INVALID`), the schedule paused when `pauseOnFailure` | action `INPUT_INVALID` (`RUN_INPUT_SCHEMA_UNUSABLE`), as for `Invalid` |
| 7 | Schedule backfill | `crates/scheduler/src/schedules.rs` → `backfill_step` `schedule_firing.rs` | as 6 | as 6 | as 6, the backfill ends `CANCELLED` and the schedule is not paused (D043-134) | as 6, the backfill ends `CANCELLED` |
| 8 | Schedule trigger-now | `crates/api-http/src/routes/schedules.rs` → `trigger` `schedule_firing.rs` → `trigger_once` | as 6 | as 6 | `422 SCHEDULE_INPUT_INVALID` with `field` `/` and `rule` `other` (D043-144) before `decide`, nothing recorded | `422 RUN_INPUT_SCHEMA_UNUSABLE`, nothing recorded |
| 9 | Event, webhook, message trigger | `crates/scheduler/src/events.rs` `dispatch` → `start_run` → `start` | `store.create` `events.rs` | the event envelope | delivery without a run + one audit + metric | as for `Invalid`, its own reason and metric |
| 10 | Cron trigger (the legacy cron pass, until the cron triggers are handed over to the schedule pass, ADR-0083) | `cron_pass` `events.rs` → `start` | as 9 | `{event, data: {scheduledAt, firedAt}}` | one audit with the move of the trigger, none when the tick's key holds a run (D043-156, D043-165) | as for `Invalid` |
| 11 | Child run | `crates/scheduler/src/passes.rs` `start_child` | `store.create` `passes.rs` | the parent node's evaluated input | parent step `CHILD_INPUT_INVALID`, unless its child was claimed (D043-164) | parent step `RUN_INPUT_SCHEMA_UNUSABLE`, as for `Invalid` |
| 12 | Continue-as-new (evaluated) | `advance_continuation_in_view` `passes.rs` | `continue_evaluated` `crates/infra-postgres/src/chain_progress.rs` | the evaluated expression | predecessor `FAILED CONTINUATION_INPUT_INVALID` | predecessor `FAILED RUN_INPUT_SCHEMA_UNUSABLE` |
| 13 | Ad hoc database execution, connection probe, private rerun | `crates/api-http/src/routes/database_executions.rs` → `crates/application/src/runs/ad_hoc.rs` | `store.create` `ad_hoc.rs` | caller, or `{}`; a private rerun the stored input | as 1 when the document declares a schema; a private rerun `field` `/` and `rule` `other` (D043-148) | in `enforce` refused at the parse with the publication's issue, naming the keyword; in `report` reported, the run created (FR-043-011) |
| 14 | Schedule create and update (not a creator) | `crates/application/src/schedules.rs` → `check` | none | the schedule's input with `nominalTimeField` filled with the next nominal time (D043-158) | `422 SCHEDULE_INPUT_INVALID`, `NOMINAL_TIME_FIELD_INVALID` | `422 RUN_INPUT_SCHEMA_UNUSABLE` |
| — | Continue-as-new (legacy, tests only) | `chain_progress.rs` | `chain_progress.rs` | the predecessor's input, byte for byte | not checked | not checked |

The refusals of the creators that nobody waits for (the batch item of row 4, the pending reset of row 5, rows 6, 7
and 9 to 12, all run by the engine) are final: nothing retries an input or a schema the version refuses
(OD-2026-11-14-11). An input past the work budget is final the first time too. The transient cases are a busy or
starved machine, never the input: no permit within the wait (`Busy`, which is also what a service built without a gate
answers, D043-120), a validation stopped by the backstop, and a compile or a validation that panicked (`Interrupted`,
D043-106, D043-166). A caller gets a retryable `503`; the engine's creators try again later. A trigger's event that
waits so is waiting for a permit, as for a full quota: it uses up none of its attempts, and only an event that has waited
for a day (`GIVE_UP_AFTER` of `crates/scheduler/src/events.rs`) ends `DEAD`, so only a validator starved for a day drops
a trigger start (G043-054). A batch `RERUN` item that finds no permit is deferred and counts an attempt, so one starved
past the executor's attempts ends `FAILED` (G043-055).

The creator label of each row, in the metrics of Technical Plan 7: `api` for rows 1 to 3, `repeat` for rows 4 and 5,
`schedule` for rows 6 to 8 and 14, `trigger` for rows 9 and 10, `child` for row 11, `continuation` for row 12 and
`ad_hoc` for row 13.

Paths that do not create a run today: a whole-run retry or replay, a script test run (it writes `script_test_runs`), a
gRPC start (`crates/api-grpc` is worker-only), internal apps (feature 027) and cross-namespace services (feature 030).
When one is built, the type of FR-043-016 makes it go through the gate.

### 2. The schema travels with the definition

The IR codec (`crates/infra-ir/src/lib.rs`, `parse_stored`, and its private twin) puts the canonical bytes of
`inputSchema` into the parsed `WorkflowDefinition` as an `InputSchema`: an `Arc<[u8]>` and a `[u8; 32]` digest. The
codec in `infra-ir` computes the SHA-256 once at parse; the domain only stores the bytes and the digest, with no
`serde_json` and no hashing crate (`sha2` is not in `DOMAIN_ALLOWED`, `scripts/check-architecture.py`). `WorkflowDefinition::footprint` counts the bytes, so the
`DefinitionCache` byte budget (`crates/application/src/execution.rs`) still bounds memory. Every creator holds the
definition when it needs the schema: rows 1 to 8, 13 and 14 through `get_version_by_id` and `get_or_parse`
(`runs.rs`; for row 14, `target_version`), rows 9 and 10 at `events.rs`, row 11 at `passes.rs`,
row 12 from the run's definition, read under the run's lock through `tx.parts()`; the check itself runs after that
transaction commits and the lock is released, and the hand-off that follows is fenced by the run's version
(`expected_run_version`), so a run that moved in between is not continued. No creator on a budgeted path
reads the database for the schema. Two paths off the budget do: the schedule write of row 14 reads the target version's
document (`ScheduleStore::target_version`, one statement, D043-125), and the check before `decide` of rows 6 to 8
resolves the version again as a start does (`ScheduledStarts::check_scheduled`, D043-126, G043-041).

### 3. The gate, the validator and the type

Two pieces, so that the port is narrow and the policy is in one place (D043-017):

```text
// crates/application/src/run_input.rs — the port, implemented in crates/infra-schema, which re-exports its types (D043-127)
trait SchemaValidator: Send + Sync {          // object-safe: held as Arc<dyn SchemaValidator> (D043-076)
    // async; the adapter owns the permits, the wait, the off-worker step and the backstop (D043-071)
    fn check<'a>(&'a self, key: SchemaKey, schema: &'a InputSchema, input: &'a [u8])
        -> Pin<Box<dyn Future<Output = Result<(), SchemaRefusal>> + Send + 'a>>;
}
struct SchemaKey { tenant: TenantId, digest: [u8; 32], shared: bool }   // built by the gate; shared = creator != AdHoc
enum SchemaRefusal { Invalid { segments: Vec<PathSegment>, rule: Rule }, Unusable(Unusable),   // final
                     TooDeep, WorkLimit,                                                       // final
                     Busy, Backstop, Interrupted }     // transient: no permit, the clock, a panic of the validator

// crates/application/src/run_input.rs — the concrete gate
struct RunInputGate { validator: Arc<dyn SchemaValidator>, mode: Mode }   // no runtime, no timer, no permit; not generic
impl RunInputGate {
    fn unwired() -> Arc<Self>;          // what a service holds until it is given a gate: Busy for every schema (D043-120)
    async fn check(&self, tenant, version: WorkflowVersionId, definition: &WorkflowDefinition,
                   input: Vec<u8>, creator: Creator, field_policy: FieldPolicy)
        -> Result<CheckedRunInput, InputRefusal>;
    async fn check_schema(..., schema: Option<&InputSchema>, ...) -> ...;   // a continuation holds only the schema
    async fn recheck(...) -> ...;        // a schedule's start: counts every outcome (D043-147, D043-184)
    async fn check_before_its_start(...) -> ...;  // a schedule's action before its decision and at its release:
                                                  // counts only what stops it
    async fn check_ahead(...) -> ...;    // a `CANCEL_SOURCE` reset's request: counts only what stops it
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
- **A service without a gate.** `RunService`, the `Scheduler` and the `EventDispatcher` hold
  `RunInputGate::unwired()` until a composition root gives them one with `.with_run_inputs`. Its port answers `Busy` for
  every schema, so a start of a version with an `inputSchema` is refused as transient (`503`, or tried again by the
  engine) and is never admitted unchecked; a version without one passes as it would through a wired gate (D043-120). A
  composition root that forgets the gate would leave an engine creator retrying forever, so the seventh rule of
  FR-043-016 fails on it (ADR-0092 decision 2, D043-161). Until it is given a gate, a `ScheduleService` skips the write
  check (it does not answer `Busy`), leaving the action-time check, which is the authority (D043-008, D043-125); the
  seventh rule keeps that to tests.
- **The type** carries the version id; `PgRunStore::create` (`crates/infra-postgres/src/execution.rs`) compares it
  with `NewRun.info.version_id`, and `continue_evaluated` with the predecessor's version (FR-043-017). The existing
  `RunInputChecker` (`crates/application/src/blobs.rs`) is removed.
- **The bound on the work is the validator's own counter** (D043-047). The validator of Technical Plan 4 charges units as
  it runs and stops at `RUN_INPUT_WORK_UNITS` (1,000,000). A budget miss is deterministic for a schema and an input, so
  it is final the first time, on every row: `RUN_INPUT_INVALID` with `rule` `workLimit` and `field` `/`. No static
  estimate of the work is needed or made: the counter charges each step before the validator takes it, and a test holds
  the validator's primitive operations within a constant multiple of the units charged (Technical Plan 4).
- **Where the runtime lives** (D043-071, ADR-0092). The gate is plain async code over the port and names no runtime.
  The permits, the wait, the off-worker step and the backstop are inside the adapter, `crates/infra-schema`, which has
  tokio (`sync`, `time`, `rt-multi-thread`) as a normal dependency, as other adapters have; the adapter has its own
  `off_worker`, a twin of the private helper of `crates/infra-postgres/src/payload_disclosure.rs`, and there is no new
  edge into `application`. `sagat-application` itself depends on tokio with no default features and the `rt` feature
  alone, for one blocking parse of design 042 in `collection_source.rs`, the only file of the crate that may name tokio
  outside test modules (ADR-0092 decision 1, D043-150, D043-154); moving that parse behind a port would restore the
  stricter rule (G043-052).
- **Threads and permits** (D043-050, D043-068, D043-140), in the adapter. Four permits per process
  (`RUN_INPUT_PERMITS`), one pool however many services hold the gate (`RunInputValidator::process()`), shared
  work-conservingly among tenants: a free permit goes to the waiting tenant that holds the fewest, FIFO among equals, so
  a tenant is granted a third or a fourth permit only while no other tenant waits. A tenant alone on the process can use
  all four; when a second arrives it is served first as permits come back, and nothing held is taken away (each
  validation is bounded by the counter and the backstop; a full budget costs at most 7.5 ms on the development machine,
G043-017). A waiter waits at most `RUN_INPUT_PERMIT_WAIT` (5 s); its
  guard leaves the queue, or returns its permit, when its future is dropped. The queue holds one entry per caller that
  waits, so it is bounded by the callers in flight and each entry by the wait. The validator runs off the async workers
  and holds its permit until it returns; `specs/043-run-input-validation/research.md` section 5 has the details. A
  pre-check outside a start (publication, a dry run, an ad hoc document) takes its permit from the same
  pool and waits as long. The parser that runs a pre-check does not know the tenant, so the queue treats all pre-checks
  as one more tenant: a permit goes to whichever holds fewer, a tenant's starts or the pre-checks, so neither can starve
  the other. Among themselves the pre-checks of different tenants are first come, first served (G043-057). With no
  permit, or when the validator panics, the document is not judged and the caller gets a retryable `503`, as for a
  start (D043-177, D043-186). The inventory runs in a process of its own, with a pool of its own that nothing else
  uses.
- **The backstop** (D043-050). The counter also reads the clock every 4,096 units; past `RUN_INPUT_BACKSTOP` (5 s) of
  work it stops with `SchemaRefusal::Backstop`. `Busy`, `Backstop` and `Interrupted` become `InputRefusal::Unavailable`
  in the gate: a caller gets a retryable `503`, and the engine's creators treat it as a lack of permits and try again later,
  as they do for a full quota. All three mean a busy or starved machine, not an input, and are never remembered or
  cached. A panic of the compile or of the validation is caught where the adapter runs it and answered `Interrupted` to
  the caller and to those who waited for the same compile, logged with the schema's digest only (D043-106, D043-166);
  a compile that panics for a given schema would panic again, so such a schema is tried again on every start
  (G043-053).
- **Capacity.** The permits, the waits, the budget and the cache's size are constants of the build, not settings: the
  cache's weight bound counts the matchers the four permits allow, so the permit count cannot change at run time
  without breaking it. A process validates at most four inputs at once; a full budget takes at most 7.5 ms on the
  development machine (G043-017), so a process completes a few hundred full-budget validations a second at worst, and
  typical inputs cost far less (CI and production unmeasured, G043-006, T043-27). An engine pass waits for its permit
  inline, up to 5 s, then leaves the work for the next pass. Past 256 compiled schemas (64 per tenant) the least
  recently used is evicted and compiled again on its next start, under a permit (16 ms for the heaviest in a test
  build). The tenant share is not work-conserving: a tenant alone on a process keeps at most 64 compiled schemas, so
  one that starts more than 64 schemas often recompiles some of them on their starts while the rest of the cache stays
  empty. The owner kept the share for 0.3.0 with this limit documented (OD-2026-11-16-6); T043-28 measures it. More
  throughput comes from more replicas.
- **Input depth.** Before validating, the input is parsed with a depth limit of 64. A deeper input, and one deeper than
  `serde_json` parses (its recursion limit is 128), is `Invalid` with `rule` `maxDepth` and `field` `/`, never an
  internal error. A version without a schema keeps today's checks only.

### 4. The validator

`InputSchemas` in `crates/infra-schema/src/run_input/` is the platform's validator for a documented subset of JSON
Schema (OD-2026-11-14-14, D043-046 to D043-049). The subset, its pattern allowlist, its limits and its charges are the
contract `specs/043-run-input-validation/contracts/input-schema-subset.md`; `jsonschema` is not on this path, and is the
oracle of a test only. What the design fixes:

- **The pre-check**, before anything is compiled, at publication, at start (cached) and in the inventory: the size and
  the depth; a vocabulary walk that records every schema position and refuses a keyword outside the subset; every
  `$ref` decoded by the validator itself to a recorded schema position (D043-056); a search over the application edges
  that refuses a cycle, a path longer than 128 and a schema whose least work passes the budget (D043-069, D043-101);
  every pattern against its allowlist and size; every name against 1 KiB (D043-102). A `$ref` is compiled to the index
  of its target, never an inlined copy, so a DAG of references stays the size of the document. The first violation is
  `Unusable` with the keyword and its pointer in the schema, author vocabulary only, cut to 512 characters (D043-104).
- **The counter** (D043-047, D043-058). Every loop step and every subschema application is charged before it runs, so
  the work of a validation is proportional to its units whatever a keyword holds; a charge that would pass the budget
  stops the whole validation with `workLimit`, never one branch. The counter is a cost model, not a measure: an
  adversarial test per keyword counts the validator's primitive operations and holds them within k × the units charged
  (k at most 32; the measured values are in G043-017), and the pattern charge assumes no bit-parallel speed-up.
- **Determinism** (D043-059). No iteration depends on a hash seed, so every replica of one release reports the same
  first violation, `field` and `rule`.
- **The cache** (D043-064, D043-105, D043-159). The classification cache of feature 016, made generic over key, value and
  failure, holds the compiled form keyed by `(tenant, digest)`: identical schemas of one tenant share an entry, two
  tenants never do. 256 entries and 64 MiB per process; one tenant holds at most a quarter by count and by weight, and
  an entry of a tenant at its share evicts that tenant's oldest. Every schema the pre-check admits weighs at most that
  share, so it is compiled once and kept (`the_heaviest_schema_the_precheck_admits_fits_a_tenant_share_and_is_kept`: the
  heaviest weighs 7.0 MiB of the 16 MiB share). Single flight per key; a remembered `Unusable` per key, 1,024 at most.
  An ad hoc key (`shared: false`, row 13, a new version per call) is compiled for its call and not kept, under a permit
  like every other (ADR-0090 decision 6, G043-031).
- **The refusal** (D043-049): a closed `Rule` and a pointer of declared names and indices only; never a value.
- **The oracle test** (T043-2, D043-060). A property test generates schemas of the subset and inputs and checks that the
  platform's validator and `jsonschema` 0.58.6 (formats off, references refused) give the same verdict in the three
  dialect choices; a regression check of the subset's meaning, not a proof. Cases the platform stops at `workLimit`,
  `maxDepth` or the backstop are skipped, and the test fails above 5% skipped.

### 5. What each creator does with a refusal

- **The synchronous answers (rows 1 to 5, 8 and 13).** `AppError::InvalidRunInput { reason, field, rule }` (D043-014), mapped in
  `crates/api-http/src/error.rs` beside `InvalidStart` to `422 VALIDATION_FAILED`, with remediation lines beside
  `SCHEDULE_INPUT_INVALID`; `InputRefusal::Unavailable` (no permits within 5 s, the
  backstop) maps to `AppError::Unavailable`, and a budget miss to `InvalidRunInput` with `workLimit`. Before answering a
  refusal, final or transient, `start_with` looks the key up and answers a run of the same key and fingerprint as any
  replay is answered (D043-004, D043-141, D043-151): what a Signal-with-Start or an Update-with-Start delivered or
  applied, "already acted" for a schedule, "already completed" for a pending reset; `start_private` has its own lookup
  (D043-029). Rows 4 and 5, the trigger-now of row 8 and the private rerun of row 13 use `FieldPolicy::Root`: `field`
  `/` and `rule` `other`, regardless of the caller's read permission (D043-121, D043-144, D043-148; G043-040). A
  `CANCEL_SOURCE` reset checks the source's input at its request, before it cancels the source, since the version and
  the input it will use are fixed then (D043-167); the completion checks again and stays the authority.
- **The repeats nobody waits for (rows 4 and 5).** A batch `RERUN` item refused for its input ends `FAILED
  VALIDATION_FAILED`, a pending reset `ADMISSION_REFUSED`, neither tried again; `error_code` and `failure_of`
  (`crates/application/src/batch_executor.rs`, `crates/application/src/reset_completion.rs`) name the variant.
- **Rows 6 to 8 (schedules).** `trigger_once` and the scheduled path check the input before `decide` through
  `ScheduledStarts::check_scheduled`, which resolves the version and its definition as a start does (D043-126) and calls
  `RunInputGate::check_before_its_start`, before the decision and again at the release of a buffered action; the start
  then checks again, against the version resolved then, with `RunInputGate::recheck`. The checks before the start count
  only what stops the action, a final or a transient refusal, and the start counts every outcome, so one action is
  counted once (D043-147, D043-184). A `CANCEL_SOURCE` reset's request checks with `RunInputGate::check_ahead`, which
  counts the same way. An exhausted schedule is not checked (D043-142). A refused trigger-now answers `422` and records
  nothing, also when the refusal comes at its start (D043-146). A refused scheduled action, or a trigger-now action
that was buffered and is refused at its release (FR-043-006, item 6), records, in one
  `ScheduleDecision`, the action (`Outcome::InputInvalid`, the reason of D043-123), the auto-pause and its
  `ScheduleAutoPaused` audit when `pauseOnFailure` is set; a refused backfill step records the action and the
  backfill's end as `CANCELLED`, as `revoke_authority` does (`schedule_firing.rs`), and does not pause the schedule
  (D043-134). The arm sits before `NotAllowed`, so nothing falls to `Err(other)`. A `BUFFERED` action
  is checked again at its release and may become `INPUT_INVALID`; the waiting row is settled where it stands
  (`settle_waiting`, D043-126). Two migrations add `INPUT_INVALID` to the action outcomes and `run-input-validation-v1`
  to the registry's capability lists and its requirement history (D043-131): `20270118000000` drops each old CHECK in
  the `ALTER TABLE` that adds its successor `NOT VALID` (a superset, named `*_known`), and `20270118000100` validates
  them; a static test keeps every later CHECK added `NOT VALID` (D043-145, D043-153). The down migration of `20270118000000` refuses
  while a row holds a new value; the second's has nothing to undo.
- **Row 14 (schedule write).** `ScheduleService::with_run_inputs` gives the write check the gate and a definition cache
  (D043-125); `check_input` (`crates/application/src/schedules.rs`) reads the target version's document
  (`ScheduleStore::target_version`), fills `nominalTimeField` with the next nominal time, as the first action will
  (D043-008, D043-158), runs the gate, and then requires the field to be a declared top-level string property, read
  through local `$ref`s and `allOf` members. The write check is never stricter than the action check, except for the
  one rule of feature 018's contract: the field must be declared. The write path is not on a budget.
- **Rows 9 and 10 (triggers).** In `events.rs` `start`, after `get_or_parse`: one store call writes, in one
  transaction, a delivery to `trigger:<id>` with no run (`event_deliveries.run_id` is nullable,
  `crates/infra-postgres/migrations/20261012000000_events.up.sql`) and, only when that row is inserted, the audit
  record `run.input_refused` (resource the workflow version, the reason `<reason> rule=<rule> via trigger <id>`, D043-128);
  then the metric, once per dispatch (D043-129), and `Ok(Started::No)`. A dispatch that waits for a permit
  is recorded on the event as "the run input could not be checked now; will retry". A retry
  of the event after another consumer failed (`events.rs`) finds the delivery and writes nothing. A cron firing
  has no inbox event; its refusal audit names the tick's key (`tick_identity`) in its reason, and is written in the
  transaction that moves the trigger past the tick, only by the dispatcher whose move found the trigger still at that
  tick, so the tick is audited once however many dispatchers reach it and wherever one stops (D043-156, closing
  G043-002). Before it, the dispatcher looks the tick's key up: a run a peer started under it (in `report`, or a 0.2.x
  engine, that stopped before it moved the trigger) makes the tick started, not refused (D043-165).
- **Row 11 (child).** In `start_child`, after `get_or_parse`, a refusal fails the parent step with its code only if,
  under the step's row lock, the step still waits for that child and its reservation was not claimed
  (`Condition::ChildNotStarted`, `RunTx::child_started`, D043-164). A child that a peer, a 0.2.x engine or an engine in
  `report` created claims the reservation in the transaction that creates it (`claim_reservation`), so a claim that
  committed first is seen, and one that comes while the refusal holds the row waits and then finds the step moved on
  and creates nothing. One statement, on the refusal path only; it replaces the lookup of D043-038 and D043-132, which
  left a window between the lookup and the failure.
- **Row 12 (continuation).** After the evaluation, the run's lock released, with `check_schema` on the schema read
  under the lock: a refused input ends the predecessor through the fenced path of `finalize_continuation_expression`,
  given the failure code, and counts `scheduler_continuation_input_refusals_total`; an input that cannot be
  checked now is `ContinuationAdvanceError::InputUnavailable`, counted as a wait
  (`scheduler_continuation_waits_total{blocker="input_check"}`), and left for the next pass (D043-133). The failure
  code is the run's stop code, which the REST `Run` shows as `failureCode` (D043-162).

### 6. The mode and the registry

`SAGAT_RUN_INPUT_VALIDATION` is read once at start by `apps/control-plane` and `apps/execution-engine`, strictly (the
ADR-0081 pattern): `enforce`, `report`, unset or empty meaning `enforce`, anything else a start-up error naming the
variable. The process logs its mode. Only in `enforce` does it list `run-input-validation-v1`
(`apps/execution-engine/src/capabilities.rs` and the control plane's twin); the capability is added to
`Capability` (`crates/application/src/instances.rs`), to the closed lists of the registry and its requirement history
by the migrations of the action outcome (Technical Plan 5), and to the registry's data model
(`specs/018-schedules-bulk-operations/data-model.md`). The operator records the requirement for `CONTROL_PLANE` and
`EXECUTION_ENGINE` with `platform_requirement_activate`, run with the owner role
(`docs/runbooks/instance-registry.md`), once every live process of those kinds lists the capability: ADR-0082 decision
2 verifies every live instance and refuses to record a requirement while one lacks it, so the requirement cannot be
recorded while a replica runs in `report` or a 0.2.x binary is live. From then on a process started in `report`, or a
0.2.x binary, stays up with its gate closed (Terms). Going back to `report` is: remove the requirement
(`platform_requirement_remove`), then restart in `report`. The charges, the budget and the pre-check's limits of
`specs/043-run-input-validation/contracts/input-schema-subset.md` belong to this capability: a release that changes
them so that an accepted input or schema is refused ships `run-input-validation-v2`, required the same way, so two
releases with different charges never answer side by side once the operator requires it. In `report` mode a refusal becomes `Checked::Reported`, is counted, and its
audit record, `run.input_refused` on the run with the reason `reported: <reason> rule=<rule>` (D043-128), is written in
the transaction that creates the run (one statement on that start), so a retry or a replay of the same start does not
audit twice (D043-039); for a continuation, in the hand-off transaction (D043-143). A start that an open holder answers
creates no run: its audit is written with the receipt a Signal-with-Start or an Update-with-Start keeps, so once across
its retries, and for a plain start answered by its holder, which keeps no receipt, in a statement of its own with each
answer (ADR-0092 decision 5, D043-157).

Because the engines are upgraded first and default to `enforce`, an operator who wants `report` for the upgrade sets
`SAGAT_RUN_INPUT_VALIDATION=report` in the manifests of every engine and control plane **before** migrating, after
running `control-plane input-schema-inventory`; otherwise the first upgraded engine records final outcomes (the batch
items of row 4, the pending resets of row 5, rows 6, 7 and 9 to 12) before anyone can choose (D043-040).

### 7. Telemetry

`run_input_validations_total{creator, outcome}` with `outcome` in `accepted`, `no_schema`, `refused`,
`schema_unusable`, `reported`, `deadline` (no permit within the wait, or the backstop) and `interrupted` (a panic of
the validator, a defect; D043-180), counted once per start: a check ahead of a start (a schedule's action before its decision or at its release, a
`CANCEL_SOURCE` reset's request) counts only a refusal, final or transient, and the start counts every outcome
(D043-147, D043-173, D043-184); `run_input_schema_cache_total{result}` with `hit`, `miss`, `evicted`,
`failure_remembered`, `unshared`; `run_input_prechecks_total{outcome}` with `admitted`, `unusable`, `busy` and `interrupted`,
for a pre-check outside a start (D043-177, D043-186); `trigger_events_total{outcome}` gains `input_invalid` and `input_schema_unusable`
(D043-129); `scheduler_continuation_input_refusals_total` and `scheduler_continuation_waits_total{blocker="input_check"}`
(D043-133). A `run_input.check` span with the attribute `sagat.run_input.outcome` and, on a refusal,
`sagat.run_input.rule`.
Logs at `debug` name the version id, the creator and the rule; a panic of the validator is logged at `error` with the
schema's digest. None carries a value, a field, a tenant or a namespace label (constitution V and VI). What to alert on
(a rise of `refused` or `reported` after the upgrade, `deadline` that does not fall, any `interrupted`) is in the
runbook.

### 8. Publish and the inventory

`validated_document_in` (`crates/infra-ir/src/lib.rs`) runs the pre-check of Technical Plan 4 on every
`inputSchema`, annotated or not. The codec takes the mode as a parameter (D043-073): the control plane passes the mode of
its process (`SAGAT_RUN_INPUT_VALIDATION`), and `sagat validate` takes `--input-validation enforce|report`, `enforce` by
default. In `enforce` mode a violation refuses the publication with an `IrIssue` that names the
keyword and its pointer (`/inputSchema/...`), as a closed issue `INPUT_SCHEMA_KEYWORD_UNSUPPORTED`,
`INPUT_SCHEMA_REFERENCE_INVALID`, `INPUT_SCHEMA_PATTERN_UNSUPPORTED`, `INPUT_SCHEMA_TOO_LARGE`, `INPUT_SCHEMA_TOO_DEEP`,
`INPUT_SCHEMA_TOO_COSTLY` or `INPUT_SCHEMA_NAME_TOO_LONG` (D043-065, D043-102); in `report` mode it is a
`DefinitionWarning` with the same fields and the version is published (D043-051). An `inputSchema` over 64 KiB or
nested deeper than 32 never reaches the pre-check at publication: feature 016's classification bound reads every
carrier first and answers `CLASSIFICATION_TOO_COMPLEX`, in both modes; the start gate and the inventory answer the
validator's own codes (D043-112, G043-020). `sagat validate` runs the same check locally. A pre-check outside a start
(publication, a dry run, an ad hoc document) runs off the async workers on a permit of the starts' pool (Technical
Plan 3); with none within the wait, or when the validator panics, the document is not judged and the caller gets a
retryable `503 UNAVAILABLE` with no `details.reason`, as a start's `Busy` (`IrError::ValidatorBusy`; the diagnostic
`VALIDATOR_BUSY` of `sagat validate`); the dry run, the replay of a publication and a promotion answer it the same way
(D043-166, D043-177, D043-183, D043-186). The stored-document path (`validated_stored_document_in`) does not run it, so a version already stored still parses and is refused at start
instead (FR-043-011). A `DefinitionWarning` is also added when a version has an `inputSchema` and a non-manual trigger.
The versioning diff already reports a tighter `inputSchema` as breaking (`crates/infra-ir/src/diff.rs`).

**The inventory** (D043-052) is a command of the control plane, `control-plane input-schema-inventory`, beside
`migrate` and `schedule-trigger-handover` (`apps/control-plane/src/cli.rs`). It runs with the database settings of the
control plane process (`config.database`, the runtime role where a deployment sets one, D043-163): it only reads, and
needs nothing but `SELECT` on `workflow_versions`, `workflows` and `runs`, which a database of 0.2.x already has, and on
`schedules` and `schedule_revisions` where the database has them (feature 018's tables, which the v0.2.0 release does not
have: the inventory looks them up once and reads them only when they exist, D043-187). The runtime role holds all five,
so the 0.3.0 binary runs it before the migration. It reads the stored versions page by page (its
cursor starts after the nil uuid, in one statement each page), runs the same pre-check on each stored `inputSchema` and prints one line per version that
would be refused: tenant, namespace, workflow and version ids, the keyword and its pointer. The keyword and the pointer
are author text, so they are printed in double quotes with `"` and `\` escaped (D043-111). In a second, informational
section it lists the versions within the subset that have an `inputSchema` and a non-manual trigger, whose triggered runs may be refused (G043-004); they do not change the exit status. A version outside the subset
that nothing starts any more, neither its workflow's active version, nor pinned by a schedule that is not deleted, nor
the version of an open run or of a run created in the last 30 days (`SUPERSEDED_QUIET_DAYS`), is superseded: the last
line, the summary, gives their number, `--all` lists them, and they never change the exit status (OD-2026-11-16-5,
D043-179, D043-185). Each refused version costs one statement over the runs of its workflow, bounded by the control
plane's statement timeout; a pre-check the inventory's own validator cannot run makes the command fail, exit 2. It prints no
schema text beyond those keywords and pointers, no input, and writes nothing. It exits 0 when no version would be
refused, 1 when at least one would, and 2 when the command itself failed (it could not connect, a read failed, or a
stored document is not a JSON object, which it still reports beside what it could read), so an upgrade or CI script
can tell "found" from "failed" (D043-072, D043-111). The release notes and the
runbook tell the operator to run it before the upgrade.

### 9. Contracts

- `specs/001-platform-foundation/contracts/openapi.yaml`: the action outcome enum gains `INPUT_INVALID`; the shared
  `Error` schema carries design 043: the list of `details.reason` refinements gains `RUN_INPUT_INVALID` and
  `RUN_INPUT_SCHEMA_UNUSABLE` (`VALIDATION_FAILED`, 422, `retryable: false`), `details.rule` is a closed enum that a later
  subset may extend (a client passes an unknown value through, as `sdks/SPEC.md` says), and `details.field` is described
  as a pointer that never holds a caller's key (D043-135). The `422` of
  the six start operations (`startRun`, `signalWithStart`, `updateWithStart`, `operateRun` for the rerun, `resetRun`,
  `startDatabaseExecution`) names the two reasons, and the pinned baselines of features 016 and 035 were re-pinned once
  after reading their diff (OD-2026-11-15-3, D043-149); the schedule writes, trigger-now and the publication are
  described by the shared `Error` schema and `errors.md`, since their operations stay pinned. The `Run` schema gains the
  optional `failureCode`, read from the run's stop (FR-043-015, D043-162); the pinned baselines digest operations only,
  so it moves none of them.
- `specs/043-run-input-validation/contracts/input-schema-subset.md`: the subset, its pattern allowlist, its limits and
  its charges.
- `specs/043-run-input-validation/contracts/errors.md` with the reasons, the failure codes, the publication issue
  codes, the `503` of a document whose pre-check found no permit, and the remediation lines. It notes that `INPUT_INVALID` has two meanings: the schedule action outcome of design
  043, and the worker step failure code of `sdks/SPEC.md`, which is older and unrelated. The `SCHEDULE_INPUT_INVALID` row
  of `specs/018-schedules-bulk-operations/contracts/errors.md` points to it and gains `RUN_INPUT_SCHEMA_UNUSABLE` at
  action time.
- `specs/016-payload-classification/contracts/README.md` (section 1.3, rule 6) says that an `inputSchema` carrier is
  also held to design 043's subset: `prefixItems` and the other applicators that rule lets appear in a carrier make an
  `inputSchema` unusable for starts and are refused at publication in `enforce`; output and state carriers are
  unchanged (O043-005).
- AsyncAPI is unchanged; `events.baseline.json` is its test.
- `sdks/SPEC.md` and the six SDK error types gain `field` and `rule`, and the six SDK `Run` types `failureCode`;
  `sdks/error-catalog.json` keeps its codes.

### 10. Rollout

Before the upgrade the operator runs `control-plane input-schema-inventory` and, if callers or schemas cannot be fixed
before the upgrade, sets `report` everywhere
(Technical Plan 6). Upgrade order (OD-2026-11-14-3): the migration first; then the engines (the batch items of row 4,
the pending resets of row 5, rows 6, 7 and 9 to 12 are checked from then on); then the control planes (the requests of
rows 1 to 5, rows 8, 13 and 14); then, once every live process runs 0.3.0 in `enforce`, the requirement
`run-input-validation-v1`. Until it is recorded, an old control plane admits what a new one refuses, and replicas may
run different modes (G043-010). Runs in flight are checked when they create a child or continue (OD-2026-11-14-9).
Rollback: remove the requirement, then downgrade the binaries; a 0.2.x binary runs on the 0.3.0 schema (the new CHECKs
are supersets), so the binaries can roll back without the down migrations. The down migration of `20270118000000` refuses while a schedule action holds
`INPUT_INVALID`, while a registry row lists `run-input-validation-v1`, or while the requirement history names it
(D043-131; `the_run_input_validation_rolls_back_only_while_nothing_names_it`).

What a 0.2.x process reads of what 0.3.0 writes, beside it during the engine roll or after a rollback:

| Written by 0.3.0 | Read by 0.2.x | What it does |
|---|---|---|
| A schedule action `INPUT_INVALID` | the action listing; the "already acted" check of a time; the decision path | lists the outcome as a string; a filter on it is refused `422` (G043-001); a time with any action is acted on, so a 0.2.x engine does not decide it again |
| A delivery to `trigger:<id>` with no run | the dispatcher's "already delivered" check by consumer; the event read | finds the delivery and starts nothing; reads `run_id` as optional |
| The stop codes `CHILD_INPUT_INVALID`, `CONTINUATION_INPUT_INVALID`, `RUN_INPUT_SCHEMA_UNUSABLE` | every reader of a run's or a step's failure code | a free-form code (`[A-Za-z0-9_.:-]`), passed through |
| The capability `run-input-validation-v1` and its requirement | the registry's check of a process's capabilities | an unknown required capability is lacking: the 0.2.x process keeps its gate closed (ADR-0082) |

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Triggers on versions whose schema describes a manual input stop starting runs (G043-004) | A delivery and an audit row per refused event, the metric, the publish warning, the release notes, `report` mode while callers are fixed |
| A run started before the upgrade fails at a child or a continuation (OD-2026-11-14-9) | Stated in the release notes with the command that lists the versions at risk; the failure code names the cause; accepted by the owner as a deviation from principle IV |
| A schema the platform did not write costs CPU or memory | The subset and its pre-check, the weighed cache whose tenant share holds the heaviest admitted schema, the validator's own work counter, the input depth and nesting limits, four permits, a 5 s backstop (NFR-043-BOUND, D043-159) |
| An `inputSchema` with tuple positions (`prefixItems`), which features 016 and 035 read, is refused in `enforce`: such a version cannot start, and once the requirement is recorded `report` is gone (O043-005) | Stated in 016's contract and the release notes; listed by the inventory; the author rewrites the schema without tuple positions; the owner decides whether tuples join the subset |
| A composition root builds a creator without the gate, which then refuses every start of a version with a schema as busy | The seventh architecture rule fails on it (ADR-0092 decision 2); the unwired gate refuses, it never admits unchecked (D043-120) |
| A defect of the validator panics on a schema or an input | Caught and answered `Interrupted`, transient, for the caller and those who waited; logged at `error` with the digest; nothing remembered (D043-106, D043-166). A schema whose compile panics is tried again on every start until the validator is fixed (G043-053) |
| Published schemas that use a keyword outside the subset (recursion, `patternProperties`, `if`, `not`, ...) become unusable (OD-2026-11-14-14) | `control-plane input-schema-inventory` lists them before the upgrade (FR-043-018); `report` mode while they are republished (G043-014) |
| The platform's validator and the drafts disagree on a case | The subset excludes the known disagreements (fractional `multipleOf`, `\d`-style classes, `$ref` siblings, tuple `items`); the oracle property test against `jsonschema` (T043-2) is a regression check, not a proof (G043-015, G043-016) |
| One tenant's burst occupies the four permits | A free permit goes to the waiting tenant that holds the fewest; a tenant may use all four while no other waits, and a waiting tenant is served first (D043-068); each validation is bounded by the counter, so a permit is held for a bounded time; a caller who cannot get one in 5 s gets a retryable `503`, never a final refusal (D043-050, D043-068); the pre-checks outside a start share the pool as one owner of the queue (D043-177) |
| The first upgraded engine records final outcomes before the operator chose `report` | The runbook sets `report` in every manifest before migrating (D043-040). The inventory finds only schemas outside the subset, never the inputs a schema in the subset refuses, so whether to choose `report` rests on what the operator knows of its callers; `report` counts and audits those refusals once it is on |
| A tenant alone on a process starts more than 64 schemas often | Each recompile runs under a permit and is bounded like any compile; the limit is documented and kept by the owner (OD-2026-11-16-6); T043-28 measures it |
| A pointer or a rule name leaks what the caller sent | Segments only where `properties` or `items` matched; the rule named by the validator's own keyword; no input value ever formatted; `/` for repeats; the sentinel test with sentinel keys |
| A refusal loops | Rows 6 to 12 record a final outcome; the trigger delivery stops the re-audit; the schedule arm sits before `NotAllowed` and `Err(other)` |
| Replicas answer differently | The capability is listed only in `enforce`; the operator's requirement closes the gate of any other process (G043-010 until it is recorded) |
| `report` mode becomes permanent | Logged at every process start, counted, and the release notes ask the operator to record the requirement |
| Existing tests and fixtures start runs whose input does not match their schema | T043-20 sweeps them first and lists each change; a fixture is fixed, never the check loosened |
| An old control plane meets an `INPUT_INVALID` action during the upgrade (G043-001) | The listing passes unknown outcomes through by design; the strict outcome filter is in the release notes; T043-11 tests the listing |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | The check runs before the creation transaction and writes nothing; a schedule refusal, its pause, its audit and the backfill's end are one decision under the schedule's fence; a trigger refusal is one transaction (delivery, then audit); a continuation refusal ends the predecessor through the existing fenced path |
| II. Modular domain, ports and adapters | Pass with conditions (ADR-0092, accepted OD-2026-11-16-4; Complexity Tracking) | The domain holds opaque bytes and a digest computed by `infra-ir`; the port and the gate are in `application`, and the gate names no runtime; the permits, the wait, `block_in_place` and the backstop are in the adapter `infra-schema` behind the port, which has tokio as a normal dependency (D043-071). `application` has tokio `rt` alone for design 042's blocking parse in one file, a divergence from ADR-0090 recorded in ADR-0092 (accepted) and held to that file by a rule; no new edge into `application` or the domain (`scripts/check-architecture.py`), and a rule keeps `jsonschema` out of the validator module (D043-074) |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect is added; a refused run has none |
| IV. Compatibility is versioned and pinned | Pass with conditions | Published versions are not changed. Three deviations, each accepted by the owner: an input that was accepted is refused (OD-2026-11-14-2); runs in progress can fail at the step that creates a child or continues (OD-2026-11-14-9); a document that publishes today can be refused by 0.3.0 publication, because the subset narrows `inputSchema` (OD-2026-11-14-14). All three in Complexity Tracking |
| V. Code, tenant boundary and secrets are isolated | Pass | No tenant code runs; a schema is data validated by the platform's own bounded validator; no remote reference is resolved; a compiled schema is shared only within its tenant; bounded compile and validation off the async workers; no grant (NFR-043-SEC); values never in errors or telemetry (FR-043-014) |
| VI. Observability and audit are part of the feature | Pass | Metrics with closed labels, span attributes; an audit row for every trigger refusal and every reported start; a schedule's refusal is its action record, with the auto-pause audited (Technical Plan 7) |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | A trigger refusal is a delivery record: a redelivery finds it and repeats nothing (D043-027) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Per-row tests on real PostgreSQL for both refusals, the crash and duplicate cases of the Verification Plan, six-SDK conformance; gaps registered |
| Delivery Constraints | Pass | No new technology; single region; the upgrade order of OD-2026-11-14-3; the registry of ADR-0082 |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A refusal where an input used to be accepted (principle IV) | The owner's decision OD-2026-11-14-2; published versions stay as they are, only the admission of new runs changes | Deferring validation, as first recommended: the owner refused it |
| A document that publishes today can be refused tomorrow (principle IV: contracts versioned, a change MUST NOT break) | OD-2026-11-14-14: the subset narrows what `inputSchema` may contain, so a schema accepted by 0.2.x publication is refused by 0.3.0 publication in `enforce`, and a stored one refuses its starts. Recorded in `workflow.schema.json` and `sdks/SPEC.md` (T043-14); listed by the inventory | Accepting any JSON Schema with the library: rejected after four reviews (OD-2026-11-14-14) |
| Children and continuations of runs in progress are checked (principle IV: a change MUST NOT break runs in progress) | OD-2026-11-14-9, against the recommendation: a child or a successor is a new run, and the owner wants every new run checked | Exempting runs created before the upgrade (the recommendation): the owner refused it. The release notes say an old run can fail there |
| `CheckedRunInput`, a `test-support` feature and seven architecture rules | Thirteen creating paths in three crates; the next one must not skip the check, the test constructor must not reach a build, and a composition root must not leave a creator unwired | A code review rule: G1955 is an unchecked path that review let through |
| `tokio` `rt` in `sagat-application`, for one file (ADR-0092) | Design 042, merged first, parses a blob source's array in a blocking task there | Moving that parse behind a port of an adapter (G043-052): the follow-up of design 042, not done in the middle of a release |
| A narrow port and a gate in `application` | The policy (mode, metrics, the type) must not live in the adapter; the adapter sees only the domain identifiers it needs (`TenantId` in `SchemaKey`) and never builds `CheckedRunInput` | One trait that does everything: the adapter would own the mode and build the type |
| The schema bytes and digest in `WorkflowDefinition` | Continue-as-new holds the definition, not the document; no statement may be added | Reading the document on a cache miss: an extra statement on the continuation path |
| A second compiled-validator cache keyed by tenant and digest | Compile once per schema; never under a lock; never shared across tenants | Compiling per start, as the blob check does today; keying by version, which ad hoc floods |
| The platform's validator for a documented subset, with a work counter (OD-2026-11-14-14) | The bound on the work holds by construction: the counter counts what the validator does and stops at a budget. Four review rounds found work that `jsonschema` does and a static walk did not charge (`unevaluated*`, percent-encoded pointers, value-sized keywords) | `jsonschema` with a static walk as the bound: rejected after four reviews; validation in an isolated process with a timeout: rejected for latency and operations |
| A blocking section with four permits shared work-conservingly among tenants, a 5 s wait and a 5 s backstop | Validation is CPU work on schemas and inputs the platform did not write; the counter bounds each run, and the fair queue keeps one tenant from holding all four while another waits | A plain semaphore with a timed acquire, relying on the existing rate limits and pass fairness: one tenant's burst would hold the four permits while another tenant's starts time out, and the rate limits are per namespace, not per permit; a memory of refused inputs per process and two clocks, which the third draft had (removed, D043-050); refusing a tenant at its cap at once (D043-053, which broke a single-tenant deployment), and a fixed cap of two per tenant (D043-061, which left permits idle); replaced by D043-068 |
| A schedule action outcome and a migration | A refused action must be visible and must not loop | Reusing `ADMISSION_RETRY` (`NotAllowed`): retries forever |
| A capability and a strict variable | Replicas must not answer differently once the operator decides | A per-process flag alone: nothing stops one replica in `report` beside others in `enforce` |
| Three failure codes (`CHILD_INPUT_INVALID`, `CONTINUATION_INPUT_INVALID`, `RUN_INPUT_SCHEMA_UNUSABLE`) | The parent and the chain must say why, without the value | `EXPRESSION_ERROR`: says the expression failed, which it did not |

## Alternatives Considered

- **`jsonschema` with a static walk as the bound on its work.** Four review rounds found cases the walk did not
  charge or resolved differently (`unevaluated*` re-running branches, percent-encoded pointers, value-sized keywords,
  recursion through `dependencies`). Rejected for the subset with the platform's validator (OD-2026-11-14-14).
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
- **A check written in each creator, no shared type.** What existed for the blob start, the one path that checked while the others did not. Rejected.
- **Validation in the SDKs only.** Six implementations of a dialect, and the engine's creators have no SDK in their
  path. Rejected.
- **Cache keyed by version id.** Simple, but an ad hoc start makes a version per call (`ad_hoc.rs`) and would fill
  the cache, and identical schemas of successive versions would compile again. Rejected for `(tenant, digest)`.
- **`report` by default in 0.3.0.** Contradicts OD-2026-11-14-2. Rejected.
- **A per-namespace mode.** Six SDK surfaces, the Studio and an audit for a transition aid. Not chosen (OD-2026-11-14-8).
- **Defer, as recommended before OD-2026-11-14-2.** The owner chose otherwise.

## Non-Functional Requirements

- **NFR-043-PERF**, **NFR-043-BOUND**, **NFR-043-SEC** and **NFR-043-COMPAT** as stated under Requirements. Their
  tests: SC-043-003 and SC-043-004 for the first two, with the heaviest-schema and permit tests of T043-2 and T043-3;
  for NFR-043-SEC, the privilege audit of `integration_platform_foundation` (no grant added), the cache test that never
  shares across tenants, and the suites run as the runtime role; for NFR-043-COMPAT, the listing of an unknown outcome
  (G043-001), the registry test of T043-12, and the 0.2.x table of Technical Plan 10.
- The user stories: US01 is CUJ 1 (FR-043-003, FR-043-015); US02 is CUJs 2 to 4 (FR-043-006 to FR-043-009); US03 is
  CUJ 5 (FR-043-013, FR-043-018).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

The owner asked for the tasks to be built before the design is approved (OD-2026-11-14-15), so each task below says
what was built, where, and which tests hold it; each is ticked at approval with its evidence. They were built in three
reviewed phases: phase 1, T043-1, T043-2, T043-13 and T043-24 (branch `feature/043-validator`); phase 2, T043-3 to
T043-12 and T043-20 to T043-23; phase 3, T043-14 to T043-19 and the fixes of the review runs (branch
`feature/043-creators`). A task that changed behaviour had its tests seen failing first; `research.md` names the red
runs. The order that kept the build green: T043-1 and T043-2; T043-13 and T043-24, which need only the pre-check; T043-3
and T043-4 together with the gate wired at every creator (a `CheckedRunInput` can only come from the gate); T043-20
before any creator refused; T043-11's migrations before T043-12 lists the capability; then the rest.

- [x] T043-1 Carried `InputSchema` (the canonical bytes, an `Arc<[u8]>`, and their SHA-256) in the parsed
  `WorkflowDefinition`, counted by its footprint, on `parse`, `parse_stored` and their private twins.
  Files: `crates/domain/src/workflow.rs`, `crates/infra-ir/src/lib.rs`, `crates/infra-ir/src/continuation.rs`.
  Tests: `crates/infra-ir/tests/cache.rs` (`the_parsed_definition_carries_the_canonical_input_schema_and_its_sha256`,
  `a_continuation_definition_carries_its_input_schema`, `the_input_schema_is_counted_by_the_definition_footprint`).
  Refs: FR-043-001, NFR-043-PERF
  Owner-gate: OD-2026-11-14-2
  Evidence (2026-11-16): the unit tests of `sagat-infra-ir` at `f48d5729` (in the 2,338 passed, 0 failed of
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval"), among them the three
    tests named above.
- [x] T043-2 Built `InputSchemas`, the platform's validator of the subset of
  `specs/043-run-input-validation/contracts/input-schema-subset.md`: the pre-check and its `Unusable { issue, keyword,
  at }`, the compiled arena, the work counter, the refusal with its closed `Rule` and pointer, and the `(tenant, digest)`
  cache with its tenant share; patterns parsed by `regex-syntax` 0.8 and compiled by `regex-automata` 0.4 (`meta`, with
  the backtracker and the one-pass DFA off and 16 KiB lazy DFA caches); a pre-check outside a start runs off the async
  workers on a permit of the starts' pool (D043-177); a panic of the compile or the validation is answered
  `Interrupted`, and a pre-check's panic is caught the same way (D043-186); `run_input_prechecks_total`; a pin test
  holds the limits and charges of `run-input-validation-v1` (D043-178, D043-186).
  Files: `crates/infra-schema/src/run_input/` (`mod.rs`, `vocabulary.rs`, `compile.rs`, `validate.rs`, `counter.rs`,
  `pattern.rs`, `pointer.rs`, `value.rs`), `crates/infra-schema/src/lib.rs`, `crates/infra-schema/src/classification/cache.rs`
  (generic over key, value and failure, with a per-owner share; the classification behaviour unchanged),
  `crates/infra-schema/Cargo.toml` (`regex-syntax`, `regex-automata`, `tracing`, a `test-support` feature, `proptest`
  for the tests; `jsonschema` stays for the scripts and as the oracle).
  Tests: `crates/infra-schema/tests/run_input.rs`: `every_keyword_outside_the_subset_is_refused_with_its_name_and_pointer`,
  `the_subset_rules_that_keep_the_drafts_equal_are_enforced`, `format_and_the_annotations_are_inert`,
  `a_pattern_past_its_states_its_program_or_the_count_is_refused`, `a_reference_must_name_a_schema_position_of_the_document`,
  `a_cycle_is_found_by_the_marked_search_through_all_of_and_through_properties`,
  `a_reference_dag_compiles_to_an_arena_the_size_of_the_document`, `size_and_depth_are_bounded_before_anything_else`,
  `a_schema_every_accepted_input_of_which_passes_the_budget_is_refused_at_publication`,
  `one_of_counts_its_cheapest_branch_at_publication`,
  `the_refusal_names_the_rule_and_a_pointer_of_declared_names_and_indices_only`,
  `a_deep_data_value_is_named_by_its_keyword_never_by_its_keys_and_pointers_are_truncated`,
  `the_backstop_stops_a_validation_past_its_time_transiently`, `two_validators_from_the_same_bytes_report_the_same_field_and_rule`,
  `the_heaviest_schema_the_precheck_admits_fits_a_tenant_share_and_is_kept`,
  `a_schema_whose_pointers_would_outgrow_the_bound_is_refused_as_too_large`,
  `every_adversarial_case_stops_at_the_budget_with_operations_bounded_by_the_units`,
  `one_of_a_cheap_match_and_a_costly_match_is_a_work_limit_never_an_acceptance`,
  `measuring_an_input_for_const_or_enum_stops_at_the_cap_instead_of_walking_it`,
  `multiple_of_on_a_huge_float_is_exact_and_bounded_by_the_units`, `a_pattern_at_the_state_cap_runs_its_charge_within_the_backstop`,
  `the_compiled_schema_is_shared_by_arc_for_the_gate`, `the_weight_counts_literals_and_a_tenant_holds_only_its_share_of_the_cache`,
  `the_limits_and_the_charges_of_run_input_validation_v1_are_pinned`, the measurement `measure_k_and_the_cost_of_a_full_budget`
  (ignored, run by hand, G043-017), the oracle `oracle::the_platform_validator_agrees_with_jsonschema_over_the_subset` and
  the property test `sentinel::no_generated_value_or_key_appears_in_a_refusal`; the unit tests of `run_input/mod.rs`
  (`the_caller_whose_compile_panics_is_answered_interrupted`, `a_compile_that_panics_is_transient_for_those_who_waited`,
  `prechecks_outside_a_start_run_at_most_four_at_once`, `a_precheck_waits_in_the_starts_pool_and_is_busy_after_its_wait`,
  `a_precheck_whose_compile_panics_is_interrupted`)
  and of `classification/cache.rs`; `the_cache_compiles_once_per_tenant_and_schema_and_remembers_a_failure_once`
  (`integration_run_input_cache`, `tests/integration/run-input-cache/`, a process of its own).
  Refs: FR-043-002, FR-043-011, FR-043-014, NFR-043-BOUND, NFR-043-SEC, SC-043-004
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-11, OD-2026-11-14-14
  Evidence (2026-11-16): the unit tests of `sagat-infra-schema` at `f48d5729` (in the 2,338 passed) with every test named
    above; `integration_run_input_cache` 1 passed in both roles at `4b6eb1a6`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-3 Built the gate and its type in `application`: the `SchemaValidator` port and its types (`SchemaKey`,
  `SchemaRefusal`, `Rule`, `PathSegment`, `Unusable`, re-exported by the adapter), `RunInputGate` with `unwired()`,
  `check`, `check_schema` and `recheck`, `CheckedRunInput` (the version id; fields and constructor private to its
  module; a `test-support` constructor and a `test-support` validator double, `Scripted`), the field policy, the metrics
  and the span (a panic counted `interrupted`, D043-180; a check ahead of a start counting only what stops it,
  D043-184), and the mapping of the transient refusals to `Unavailable`;
  and in the adapter (D043-071) the permits: four per process, the work-conserving fair queue of D043-068, the 5 s wait,
  the guards of a dropped future, the blocking acquire of a pre-check (D043-177), `off_worker` and the backstop.
  `RunInputChecker` (the blob-only check) was removed by T043-5, which replaces it.
  Files: `crates/application/src/run_input.rs`, `crates/application/src/lib.rs`, `crates/application/Cargo.toml` (the
  `test-support` feature; `tokio` stays a normal dependency with `rt` alone, for design 042's `collection_source.rs`,
  ADR-0092 decision 1), `crates/infra-schema/src/run_input/permits.rs`, `crates/infra-schema/src/run_input/mod.rs`,
  `crates/infra-schema/Cargo.toml` (tokio `sync`, `time`, `rt-multi-thread`).
  Tests: the unit tests of `crates/application/src/run_input.rs`
  (`an_invalid_input_names_the_pointer_and_the_rule_and_never_the_value`, `a_repeat_whose_source_is_withheld_is_told_the_root_and_other`,
  `the_budget_and_the_depth_are_final_at_the_root_and_the_rest_transient`,
  `report_mode_admits_a_final_refusal_and_audits_it_but_never_a_transient_one`, `an_ad_hoc_key_is_never_shared_and_every_other_is`,
  `an_unwired_gate_refuses_a_schema_transiently_and_passes_a_version_without_one`,
  `a_version_without_a_schema_is_not_sent_to_the_validator`, `the_debug_of_a_checked_input_shows_no_byte_of_it`,
  `a_refusal_maps_to_its_closed_reason`, `a_refusing_recheck_is_counted`, `a_reporting_recheck_is_counted`,
  `a_report_is_counted_once_per_start_whatever_checks_it_before`, `a_panic_of_the_validator_is_counted_interrupted_and_no_room_deadline`),
  `a_buffered_action_released_and_started_is_counted_once` (`integration_schedules_bulk_operations`), and the unit tests
  of `crates/infra-schema/src/run_input/permits.rs`
  (`a_tenant_alone_holds_every_permit_and_no_more`, `a_newcomer_is_served_before_the_holders_next_waiter`,
  `a_dropped_waiter_leaves_the_queue_and_a_dropped_grant_returns_its_permit`,
  `a_release_in_the_gap_of_a_timed_out_waiter_is_not_counted_twice`, `no_permit_within_the_wait_is_busy`).
  Refs: FR-043-001, FR-043-013, FR-043-014, NFR-043-PERF, NFR-043-BOUND
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-8, OD-2026-11-14-14
  Evidence (2026-11-16): the unit tests of `sagat-application` and `sagat-infra-schema` at `f48d5729` (in the 2,338
    passed); `a_buffered_action_released_and_started_is_counted_once` passed in both roles of
    `integration_schedules_bulk_operations` (757 passed) at `4b6eb1a6`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-4 Made `NewRun.input`, `ContinuationSelection` and `EvaluatedChainHandoffCommand.input` a `CheckedRunInput`,
  so that every creator that builds one goes through the gate. `PgRunStore::create` and `continue_evaluated` refuse an
  input checked against another version (FR-043-017). The seven rules of FR-043-016 were added. Every composition root,
  and the test harness, gives each creator it builds the gate (`.with_run_inputs` on `RunService`, `Scheduler`,
  `EventDispatcher` and `ScheduleService`), one gate per process over `RunInputValidator::process()`.
  Files: `crates/application/src/runs.rs`, `crates/application/src/runs/ad_hoc.rs`, `crates/application/src/chain_progress.rs`,
  `crates/application/src/schedules.rs`, `crates/scheduler/src/events.rs`, `crates/scheduler/src/passes.rs`,
  `crates/infra-postgres/src/execution.rs`, `crates/infra-postgres/src/chain_progress.rs`, `scripts/check-architecture.py`,
  `scripts/tests/test_check_architecture.py`, `apps/control-plane/src/lib.rs` (the run service of the API, and the
  schedule service with its trigger-now run service), `apps/execution-engine/src/lib.rs` (the scheduler, the event
  dispatcher, and the run services of the schedule firing, the batch executor and the reset completer), the test
  harness (`tests/support/api.rs`, `tests/support/engine.rs`, `tests/support/schedules.rs`, `tests/support/run_input.rs`),
  and the `[dev-dependencies]` of the `Cargo.toml`s that use `test-support`.
  Tests: `the_store_refuses_an_input_checked_against_another_version` (`integration_durable_execution`),
  `prepared_continuation_commits_replays_and_rolls_back_atomically` (`integration_workflow_primitives`, its FR-043-017
  case), and the fixtures of `scripts/tests/test_check_architecture.py`.
  Refs: FR-043-016, FR-043-017
  Owner-gate: OD-2026-11-14-2
  Evidence (2026-11-16): `integration_durable_execution` 192 passed and `integration_workflow_primitives` 805 passed, both
    roles, at `4b6eb1a6`; `scripts/check-architecture.py` passed and its fixtures ran in `python3 -m unittest discover
    -s scripts/tests` (OK) at `f48d5729`; `specs/043-run-input-validation/validation-report.md`, section "the gates of
    the approval".
- [x] T043-5 Checked rows 1 to 5 in `start_with`: the error variant and its mapping, the lookup of the key before any
  refusal, the `409` after a promotion, the refusal with an open holder, `FieldPolicy::Root` for every repeat (D043-121),
  the uploaded-input start against its descriptor (replacing `RunInputChecker`), the check of a `CANCEL_SOURCE` reset at
  its request, and the batch and pending-reset outcomes.
  Files: `crates/application/src/runs.rs`, `crates/application/src/runs/reset.rs`, `crates/application/src/runs/pending_reset.rs`,
  `crates/application/src/blobs.rs`, `crates/application/src/error.rs`, `crates/api-http/src/error.rs`,
  `crates/application/src/batch_executor.rs`, `crates/application/src/reset_completion.rs`.
  Tests: `integration_durable_execution` (`tests/integration/durable-execution/run_input.rs`):
  `row_1_a_refused_start_names_the_field_and_the_rule_and_creates_nothing`,
  `row_1_an_unusable_schema_refuses_every_start_and_no_schema_takes_any_object`,
  `a_refused_start_replays_the_run_of_its_key_and_a_promotion_makes_it_another_request`,
  `a_retry_with_no_room_replays_the_run_its_key_holds`; `integration_workflow_primitives`:
  `row_2_an_uploaded_input_start_checks_the_descriptor_and_a_refusal_claims_nothing`; `integration_schedules_bulk_operations`
  (`run_input.rs`): `row_3_with_start_is_refused_for_its_input_even_with_an_open_holder`,
  `a_signal_with_start_retry_the_gate_refuses_replays_what_the_first_request_delivered`,
  `row_4_a_rerun_of_an_input_the_version_refuses_says_the_root_and_other`,
  `an_unusable_schema_refuses_with_start_rerun_and_trigger_now`,
  `a_pending_reset_the_gate_refuses_after_another_engine_completed_it_is_already_completed`; and (`run_input_repeats.rs`)
  `row_4_a_batch_rerun_item_refused_for_its_input_fails_with_validation_failed`,
  `row_5_a_synchronous_reset_of_an_input_the_version_refuses_says_the_root_and_other`,
  `row_5_a_reset_of_a_version_whose_schema_is_unusable_is_refused_schema_unusable`,
  `row_5_a_cancelling_reset_whose_input_the_version_refuses_is_refused_and_cancels_nothing`,
  `row_5_a_pending_reset_whose_input_the_version_refuses_ends_admission_refused`; `crates/api-http/tests/run_input_errors.rs`
  (the problem shape: `a_refused_run_input_is_a_validation_failure_with_its_field_and_its_rule`,
  `an_unusable_schema_has_no_field_and_no_rule_and_no_room_is_retryable`, `every_rule_is_answered_by_its_closed_name`).
  Refs: FR-043-001, FR-043-003, FR-043-004, FR-043-005, FR-043-010, FR-043-011
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-10, OD-2026-11-14-11
  Evidence (2026-11-16): `integration_durable_execution` 192, `integration_workflow_primitives` 805 and
    `integration_schedules_bulk_operations` 757 passed in both roles at `4b6eb1a6`; the `sagat-api-http` unit tests at
    `f48d5729`; `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-6 Checked row 13 (the ad hoc execution, the connection probe and the private rerun): the caller's document
  parsed in the process's mode, its own replay, a key never shared, the same permits, `/` and `other` for a private rerun.
  Files: `crates/application/src/runs/ad_hoc.rs`, `crates/application/src/execution.rs` (`parse_document_in`).
  Tests: `tests/integration/database-connections/run_input.rs` (refused with the field and the rule, the replay, the
  private rerun, a schema outside the subset in `enforce` and in `report`, the sentinel of T043-17): the function
  `run_input::run`, one scenario of the target `integration_database_connections`, whose single test,
  `durable_provider_intents_reconcile_without_plaintext_or_duplicate_rotation`, runs its scenarios in turn on one
  database and one connection.
  Refs: FR-043-001, FR-043-003, FR-043-004, FR-043-011
  Owner-gate: OD-2026-11-14-2
  Evidence (2026-11-16): `integration_database_connections` 7 passed in both roles at `4b6eb1a6`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-7 Checked rows 9 and 10: the delivery without a run and its audit in one transaction, the cron tick's audit
  with the compare-and-set move of its trigger, no audit for a tick whose key holds a run, the metric, `Started::No`.
  Files: `crates/scheduler/src/events.rs`, `crates/application/src/dispatch.rs` (`DeliveryRef::refused_at_trigger`, the
  ports `record_refused_delivery` and `refuse_cron_tick`), `crates/infra-postgres/src/dispatch.rs` (their writes),
  `crates/domain/src/audit.rs` (`RunInputRefused`).
  Tests: `integration_events_webhooks_messaging`: `row_9_a_refused_trigger_records_a_delivery_and_one_audit_and_the_others_are_served`,
  `row_9_a_refused_trigger_does_not_keep_a_wait_for_the_same_event_from_waking`, `row_9_an_unusable_schema_is_recorded_with_its_reason`,
  `a_cron_trigger_whose_input_is_refused_starts_nothing_and_audits_the_tick`,
  `a_cron_trigger_whose_schema_is_unusable_audits_the_tick_with_its_reason`; `fault_events_webhooks_messaging`:
  `two_engines_on_one_refused_cron_tick_audit_it_once_with_the_tick_key`,
  `a_cron_refusal_whose_writer_stops_before_the_trigger_moves_is_audited_once_by_the_next_round`,
  `a_refused_tick_whose_run_a_reporting_peer_created_is_not_audited`,
  `a_crash_between_the_refused_delivery_and_its_audit_keeps_neither_and_the_retry_records_both_once`.
  Refs: FR-043-001, FR-043-007
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-11
  Evidence (2026-11-16): `integration_events_webhooks_messaging` 120 and `fault_events_webhooks_messaging` 12 passed in
    both roles at `4b6eb1a6`; `specs/043-run-input-validation/validation-report.md`, section "the gates of the
    approval".
- [x] T043-8 Checked row 11 with `CHILD_INPUT_INVALID` and `RUN_INPUT_SCHEMA_UNUSABLE`, also for a parent created before
  the check was on; fail the step only if, under the step's row lock, its child was not claimed (D043-164).
  Files: `crates/scheduler/src/passes.rs` (`start_child`, `Condition::ChildNotStarted`, the test hook
  `with_before_child_check`), `crates/application/src/execution.rs` and `crates/infra-postgres/src/execution.rs`
  (`RunTx::child_started`), `crates/domain/src/execution/event.rs` (the codes).
  Tests: `integration_workflow_primitives`: `row_11_a_refused_child_is_not_created_and_its_parent_step_fails_with_the_code`,
  `row_11_the_child_of_a_parent_admitted_before_the_check_is_checked`,
  `a_child_a_peer_created_before_the_check_is_not_failed_by_the_refusal`,
  `a_child_claimed_while_its_refusal_waits_for_the_step_is_not_failed`.
  Refs: FR-043-001, FR-043-008, FR-043-011
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-9, OD-2026-11-14-11
  Evidence (2026-11-16): `integration_workflow_primitives` 805 passed in both roles at `4b6eb1a6`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-9 Checked row 12 with `CONTINUATION_INPUT_INVALID` and `RUN_INPUT_SCHEMA_UNUSABLE`, also for a chain started
  before the check was on: the schema read under the run's lock, checked after it is released, the failure through the
  fenced path; `InputUnavailable` counted as a wait; the `report` audit in the hand-off.
  Files: `crates/scheduler/src/passes.rs`, `crates/infra-postgres/src/chain_progress.rs`, `crates/domain/src/execution/event.rs`.
  Tests: `integration_workflow_primitives`: `row_12_a_refused_continuation_fails_its_run_and_creates_no_successor`;
  `fault_workflow_primitives`: `a_restart_between_the_check_of_a_continuation_and_its_failure_fails_the_run_once`.
  Refs: FR-043-001, FR-043-009, FR-043-011
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-9, OD-2026-11-14-11
  Evidence (2026-11-16): `integration_workflow_primitives` 805 and `fault_workflow_primitives` 93 passed in both roles at
    `4b6eb1a6`; `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-10 Checked row 14: `ScheduleStore::target_version` (the target version's document, one statement; `target_check`
  unchanged) and `ScheduleService::with_run_inputs` (the gate and a definition cache); the input checked with
  `nominalTimeField` filled with the next nominal time; `SCHEDULE_INPUT_INVALID` under `/action/input`;
  `NOMINAL_TIME_FIELD_INVALID` for a field the schema does not declare as a string, read through `$ref` and `allOf`;
  `RUN_INPUT_SCHEMA_UNUSABLE`.
  Files: `crates/application/src/schedules.rs`, `crates/application/src/schedule_firing.rs` (`with_nominal_time`),
  `crates/infra-postgres/src/schedules.rs` (`target_version`), `crates/api-http/src/error.rs`.
  Tests: `integration_schedules_bulk_operations`: `row_14_a_write_whose_input_the_target_refuses_is_refused_under_action_input`,
  `row_14_the_nominal_time_field_must_be_declared_and_an_unusable_schema_refuses`,
  `row_14_the_write_fills_the_next_nominal_time_and_resolves_the_declaration`.
  Refs: FR-043-006, FR-043-011
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-11
  Evidence (2026-11-16): `integration_schedules_bulk_operations` 757 passed in both roles at `4b6eb1a6`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-11 Checked rows 6 to 8 and added the outcome `INPUT_INVALID`:
  - two migrations: `20270118000000_run_input_validation` drops each CHECK of the action outcomes, of the registry's
    capability lists and of its requirement history in the `ALTER TABLE` that adds its `*_known` superset `NOT VALID`,
    and `20270118000100_run_input_validation_validate` validates them; the first's down migration refuses while a row
    holds a new value (the second's is `SELECT 1`: a validated CHECK has nothing to undo);
  - `ScheduledStarts::check_scheduled` before `decide`, and `recheck` at the start;
  - one decision with the action, the auto-pause and its audit, or a backfill's end;
  - trigger-now's `422` before it decides and at its start; `BUFFERED` to `INPUT_INVALID` settled where it waits; an
    exhausted schedule not checked.
  Files: the four migration files under `crates/infra-postgres/migrations/`, `crates/domain/src/schedules.rs`,
  `crates/application/src/schedule_firing.rs`, `crates/application/src/runs.rs` (`check_scheduled`),
  `crates/infra-postgres/src/schedules.rs` (`settle_waiting`), `frontend/apps/studio/src/features/schedules/model.ts`.
  Tests: `integration_schedules_bulk_operations`: `row_6_an_action_the_new_version_refuses_is_input_invalid_and_pauses_once`,
  `row_6_without_pause_the_refused_time_is_not_owed_and_an_unusable_schema_says_so`,
  `row_7_a_backfill_whose_input_is_refused_ends_cancelled`,
  `row_7_a_backfill_of_a_version_whose_schema_is_unusable_ends_cancelled_with_the_reason`,
  `row_8_trigger_now_answers_422_and_records_nothing`, `row_8_trigger_now_answers_422_where_the_decision_would_buffer`,
  `a_trigger_now_refused_at_its_start_answers_422_and_records_nothing`, `a_buffered_action_is_checked_again_at_its_release`,
  `an_exhausted_schedule_records_no_input_invalid_for_a_version_that_refuses_its_input`,
  `one_scheduled_action_is_one_validation_in_two_spans`,
  `actions::the_listing_passes_an_outcome_it_does_not_know_through_as_a_string` (G043-001); the `InputInvalid` cases of
  `firing_decisions` (`fault_schedules_bulk_operations`); `a_constraint_added_not_valid_is_validated_by_a_later_migration`
  and `the_run_input_validation_rolls_back_only_while_nothing_names_it` (`integration_platform_foundation`).
  Refs: FR-043-006, NFR-043-COMPAT
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-11
  Evidence (2026-11-16): `integration_schedules_bulk_operations` 757, `fault_schedules_bulk_operations` 131 and
    `integration_platform_foundation` 107 passed in both roles at `4b6eb1a6`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-12 Read the mode and registered the capability: `SAGAT_RUN_INPUT_VALIDATION` parsed strictly in both apps (an unknown value
  stops the process naming the variable), logged at start, passed to publication and to the gate; the capability
  `run-input-validation-v1` listed only in `enforce`.
  Files: `crates/application/src/instances.rs`, `crates/application/src/definitions.rs` (`InputValidationMode`),
  `apps/execution-engine/src/capabilities.rs`, `apps/execution-engine/src/settings.rs`, `apps/control-plane/src/config.rs`,
  `apps/control-plane/src/registry.rs`, `specs/018-schedules-bulk-operations/data-model.md`.
  Tests: `apps/control-plane/tests/config.rs` (`the_run_input_validation_mode_is_read_strictly_and_enforce_by_default`,
  `no_shipped_manifest_sets_run_input_validation_to_report`); `apps/execution-engine/tests/registry.rs`
  (`the_default_engine_runs_no_schedule_pass_and_lists_only_run_input_validation`,
  `an_engine_in_report_mode_does_not_list_run_input_validation`, `the_run_input_validation_mode_of_the_engine_is_read_strictly`);
  `the_run_input_requirement_is_refused_while_an_engine_reports_and_closes_one_started_in_report_after`
  (`integration_schedules_bulk_operations`).
  Refs: FR-043-013, NFR-043-COMPAT
  Owner-gate: OD-2026-11-14-8
  Evidence (2026-11-16): the configuration and registry tests of `sagat-control-plane` and `sagat-execution-engine` at
    `f48d5729` (in the 2,338 passed); `integration_schedules_bulk_operations` 757 passed in both roles at `4b6eb1a6`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-13 Ran the pre-check at publication, in `sagat validate` and in `sagat workflow validate`, in the mode given
  (the process's mode in the control plane; `--input-validation enforce|report`, `enforce` by default, in the CLI): an `IrIssue` naming the keyword
  and its pointer in `enforce`, a `DefinitionWarning` in `report`; a warning for a schema beside a non-manual trigger;
  a version already stored, which publication parses again in `report` when it replays, activates or promotes it, so
  that it is never refused there (D043-081); a retryable `503` when the pre-check finds no permit or panics, on the
  publication, the dry run, the replay and the promotion (D043-177, D043-183, D043-186).
  Files: `crates/infra-ir/src/lib.rs` (`parse_with`), `crates/infra-ir/src/input_schema.rs`, `crates/domain/src/workflow.rs`
  (`InputSchemaIssue`, `DefinitionWarning`), `crates/application/src/definitions.rs` (`IrCodec::parse_with`,
  `IrError::ValidatorBusy`), `crates/application/src/publishing.rs`, `crates/api-http/src/error.rs`,
  `apps/control-plane/src/lib.rs` (the mode given to publication), `apps/cli/src/args.rs`, `apps/cli/src/validate.rs`,
  `apps/cli/src/workflow.rs` and `apps/cli/src/lib.rs` (`sagat workflow validate`), `apps/cli/src/diagnostics.rs`.
  Tests: `crates/infra-ir/tests/input_schema_publication.rs`;
  `publication_refuses_a_schema_outside_the_subset_in_enforce_and_warns_in_report` (`integration_gitops_versioning`);
  `the_commands_for_one_workflow_take_the_words_and_flags_they_document` (`apps/cli/tests/args.rs`);
  `the_input_schema_subset_is_checked_locally_in_the_mode_given` (`apps/cli/tests/diagnostics.rs`);
  `a_document_whose_pre_check_found_no_permit_is_unavailable_and_retryable` (`crates/api-http/tests/run_input_errors.rs`);
  `a_dry_run_whose_pre_check_found_no_permit_is_unavailable_not_a_problem` (`crates/application/tests/definition_service.rs`);
  `a_busy_stored_parse_is_unavailable_and_any_other_error_is_the_callers` (`crates/application/src/publishing.rs`);
  `a_promotion_whose_parse_finds_no_permit_is_unavailable_not_draft_has_changes` (`integration_gitops_versioning`).
  Refs: FR-043-012
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-14
  Evidence (2026-11-16): `integration_gitops_versioning` 41 passed in both roles at `4b6eb1a6`; the unit tests of
    `sagat-infra-ir`, `sagat-application`, `sagat-api-http` and `sagat-cli` at `f48d5729` (2,338 passed, after the
    `VALIDATOR_BUSY` help fix); `specs/043-run-input-validation/validation-report.md`, section "the gates of the
    approval".
- [x] T043-14 Updated the contracts: in the OpenAPI the action outcome `INPUT_INVALID`, the reasons, the closed
  `details.rule` (open to new values, which a client passes through), `details.field`, the `422` of the six start
  operations (with the 016 and 035 baselines re-pinned once, OD-2026-11-15-3), and the `Run`'s `failureCode` with its
  read from `runs.stop_code`; the subset contract and `errors.md`; the note on `inputSchema` carriers in 016's contract;
  the 018 `errors.md` row; the subset in the IR schema and `sdks/SPEC.md`; the Studio types; AsyncAPI unchanged.
  Files: `specs/001-platform-foundation/contracts/openapi.yaml`, `specs/001-platform-foundation/contracts/workflow.schema.json`,
  `specs/043-run-input-validation/contracts/input-schema-subset.md`, `specs/043-run-input-validation/contracts/errors.md`,
  `specs/016-payload-classification/contracts/README.md`, `specs/018-schedules-bulk-operations/contracts/errors.md`,
  `docs/quickstarts/errors.md`, `sdks/SPEC.md`, `frontend/apps/studio/src/api/schema.d.ts`, `crates/api-http/src/dto_exec.rs`,
  `crates/application/src/execution.rs` (`RunRecord::failure_code`), `crates/infra-postgres/src/execution.rs`
  (`RUN_SELECT`, the list query), the baselines of `contract_payload_classification` and `contract_iceberg_writes`.
  Tests: `a_run_stopped_with_a_failure_code_says_it` (`crates/api-http`); `contract_rest_durable_execution`,
  `contract_schedules_bulk_operations`, `contract_asyncapi_events_webhooks_messaging`, `contract_payload_classification`,
  `contract_iceberg_writes`; `npm run check:api`.
  Refs: FR-043-003, FR-043-006, FR-043-015
  Owner-gate: OD-2026-11-14-2, OD-2026-11-15-3
  Evidence (2026-11-16): the 23 contract targets, 366 passed, at `4b6eb1a6`; `npm run check:api` matches at `f48d5729`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-15 Gave the six SDK error types `field` and `rule`, and the six SDK `Run` types the optional `failureCode`;
  added the conformance scenario `run-input-validation`, the failure codes included (the chain's run view shows
  `failureCode` `CONTINUATION_INPUT_INVALID`); regenerated the goldens whose run views gain `failureCode`, each diff read
  first (D043-172).
  Files: `sdks/SPEC.md`, `sdks/RELEASE_NOTES.md`; per SDK its error type, its transport (which reads `field` and `rule`
  from the problem), its `Run` and its runner: TypeScript `sdks/typescript/src/errors.ts`, `src/http.ts`, `src/client.ts`,
  `conformance/runner.ts`; Go `sdks/go/error.go`, `client/transport.go`, `client/run.go`, `conformance/run.go`; Python
  `sdks/python/sagat_octopus_orchestrator/errors.py`, `_http.py`, `client.py`, `conformance/runner.py`; C#
  `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/SagatException.cs`, `Internal/HttpTransport.cs`, `Models.cs`,
  `SagatClient.cs`, `sdks/dotnet/conformance/RunMode.cs`; Java `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/`
  `SagatException.java`, `RestClient.java`, `Run.java`, `SagatClient.java`, and
  `sdks/java/conformance/src/main/java/dev/sagatoctopusorchestrator/conformance/Runner.java`; Rust `sdks/rust/src/error.rs`,
  `src/types.rs`, `sdks/rust/conformance/src/runtime.rs`; `tests/contract/sdk-conformance/scenarios/run-input-validation.json`
  and the goldens under `tests/contract/sdk-conformance/expected/`.
  Tests: the run parsing of each SDK: "a run stopped with a failure code carries it, and a run without one has none"
  (`sdks/typescript/test/client.test.ts`), `TestARunStoppedWithAFailureCodeCarriesItAndOtherRunsHaveNone`
  (`sdks/go/client/client_test.go`), `test_a_run_stopped_with_a_failure_code_carries_it_and_a_run_without_one_has_none`
  (`sdks/python/tests/test_client.py`), `ARunStoppedWithAFailureCodeCarriesItAndARunWithoutOneHasNone`
  (`sdks/dotnet/tests/SagatOctopusOrchestrator.Sdk.Tests/ClientTests.cs`),
  `aRunStoppedWithAFailureCodeCarriesItAndARunWithoutOneHasNone`
  (`sdks/java/sdk/src/test/java/dev/sagatoctopusorchestrator/sdk/SagatClientTest.java`),
  `a_run_stopped_with_a_failure_code_carries_it_and_a_run_without_one_has_none` (`sdks/rust/tests/operations.rs`); the
  tests that read `field` and `rule` beside `RUN_INPUT_INVALID`, in the same files (for Python also `sdks/python/tests/test_shared.py`; for Rust the unit tests of `sdks/rust/src/error.rs`); the per-SDK checks of `sdks/AGENTS.md`; `sdk_conformance`, every
  scenario, in six languages.
  Refs: FR-043-015, SC-043-005
  Owner-gate: OD-2026-11-14-2
  Evidence (2026-11-16): `sdk_conformance`, 78 scenarios in six languages, passed with no golden changed at `f48d5729`;
    the per-SDK checks of `sdks/AGENTS.md` passed in the six languages;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-16 Showed the refusal (`reason`, `field`, `rule`) in the CLI and in the Studio's error state.
  Files: `apps/cli/src/client.rs`, `frontend/apps/studio/src/ui/ErrorState.tsx`.
  Tests: `a_refused_run_input_is_shown_with_its_field_and_its_rule` (`apps/cli/src/client.rs`); `ErrorState.test.tsx`
  ("says where a run input was refused and by which rule, never the value").
  Refs: FR-043-015
  Owner-gate: OD-2026-11-14-2
  Evidence (2026-11-16): the `sagat-cli` unit tests at `f48d5729` (in the 2,338 passed) and the Studio tests at `f48d5729`
    (99 files, 1,530 passed, `ErrorState.test.tsx` among them); `specs/043-run-input-validation/validation-report.md`,
    section "the gates of the approval".
- [x] T043-17 Ran the sentinel across rows 1 to 14 (row 10 carries no caller value: its input is the platform's tick)
  and checked the metric labels and the metrics text.
  Files: `tests/integration/payload-classification/run_input_sentinel.rs` (rows 1, 4, 9 and 11, with the metrics text),
  `tests/integration/payload-classification/run_input_sentinel_rows.rs` (rows 3, 5, 6, 7, 8, 12 and 14),
  `tests/integration/workflow-primitives/run_input.rs` (row 2) and `tests/integration/database-connections/run_input.rs`
  (row 13).
  Tests: `no_sentinel_reaches_an_answer_a_log_a_span_or_an_audit_row_on_any_row`,
  `no_sentinel_reaches_an_answer_a_log_an_audit_a_record_or_a_metric_on_the_other_rows` (`integration_payload_classification`),
  `row_2_an_uploaded_input_start_checks_the_descriptor_and_a_refusal_claims_nothing` (`integration_workflow_primitives`)
  and the `sentinel` step of the row 13 scenario (`integration_database_connections`).
  Refs: FR-043-014, SC-043-002
  Owner-gate: OD-2026-11-14-2
  Evidence (2026-11-16): `integration_payload_classification` 43, `integration_workflow_primitives` 805 and
    `integration_database_connections` 7 passed in both roles at `4b6eb1a6`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-18 Budgeted a start, a triggered start, a child start and a continuation with and without an `inputSchema`, in
  `enforce` mode.
  Files: `tests/integration/claim-transition-efficiency/work_budget.rs`.
  Tests: `a_checked_start_costs_what_an_unchecked_one_does_once`, `a_checked_triggered_start_costs_what_an_unchecked_one_does_once`,
  `a_checked_child_start_costs_what_an_unchecked_one_does_once`, `a_checked_continuation_costs_what_an_unchecked_one_does_once`.
  Refs: NFR-043-PERF, SC-043-003
  Owner-gate: OD-2026-11-14-2
  Evidence (2026-11-16): `integration_claim_transition_efficiency` 17 passed in both roles at `4b6eb1a6`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-19 Wrote the runbook and the release notes: the warning, the subset, the inventory and the choice of `report`
  before migrating, old runs failing at a child or a continuation, recording the requirement once every process is in
  `enforce`, removing it before any process goes back to `report`; the traceability rows; G1955 and G2081 of feature
  018 closed by reference.
  Files: `docs/runbooks/run-input-validation.md`, `docs/releases/unreleased.md`, `docs/traceability.md`,
  `specs/018-schedules-bulk-operations/research.md`, `specs/043-run-input-validation/research.md`, `docs/reference`
  (regenerated after reading its diff).
  Refs: FR-043-001, FR-043-006, FR-043-013, FR-043-018
  Owner-gate: OD-2026-11-14-8, OD-2026-11-14-9, OD-2026-11-14-14
  Evidence (2026-11-16): `contract_developer_experience` 8 passed at `f48d5729`; `python3 scripts/check-design-docs.py`
    and `check-agent-config.py` passed; `specs/043-run-input-validation/validation-report.md`, section "the gates of the
    approval".
- [x] T043-20 Swept the tests, fixtures and examples whose inputs or schemas the check refuses, and changed the fixture,
  never the check.
  Files: `crates/infra-ir/tests/` (D043-095, D043-110), the classification fixtures published in `report` (`e0e345f8`),
  and `tests/contract/sdk-conformance/scenarios/builders.json` (a fractional `multipleOf` moved to `default`).
  Tests: `cargo test -p sagat-infra-ir` (the swept IR fixtures), `integration_payload_classification` (the
  classification fixtures), `sdk_conformance` (`builders`).
  Refs: SC-043-001
  Owner-gate: OD-2026-11-14-2
  Evidence (2026-11-16): the unit tests of `sagat-infra-ir` (in the 2,338 passed), `integration_payload_classification` 43
    in both roles, and `sdk_conformance` 78 scenarios; `specs/043-run-input-validation/validation-report.md`, section
    "the gates of the approval".
- [x] T043-21 Tested the budget and the transient cases on every row that has them: an input past the budget is final the
  first time with `workLimit`; no permit within the wait and the backstop are transient (`503`, a batch item deferred, a
  pending reset left pending, a schedule time owed, an event retried, a child or a continuation left for the next pass),
  never cached, never final. The permits' own tests (four held by a lone tenant, a newcomer served first, the dropped
  futures, the timed-out waiter) are T043-3's, against the real queue.
  Files: tests in `tests/integration/durable-execution/run_input.rs`, `tests/integration/schedules-bulk-operations/run_input.rs`
  and `run_input_repeats.rs`, `tests/integration/events-webhooks-messaging/run_input.rs`,
  `tests/integration/workflow-primitives/run_input.rs`.
  Tests: `the_budget_is_final_and_no_room_is_transient`, `row_6_no_room_keeps_the_time_owed_and_the_budget_is_final`,
  `a_batch_rerun_item_with_no_room_is_deferred_and_succeeds_once_there_is_room`,
  `a_pending_reset_with_no_room_is_left_pending_and_the_next_step_creates_the_run`,
  `a_synchronous_reset_with_no_room_is_unavailable_and_the_retry_makes_the_run`, `row_9_no_room_retries_the_event_and_records_nothing`,
  `row_9_the_budget_is_final_the_first_time`, `row_11_the_budget_is_final_and_no_room_leaves_the_child_for_the_next_pass`,
  `row_12_no_room_waits_and_the_budget_is_final`.
  Refs: NFR-043-BOUND, FR-043-001
  Owner-gate: OD-2026-11-14-2, OD-2026-11-14-11
  Evidence (2026-11-16): `integration_durable_execution` 192, `integration_schedules_bulk_operations` 757,
    `integration_events_webhooks_messaging` 120 and `integration_workflow_primitives` 805 passed in both roles at
    `4b6eb1a6`; `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-22 Tested `report` mode: a start through the API and an event trigger create the run and audit the refusal once
  across a retry and a replay; a continuation audits it with its hand-off; a start an open holder answers is audited
  with that answer; an ad hoc document outside the subset is reported.
  Files: tests in `tests/integration/durable-execution/run_input.rs`, `tests/integration/events-webhooks-messaging/run_input.rs`,
  `tests/integration/workflow-primitives/run_input.rs`, `tests/integration/schedules-bulk-operations/run_input.rs`,
  `tests/integration/database-connections/run_input.rs`.
  Tests: `report_mode_creates_the_run_and_audits_the_refusal_once_across_a_replay`,
  `report_mode_starts_the_triggered_run_and_audits_the_refusal_once`, `report_mode_audits_a_refused_continuation_with_its_hand_off`,
  `report_mode_audits_a_start_that_an_open_holder_answers`, the row 13 `report` case of `integration_database_connections`.
  Refs: FR-043-013
  Owner-gate: OD-2026-11-14-8
  Evidence (2026-11-16): `integration_durable_execution`, `integration_events_webhooks_messaging`,
    `integration_workflow_primitives`, `integration_schedules_bulk_operations` and `integration_database_connections`
    passed in both roles at `4b6eb1a6`; `specs/043-run-input-validation/validation-report.md`, section "the gates of the
    approval".
- [x] T043-23 Checked that every `Role` holding `run:start` also holds `definition:read`, so `details.field` names only what
  the caller may read (keeps G043-003 closed).
  Files: `crates/domain/src/identity.rs`.
  Tests: `every_role_that_may_start_a_run_may_read_the_definition_it_starts`.
  Refs: FR-043-003, FR-043-014
  Owner-gate: OD-2026-11-14-2
  Evidence (2026-11-16): the unit tests of `sagat-domain` at `f48d5729` (in the 2,338 passed), among them
    `every_role_that_may_start_a_run_may_read_the_definition_it_starts`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".
- [x] T043-24 Built `control-plane input-schema-inventory`: with the process's database settings, a paged read of the
  stored versions of every tenant (`workflow_versions` joined to `workflows`), the pre-check on each, whether something
  still starts a refused one (`version_still_started`: its workflow's active version, the pin of a schedule that is
  not deleted, an open run or one created in the last 30 days; no schedule is read on a database of the v0.2.0 release,
  which has none), the refused ones
  with their ids, issue, keyword and pointer, the informational ones (inside the subset, beside a non-manual trigger),
  no schema text; exit 0, 1 or 2; a superseded version counted, listed by `--all`, never a reason to exit 1; a schema
  that makes the validator panic listed `interrupted`, exit 2 (OD-2026-11-16-5, D043-179, D043-185, D043-187).
  Files: `apps/control-plane/src/cli.rs`, `apps/control-plane/src/inventory.rs`, `apps/control-plane/src/main.rs`,
  `crates/infra-postgres/src/definitions.rs` (`stored_versions_page`, `version_still_started`, `schedules_exist`),
  `crates/infra-ir/src/input_schema.rs`.
  Tests, each inventory run as the runtime role: `the_inventory_lists_the_versions_that_would_be_refused_and_exits_by_what_it_found`,
  `a_superseded_version_nothing_starts_does_not_fail_the_inventory_and_all_lists_it`,
  `a_version_a_live_schedule_pins_is_never_superseded` and `the_inventory_runs_on_a_database_of_0_2_0_before_the_migration`
  (`integration_gitops_versioning`); `the_input_schema_inventory_exits_2_when_it_cannot_run_and_never_shows_the_url`
  (`apps/control-plane/tests/cli.rs`); `a_version_the_validator_cannot_judge_makes_the_inventory_exit_2_and_is_listed`
  (`apps/control-plane/src/inventory.rs`).
  Refs: FR-043-018
  Owner-gate: OD-2026-11-14-14
  Evidence (2026-11-16): `integration_gitops_versioning` 41 passed in both roles at `4b6eb1a6`, the four inventory tests
    named above among them, each run as the runtime role; the `sagat-control-plane` unit and CLI tests at `f48d5729`;
    `specs/043-run-input-validation/validation-report.md`, section "the gates of the approval".

#### Follow-up tasks

- [ ] T043-25 Move design 042's blocking parse of a blob source behind a port of an adapter, and restore the rule that
  `sagat-application` names no runtime (G043-052, ADR-0092 decision 1).
  Refs: FR-043-016
- [ ] T043-26 Keep the legacy continue-as-new out of production builds (behind `test-support`, or its tests moved to the
  evaluated path and the method deleted), so rule 2 and its forwarder exemption go and `INSERT INTO runs` has two places.
  Refs: FR-043-016
- [ ] T043-27 Measure `RUN_INPUT_WORK_UNITS` on the CI machine and decide the budget (G043-006, G043-017).
  Refs: NFR-043-BOUND
- [ ] T043-28 Measure the recompiles of a tenant alone on a process that starts more than 64 schemas often, the limit
  of the tenant share the owner kept (OD-2026-11-16-6): one tenant starting 65, 128 and 256 distinct schemas in turn,
  in a release build, recording the recompiles per minute and the share of the permits' time they take. Above 5% of
  that time, put a work-conserving share to the owner as an owner question (an O043 entry of `research.md`); below it,
  record the numbers in G043-017.
  Refs: NFR-043-BOUND, SC-043-004
- [ ] T043-29 Add feature 026's run creators to Technical Plan 1 and its creator labels to Technical Plan 7 before
  feature 026's code that calls the gate (Decision 026-017 of `specs/026-invocation-human-surfaces/research.md`,
  recorded here on 2026-11-26 as a follow-up task because a change to Technical Plan 1 reopens this approved design; the
  next revision moves these four rows into the table). (a) Studio generated form: row 1 unchanged, label `api`; an
  invalid input as row 1 with the form marking the one field; a busy gate `503`, the form keeps its key and retries.
  (b) `executeRun` (`crates/api-http/src/routes/invocation.rs` → `start_with_options`, a second entry of row 1): the gate
  as row 1 before any waiter is registered, label `api`; invalid `422 RUN_INPUT_INVALID` with `field` and `rule`
  (policy `Show`) and no wait; unusable `422 RUN_INPUT_SCHEMA_UNUSABLE`; busy `503`, retryable, key not consumed.
  (c) Public-link submission (`crates/api-http/src/routes/public.rs` → `crates/application/src/public_links.rs` →
  `start_with`): `RunInputGate::check` on the merged input of the unlocked revision before the admission transaction,
  which refuses a revision other than the one checked so the use case repeats once from the gate; new label
  `public_link`; invalid `422 VALIDATION_FAILED` `RUN_INPUT_INVALID` keeping `field` only inside an exposed pointer, else
  `/` and `other`, `rule` withheld outside exposed fields; unusable the uniform `404`, audited `public_link.refused`
  `SCHEMA_UNUSABLE` and raised as the alert `PUBLIC_LINK_UNUSABLE`; busy `503` with `Retry-After` and no detail.
  (d) Synchronous custom route (with feature 021): the dispatcher, row 9 unchanged, label `trigger`; invalid and
  unusable `422` with `field` `/` and `rule` `other` (Decision 026-010); busy, the dispatcher retries as row 9 and the
  caller gets `202` at the end of its wait. A field outside the exposed pointers is refused before the gate with
  `422 VALIDATION_FAILED` `FORM_INPUT_INVALID` and `field` `/`.
  Refs: FR-043-001, FR-043-016

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`python3 scripts/check-architecture.py` with its fixtures (`python3 -m unittest scripts.tests.test_check_architecture`),
`python3 scripts/check-agent-config.py` and `python3 scripts/check-design-docs.py`, the unit tests of the crates it
changes, and the targets below. For each row whose creator it changes it runs every target that names that row; a
change to the gate, the validator or `CheckedRunInput` runs every target below; a change to publication, a promotion
or the inventory runs `integration_gitops_versioning`; to a migration, `integration_platform_foundation`; to a contract
or an SDK, every `contract_*` target named below, `sdk_conformance` and the per-SDK checks; to the mode or the
capability, the configuration and registry tests and `integration_schedules_bulk_operations`; to the Studio, its checks;
to a documented command, `contract_developer_experience`:

- `integration_durable_execution` (row 1, FR-043-004, FR-043-013, FR-043-017), also with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
- `integration_schedules_bulk_operations`, `fault_schedules_bulk_operations`, `contract_schedules_bulk_operations`
  (rows 3 to 8 and 14, the registry, the migrations as the runtime role), the integration suite also with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
- `integration_events_webhooks_messaging`, `fault_events_webhooks_messaging`, `contract_asyncapi_events_webhooks_messaging`
  (rows 9 and 10, AsyncAPI unchanged), the integration suite also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
- `integration_workflow_primitives`, `fault_workflow_primitives` (rows 2, 11 and 12, pre-upgrade parents included), the
  integration suite also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
- `integration_database_connections` (row 13 and its sentinel);
- `integration_gitops_versioning` (FR-043-012, FR-043-018: publication, the dry run, a promotion, the inventory);
- `integration_payload_classification` (the sentinels of rows 1, 3 to 9, 11, 12 and 14, and the metrics text),
  `contract_payload_classification` and
  `contract_iceberg_writes` (the re-pinned baselines);
- `integration_run_input_cache` (SC-043-004, in a process of its own);
- `integration_claim_transition_efficiency` (SC-043-003; rows 1, 9, 11 and 12): its earlier assertions pass unchanged,
  and T043-18 adds four;
- `integration_platform_foundation` (the migrations added `NOT VALID`, the guards of `20270118000000`'s down migration, the privilege
  audit);
- `contract_rest_durable_execution` and the unit tests of `crates/api-http` (the problem shape, the `Run`);
- the configuration and registry tests of `apps/control-plane` and `apps/execution-engine` (FR-043-013) and the CLI
  tests;
- `sdk_conformance` for every scenario in the six languages (`SAGAT_SDK_SCENARIO=<name> cargo test -p sagat-tests
  --test sdk_conformance --locked -- --ignored`), a golden regenerated only after its diff is read;
- the per-SDK checks of `sdks/AGENTS.md`, and `(cd frontend/apps/studio && npm run api && npm run check:api && npm test)`;
- `SAGAT_DOCS_BLESS=1 cargo test -p sagat-tests --test contract_developer_experience` only after reading the diff, then
  again without the switch.

Failure cases (constitution VIII), each a test of the tasks above: a crash between the check and the creation (the
check wrote nothing); a redelivered event (the delivery row stops a second audit); a crash between the trigger's
delivery and its audit (one transaction, so neither or both); two engines on one refused cron tick, and a writer that
stops before it moves the trigger (one audit); a peer's run under a tick's key (no audit); a child a peer creates
before or while the refusal waits for the step (not failed); a restart between a continuation's check and its failure
(failed once); a cold cache (one compile, no statement); two creators compiling one schema at once (single flight); a
compile that panics (transient, for the leader and those who waited); a budget miss (final the first time); no permit
within 5 s, a dropped waiting future, or the backstop (transient, never remembered). A test that fails is a defect to
explain, not to retry.
