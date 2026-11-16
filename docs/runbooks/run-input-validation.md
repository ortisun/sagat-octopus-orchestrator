# Run input validation (design 043)

From release 0.3.0 every run input is checked against the `inputSchema` of the workflow version the run pins, before the
run is created (ADR-0090). This runbook is what an operator does before, during and after the upgrade. The contract of
the refusals is `specs/043-run-input-validation/contracts/errors.md`.

## What changes

- A start whose input the version's `inputSchema` refuses is `422 VALIDATION_FAILED` with `details.reason`
  `RUN_INPUT_INVALID`, `details.field` (a JSON pointer of names the schema declares, never a value) and `details.rule`.
  It used to be accepted.
- An `inputSchema` must stay within the supported subset of JSON Schema (`workflow.schema.json`, `inputSchema`). A
  stored version whose schema is outside it refuses every start (`RUN_INPUT_SCHEMA_UNUSABLE`), and 0.3.0 publication
  refuses a new one in `enforce` mode. `format` is an annotation and never refuses an input (G043-005); `pattern`
  refuses `\d`, `\w`, `\s`, `\b`, Unicode and POSIX classes, flags and named groups (write `[0-9]` and the like,
  G043-016); `multipleOf` takes only positive integers (G043-015).
- Creators nobody waits for record a final outcome instead: a trigger records a delivery without a run and one audit
  record `run.input_refused`; a schedule records the action `INPUT_INVALID` (and pauses when `pauseOnFailure` is set); a
  child fails its parent step with `CHILD_INPUT_INVALID`; a continuation fails its run with
  `CONTINUATION_INPUT_INVALID`; a batch rerun item ends `FAILED VALIDATION_FAILED`; a pending reset ends
  `ADMISSION_REFUSED`.
- **Runs in progress are affected** (OD-2026-11-14-9): a run started before the upgrade can fail at the step that creates
  a child or continues, when that new run's input does not match its version's schema.
- A trigger on a version whose schema describes a manual input (and not the event envelope `{event, data}`) stops
  starting runs (G043-004).

## Before the upgrade

1. List the versions at risk, with the control plane's database settings (the runtime role is enough: it only reads):

   ```sh
   control-plane input-schema-inventory
   ```

   It prints, for each stored version whose schema would be refused, its ids, the keyword and its pointer, and, for
   information, the versions within the subset that have an `inputSchema` and a non-manual trigger. It prints no schema
   text and no input and writes nothing. Exit 0: nothing would be refused; 1: at least one version would; 2: the command
   failed (it could not connect, or a stored document could not be read).
2. Decide the mode. If callers or schemas cannot be fixed before the upgrade, set
   `SAGAT_RUN_INPUT_VALIDATION=report` in the manifest of **every** execution engine and control plane **before you
   migrate** (D043-040): the engines are upgraded first and default to `enforce`, so the first upgraded engine would
   otherwise record final outcomes (rows 4 to 12 of the design) before anyone could choose. The variable is read
   strictly: `enforce`, `report`, or unset or empty (`enforce`); anything else stops the process naming it.

## During the upgrade

The order of OD-2026-11-14-3 holds: the migration, then the engines, then the control planes. The migration adds the
action outcome `INPUT_INVALID` and the capability `run-input-validation-v1`; each constraint is added `NOT VALID` and
then validated. Until the requirement is recorded, a 0.2.x control plane admits what a new one refuses, and replicas may
run different modes (G043-010). An old control plane passes an unknown action outcome through in a listing, and refuses
`INPUT_INVALID` as a filter (G043-001).

In `report` mode every check still runs; a refusal is counted (`run_input_validations_total{outcome="reported"}`),
audited once with the run it admits (`run.input_refused`, reason `reported: ...`), and the run is created. The final
outcomes recorded before a switch to `report` stay as they were recorded.

## After the upgrade: lock it

When every caller and schema is fixed:

1. Restart every engine and control plane in `enforce` (unset the variable).
2. Record the requirement `run-input-validation-v1` for `CONTROL_PLANE` and `EXECUTION_ENGINE` through the instance
   registry ([instance-registry](instance-registry.md)). Recording is refused while any live replica lacks the capability,
   that is, while one runs in `report` or is a 0.2.x binary (ADR-0082 decision 2).
3. From then on a process started in `report`, or a 0.2.x binary, stays up with its gate closed.

Going back to `report`: remove the requirement first, then restart in `report`.

## Watching it

- `run_input_validations_total{creator, outcome}`: `accepted`, `no_schema`, `refused`, `schema_unusable`, `reported`,
  `deadline` (no permit within 5 s or the backstop: transient, never remembered).
- `run_input_schema_cache_total{result}`: `hit`, `miss`, `evicted`, `failure_remembered`, `unshared` (ad hoc starts).
- `trigger_events_total{outcome="input_invalid" | "input_schema_unusable"}`; `scheduler_continuation_input_refusals_total`.
- Audit records `run.input_refused`. No metric label, log line, span or audit record carries an input value, a field
  name or a tenant.

A sustained `deadline` means the validator has no room: four validations run at once per process, a tenant alone may use
all four, and a caller who waits more than 5 s gets a retryable `503`.

## Rollback

Remove the requirement, then downgrade the binaries. The down migration refuses while an `INPUT_INVALID` action, a
recorded requirement, a history row or a registered process naming `run-input-validation-v1` exists.
