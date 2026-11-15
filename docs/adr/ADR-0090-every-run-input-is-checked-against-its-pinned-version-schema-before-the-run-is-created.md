# ADR-0090: Every run input is checked against its pinned version's schema before the run is created

**Status:** Proposed
**Decided by:** owner: that every start is validated in 0.3.0 (OD-2026-11-14-2), the mode (OD-2026-11-14-8), runs in progress (OD-2026-11-14-9), with-start (OD-2026-11-14-10) and schemas that do not compile (OD-2026-11-14-11). The mechanism is an agent's proposal, revised after the architecture, security and correctness reviews of design 043 on 2026-11-14.
**Refines / supersedes:** none. It makes the `inputSchema` of the IR (feature 001) binding, replaces the blob-only `RunInputChecker`, and adds a capability to the instance registry of ADR-0050 decision 9 and ADR-0082.
**Enforced by:** the type `CheckedRunInput` (its fields and constructor are private to the gate's module, it carries the version id, and `PgRunStore::create` and `continue_evaluated` refuse a mismatch); `scripts/check-architecture.py`, which fails on an `INSERT INTO runs` outside its three places, on `.continue_as_new(` outside tests, on the `test-support` feature outside `[dev-dependencies]`, and on a construction of `CheckedRunInput` outside its module; the missed-deadline tests with a validator double held by a barrier; `no_shipped_manifest_sets_run_input_validation_to_report`; the per-row integration tests and the sentinel test of design 043 (`integration_payload_classification`); the statement budget of `integration_claim_transition_efficiency`.

## Context

The IR declares `inputSchema` (`specs/001-platform-foundation/contracts/workflow.schema.json:47`), and until now only a
start with an uploaded input checked it (`crates/application/src/runs.rs:1673`). Thirteen paths create runs, in the API
process and in the engine, and many of them have nobody waiting for an answer: triggers, schedule actions, backfills,
children, continuations, batch items and pending resets. The parsed `WorkflowDefinition` does not hold the schema, and
continue-as-new does not hold the stored document. A published schema without an `x-sagat-classification` annotation
was never compiled (`crates/infra-ir/src/carrier_schema_check.rs:12-23`), and a schema is author data the platform
will now evaluate on every start. Under drafts 4, 6 and 7 the `jsonschema` crate asserts `format` and `content*` by
default. The run input is a classified payload. Design 038 holds the statements of the hot path to a budget.

The owner decided on 2026-11-14 that in release 0.3.0 the input of every run is validated against the target version's
`inputSchema` through one validator port (OD-2026-11-14-2), and then answered the questions of design 043
(OD-2026-11-14-8 to -11).

## Decision

1. Every path that creates a run MUST check the run's input against the `inputSchema` of the exact version the run
   pins, through the `RunInputGate`, before the transaction that creates the run opens. This includes the children and
   continuations of runs created before the upgrade, and the reruns and resets of such runs. The only exception is the
   legacy continue-as-new, which carries the predecessor's input byte for byte to the same version and MUST NOT be
   called outside tests.
2. The creation input of a run (`NewRun.input`, `ContinuationSelection`, `EvaluatedChainHandoffCommand.input`) MUST be a
   `CheckedRunInput` that names the version it was checked against. Outside tests it MUST be built only inside the
   gate's module, and it MUST NOT implement `Default` or `Deserialize`; the stores MUST refuse one whose version is
   not the run's.
3. The gate MUST call a narrow `SchemaValidator` port that takes the schema and input bytes and answers accept or a
   refusal; the policy (mode, metrics, the type) MUST stay in the gate.
4. The schema MUST come from the parsed definition of that version, which carries the schema's canonical bytes and
   their digest. In `enforce` mode the check MUST NOT add a SQL statement to a start, a triggered start, a child start or
   a continuation. Compiling and validating MUST NOT run inside a database transaction or on an async worker thread.
   They run on blocking threads that hold their permits until they end: at most four per process, and at most one
   shared and one ad hoc permit per tenant. The bound on the work MUST be an instance-aware walk charged against the
   input before validating, which follows every `$ref` and walks every applicator, so that its charge is an upper bound
   on the validator's evaluation; an input past it is refused (`workLimit`). The deadline is a backstop: a timeout while
   waiting for a permit is always transient; a miss of the work deadline MUST NOT be remembered as an unusable schema,
   and the second such miss of a schema and input (of a schema and tick, for a cron trigger) is final.
5. A compiled validator MUST be cached by tenant and schema digest, never shared across tenants, with bounded entries
   and memory and a remembered failure. An ad hoc start MUST NOT use the shared cache.
6. A schema MUST be compiled within the bounds of the classification compile (schema size, depth, number and size of
   patterns, expansion steps), with `format` and `content*` never asserting under every draft. The retriever MUST refuse
   every URI: it is the control on remote references, wherever they sit in the document. Every `$ref` MUST be a JSON
   pointer into the document; the schema MUST have no `$ref` cycle of any kind, no dynamic or recursive reference or
   anchor, no `$id` below the root, and no `$ref` under a keyword the walk does not walk. A schema outside these is
   unusable: recursive schemas are not supported.
7. A version without `inputSchema` accepts any input that passes the existing object and size checks. A version whose
   schema is unusable MUST refuse every start with `RUN_INPUT_SCHEMA_UNUSABLE`.
8. A refusal MUST name a closed reason, the failing keyword from a closed enum obtained by an exhaustive match, and a
   JSON pointer that keeps only segments matched through `properties` or array indices (`/` with rule `other` for a
   rerun or reset whose caller's view of the source input withholds anything). It MUST NOT contain, log, trace, audit or label any value of the input or any
   key the caller chose, and a validation error MUST NOT be formatted.
9. A caller who waits gets `422 VALIDATION_FAILED` (`RUN_INPUT_INVALID`, `RUN_INPUT_SCHEMA_UNUSABLE`,
   `SCHEDULE_INPUT_INVALID`, `NOMINAL_TIME_FIELD_INVALID`), after a replay of an existing run of the same idempotency key
   has been looked for. A Signal-with-Start or Update-with-Start MUST be refused for a mismatched start input even when
   an open holder would answer. A creator nobody waits for MUST record a final, value-free outcome and MUST NOT retry
   it: a trigger records a delivery without a run and audits it once; a schedule records an `INPUT_INVALID` action in
   the same decision as its auto-pause and a backfill's end; a child, after looking up whether it was already created,
   fails its parent step with `CHILD_INPUT_INVALID`;
   a continuation fails its run with `CONTINUATION_INPUT_INVALID`; both use `RUN_INPUT_SCHEMA_UNUSABLE` for an unusable
   schema.
10. The mode is `enforce` unless `SAGAT_RUN_INPUT_VALIDATION=report`, parsed strictly. A process MUST list the
    capability `run-input-validation-v1` only in `enforce`. The requirement can be recorded only once every live
    process lists it (ADR-0082 decision 2); from then on a process started in `report`, or a 0.2.x binary, stays up with
    a closed gate. An operator who wants `report` for the upgrade sets it before migrating. In `report` mode the audit of
    a reported refusal MUST be written in the transaction that creates the run. No manifest this repository ships MAY
    set `report`.
11. From 0.3.0 a new version whose `inputSchema` is unusable MUST NOT be published, annotated or not.

## Considered options

1. **A gate in `application` over a narrow validator port, a type only the gate builds, the schema in the parsed
   definition** — chosen. The compiler and three architecture rules keep a new creator from skipping the check; no
   statement is added; the work runs off the async workers and outside every transaction.
2. **The check inside `PgRunStore::create`** — one call site for most creators, but it puts JSON Schema in the
   PostgreSQL adapter, compiles under the run's locks, and continue-as-new does not pass there.
3. **A check written in each creator** — what exists for the uploaded input. It drifts, and nothing stops the next
   creator from forgetting it.
4. **Validation in the SDKs** — six implementations of a dialect, and the engine's creators have no SDK in their path.
5. **A per-process flag without the registry** — cheaper, but nothing stops one replica in `report` from answering
   differently from the others.

## Consequences

- A behaviour change for existing users: an input their version's schema refuses now fails, and a trigger on a version
  whose schema describes a manual input rather than the event envelope stops starting runs. The release notes carry a
  warning and a query that lists the versions at risk; `report` mode covers the time to fix callers.
- **Runs in progress are affected**, an accepted deviation from constitution principle IV (OD-2026-11-14-9): a run
  started before the upgrade can fail at the step that creates a child or continues, when that new run's input does not
  match its version's schema. The release notes say so.
- Recursive schemas are unusable: a published version whose schema recurses refuses every start until a version
  without recursion is published (G043-014). The release notes' inventory lists them.
- `WorkflowDefinition` grows by the schema bytes; the definition cache's byte budget counts them.
- A second compiled-validator cache, a blocking pool and an in-process memory of missed deadlines per process. A load
  spike can make a slow input final after two misses in one process (G043-011, G043-013).
- One migration widens the schedule action outcomes and the registry's capability lists, each added `NOT VALID` and
  then validated; its down migration refuses while a row holds a new value.
- In `report` mode a reported refusal writes an audit row: one statement on that start.
- Rolled back by removing the requirement and downgrading the binaries; an old control plane passes an unknown action
  outcome through in a listing and refuses it as a filter (G043-001).

## Evidence

- Design: `docs/designs/043-run-input-validation.md`; registers: `specs/043-run-input-validation/research.md`
  (D043-001 to D043-045, G043-001 to G043-014, O043-001 to O043-004).
- Background: G1955 and G2081 in `specs/018-schedules-bulk-operations/research.md`.
- Existing tests this builds on: `crates/infra-schema/tests/annotation_is_inert.rs`;
  `claim_check_schema_validation_refuses_external_refs_without_exposing_payloads` in `crates/infra-schema/src/lib.rs`;
  `no_shipped_manifest_turns_the_reset_switch_on` in `apps/execution-engine/tests/config.rs` (the pattern of rule 10).
- No benchmark yet (G043-006); the statement budget and the compile counters are the tests of rule 4.
