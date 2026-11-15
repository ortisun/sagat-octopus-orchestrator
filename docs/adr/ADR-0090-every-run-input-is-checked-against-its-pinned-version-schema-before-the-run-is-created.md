# ADR-0090: Every run input is checked against its pinned version's schema before the run is created

**Status:** Proposed
**Decided by:** owner: that every start is validated in 0.3.0 (OD-2026-11-14-2), the mode (OD-2026-11-14-8), runs in progress (OD-2026-11-14-9), with-start (OD-2026-11-14-10), schemas that do not compile (OD-2026-11-14-11) and the supported subset with a validator of the platform (OD-2026-11-14-14). The mechanism is an agent's proposal, revised after four rounds of architecture, security and correctness review of design 043 on 2026-11-14.
**Refines / supersedes:** none. It makes the `inputSchema` of the IR (feature 001) binding, replaces the blob-only `RunInputChecker`, and adds a capability to the instance registry of ADR-0050 decision 9 and ADR-0082.
**Enforced by:** the type `CheckedRunInput` (its fields and constructor are private to the gate's module, it carries the version id, and `PgRunStore::create` and `continue_evaluated` refuse a mismatch); `scripts/check-architecture.py`, which fails on an `INSERT INTO runs` outside its three places, on `.continue_as_new(` outside tests, on the `test-support` feature outside `[dev-dependencies]`, and on a construction of `CheckedRunInput` outside its module; a rule of `scripts/check-architecture.py` that rejects `jsonschema` under the run-input validator module, with its negative fixture, and a rule that fails when `tokio` becomes a normal dependency of `sagat-application`; the oracle property test of the validator against `jsonschema` and its adversarial budget cases (regression checks, not proofs); the vocabulary test and `control-plane input-schema-inventory`'s test; the backstop tests with a validator double held by a barrier; `no_shipped_manifest_sets_run_input_validation_to_report`; the per-row integration tests and the sentinel test of design 043 (`integration_payload_classification`); the statement budget of `integration_claim_transition_efficiency`.

## Context

The IR declares `inputSchema` (`specs/001-platform-foundation/contracts/workflow.schema.json:47`), and until now only a
start with an uploaded input checked it (`crates/application/src/runs.rs:1673`). Thirteen paths create runs, in the API
process and in the engine, and many of them have nobody waiting for an answer: triggers, schedule actions, backfills,
children, continuations, batch items and pending resets. The parsed `WorkflowDefinition` does not hold the schema, and
continue-as-new does not hold the stored document. A published schema without an `x-sagat-classification` annotation
was never compiled (`crates/infra-ir/src/carrier_schema_check.rs:12-23`), and a schema is author data the platform
will now evaluate on every start. Four review rounds showed that the cost of validating an arbitrary schema with the
`jsonschema` crate cannot be bounded beforehand by a static walk: `unevaluated*` re-runs branches, the walk and the
`referencing` crate decode pointers differently, and value-sized keywords grow with the input. The run input is a classified payload. Design 038 holds the statements of the hot path to a budget.

The owner decided on 2026-11-14 that in release 0.3.0 the input of every run is validated against the target version's
`inputSchema` through one validator port (OD-2026-11-14-2), and then answered the questions of design 043
(OD-2026-11-14-8 to -11), and after the fourth review chose a supported subset (OD-2026-11-14-14).

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
3. The gate MUST call a narrow, object-safe `SchemaValidator` port, held as `Arc<dyn SchemaValidator>`, whose `check`
   takes the key, the schema and the input bytes and returns a boxed future of accept or a refusal; the policy (mode,
   metrics, the type) MUST stay in the gate, which is not generic over the validator.
4. The schema MUST come from the parsed definition of that version, which carries the schema's canonical bytes and
   their digest. In `enforce` mode the check MUST NOT add a SQL statement to a start, a triggered start, a child start or
   a continuation, and MUST NOT run inside a database transaction or on the async scheduler. The adapter behind the
   port, not `application`, owns that: `block_in_place`, or in place on a current-thread runtime, holding a permit of a
   bounded pool (by default four per process) shared work-conservingly (a free permit goes to the waiting tenant that
   holds the fewest; a tenant MAY hold more than half only while no other waits), with a bounded wait (by default 5 s);
   a dropped waiter MUST release what it holds.
5. An `inputSchema` MUST be within the documented subset: `type`, `properties`, `required`, `items` (one schema),
   `enum`, `const`, the numeric bounds, `multipleOf` (a positive integer), the length bounds, a `pattern` whose parsed
   syntax passes an allowlist (no Perl, Unicode or POSIX classes, no word boundaries, flags or named groups) and whose
   compiled program stays within 16 KiB and 512 NFA states, `additionalProperties`, `allOf`, `anyOf`, `oneOf`, a local
   `$ref` (a JSON pointer whose target is a schema position, no cycle, no sibling keyword but annotations), `$defs` and
   `definitions`, the ignored annotations (`title`, `description`, `default`, `examples`, `$comment`, `format`; `$schema`
   and `$id` at the root only) and `x-sagat-classification`, with the same meaning when `$schema` is absent, draft-07 or
   2020-12, with no application path longer than 128 and no lower bound of the units any input pays above the budget. Any other keyword, a remote or cyclic `$ref`, or another
   `$schema` makes the schema unusable. `patternProperties`, `uniqueItems` and `not` are out of 0.3.0.
6. The input MUST be validated by the platform's own validator of that subset, which carries a work counter: every
   loop step and every subschema application costs at least one unit (an empty schema included), an `enum` or `const`
   comparison costs per member and by the size of the values, a length check costs its length, and a `pattern` match
   costs (bytes + 1) × NFA states, charged before it runs; the counter stops the whole validation (never one branch)
   before a step that would pass the budget. References MUST be compiled to node indices, never inlined, and iteration MUST NOT depend on a hash seed,
   so replicas report the same violation. A budget
   miss is deterministic and final (`RUN_INPUT_INVALID`, `workLimit`). The counter also stops past 5 s of work as a
   backstop; that stop and a permit not obtained are transient and MUST NOT be remembered. The compiled form MUST be
   cached by tenant and schema digest, never shared across tenants, with bounded entries and memory and a remembered
   `Unusable`; an ad hoc start MUST NOT use the shared cache.
7. A version without `inputSchema` accepts any input that passes the existing object and size checks. A version whose
   schema is unusable MUST refuse every start with `RUN_INPUT_SCHEMA_UNUSABLE`.
8. A refusal MUST name a closed reason, the failing keyword as the validator names it from a closed enum, and a
   JSON pointer that keeps only segments matched through `properties` or array indices (`/` with rule `other` for a
   rerun or reset whose caller's view of the source input withholds anything). It MUST NOT contain, log, trace, audit or label any value of the input or any
   key the caller chose.
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
11. From 0.3.0 a new version whose `inputSchema` is outside the subset MUST NOT be published in `enforce` mode, annotated
    or not: the refusal names the keyword and its pointer. In `report` mode it is a warning. `control-plane
    input-schema-inventory` MUST list the stored versions that would be refused.

## Considered options

1. **A documented subset validated by a validator of the platform with a work counter, behind a gate in `application`,
   with a type only the gate builds** — chosen (OD-2026-11-14-14). The bound holds by construction, because the counter
   counts what the validator does; the static pre-check only checks the vocabulary, the references and their cycles; the
   compiler and six architecture rules keep a new creator from skipping the check, `jsonschema` off the path and the
   runtime out of `application`.
2. **The `jsonschema` library with a static walk as the bound on its work** — rejected after four review rounds, each of
   which found work the walk did not charge or a reference it resolved differently.
3. **Validation in an isolated process with a timeout** — it bounds the damage without bounding the work, adds a round
   trip to every start and a process to deploy and watch. Rejected for latency and operations.
4. **The check inside `PgRunStore::create`** — it puts JSON Schema in the PostgreSQL adapter, runs under the run's locks,
   and continue-as-new does not pass there.
5. **A per-process flag without the registry** — nothing stops one replica in `report` from answering differently.

## Consequences

- A behaviour change for existing users: an input their version's schema refuses now fails, and a trigger on a version
  whose schema describes a manual input rather than the event envelope stops starting runs. The release notes carry a
  warning and a command that lists the versions at risk; `report` mode covers the time to fix callers.
- **Runs in progress are affected**, an accepted deviation from constitution principle IV (OD-2026-11-14-9): a run
  started before the upgrade can fail at the step that creates a child or continues, when that new run's input does not
  match its version's schema. The release notes say so.
- `WorkflowDefinition` grows by the schema bytes; the definition cache's byte budget counts them.
- A validator of the platform to maintain, and a subset narrower than JSON Schema: a published schema that uses a keyword
  outside it (recursion, `patternProperties`, `not`, `if`, `unevaluated*`, a fractional `multipleOf`, `\d` in a pattern)
  refuses every start of its version in `enforce`, until a version within the subset is published. The inventory
  command lists them before the upgrade; `report` mode covers the time to republish (G043-014 to G043-016).
- A second compiled-schema cache and a permit pool per process, both in `crates/infra-schema`. `crates/application`
  keeps no runtime: the gate is plain async code over the port, and the adapter gains tokio as a normal dependency
  (option (b) of the architecture review, D043-071). `jsonschema` stays out of the validator, by a rule (D043-074).
- The subset narrows what `inputSchema` may hold: a schema that 0.2.x published is refused by 0.3.0 publication in
  `enforce`, recorded in `workflow.schema.json` and `sdks/SPEC.md` (OD-2026-11-14-14).
- One migration widens the schedule action outcomes and the registry's capability lists, each added `NOT VALID` and
  then validated; its down migration refuses while a row holds a new value.
- In `report` mode a reported refusal writes an audit row: one statement on that start.
- Rolled back by removing the requirement and downgrading the binaries; an old control plane passes an unknown action
  outcome through in a listing and refuses it as a filter (G043-001).

## Evidence

- Design: `docs/designs/043-run-input-validation.md`; registers: `specs/043-run-input-validation/research.md`
  (D043-001 to D043-077, G043-001 to G043-017, O043-001 to O043-004).
- Background: G1955 and G2081 in `specs/018-schedules-bulk-operations/research.md`.
- Existing tests this builds on: `crates/infra-schema/tests/annotation_is_inert.rs`;
  `claim_check_schema_validation_refuses_external_refs_without_exposing_payloads` in `crates/infra-schema/src/lib.rs`;
  `no_shipped_manifest_turns_the_reset_switch_on` in `apps/execution-engine/tests/config.rs` (the pattern of rule 10).
- No benchmark yet (G043-006); the statement budget and the compile counters are the regression checks of rule 4, and
  the oracle property test and the adversarial budget cases those of rules 5 and 6.
