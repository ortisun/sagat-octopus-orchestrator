# ADR-0090: Every run input is checked against its pinned version's schema before the run is created

**Status:** Proposed
**Decided by:** owner (OD-2026-11-14-2, that every start is validated in 0.3.0); the mechanism is an agent's proposal, to be reviewed with design 043
**Refines / supersedes:** none. It makes the `inputSchema` of the IR (feature 001) binding, and replaces the blob-only `RunInputChecker` of the uploaded-input start.
**Enforced by:** the type `CheckedRunInput`, which only the `RunInputValidator` port builds and which `NewRun` and the continuation input require; `scripts/check-architecture.py`, which fails when the test constructor of that type is named outside `tests/` or `#[cfg(test)]`; the per-creator integration tests and the sentinel test of design 043 (`integration_payload_classification`); the statement budget of `integration_claim_transition_efficiency`.

## Context

The IR declares `inputSchema` (`specs/001-platform-foundation/contracts/workflow.schema.json:47`), and until now only a
start with an uploaded input checked it (`crates/application/src/runs.rs:1673`). Thirteen paths create runs, in the API
process and in the engine, and many of them have nobody waiting for an answer: triggers, schedule actions, backfills,
children, continuations, batch items and pending resets. The parsed `WorkflowDefinition` does not hold the schema, and continue-as-new does not hold
the stored document. A published schema without an `x-sagat-classification` annotation was never compiled
(`crates/infra-ir/src/carrier_schema_check.rs:12-23`). The run input is a classified payload. Design 038 holds the
statements of the hot path to a budget.

The owner decided on 2026-11-14 (OD-2026-11-14-2) that in release 0.3.0 the input of every run start is validated
against the target version's `inputSchema` through one validator port, including schedules, and that an input that does
not match is refused where it was accepted.

## Decision

1. Every path that creates a run MUST check the run's input against the `inputSchema` of the exact version the run
   pins, through the `RunInputValidator` port, before the transaction that creates the run opens. The only exception is
   the legacy continue-as-new, which carries the predecessor's input byte for byte to the same version and is reachable
   from tests only.
2. The creation input of a run (`NewRun.input`, the continuation input) MUST be a `CheckedRunInput`. Code outside
   `tests/` and `#[cfg(test)]` MUST NOT build one other than through the port.
3. The schema MUST come from the parsed definition of that version, which carries the schema's canonical bytes. The
   check MUST NOT add a SQL statement to a start, a triggered start, a child start or a continuation, and MUST NOT
   compile a schema inside a database transaction. A compiled validator is cached per version id, with bounded entries
   and memory and a remembered compile failure.
4. A version without `inputSchema` accepts any input that passes the existing object and size checks. The dialect is
   `$schema` when present, else draft 2020-12, `format` as an annotation, no external reference, patterns on the
   linear-time engine; `x-sagat-classification` does not change the result.
5. A refusal MUST name a closed reason, the failing keyword from a closed list, and a JSON pointer cut before the first
   segment that is not an array index or a property name declared in the schema. It MUST NOT contain, log, trace, audit
   or label any value of the input or any key the caller chose.
6. A caller who waits gets `422 VALIDATION_FAILED` with `RUN_INPUT_INVALID` (`SCHEDULE_INPUT_INVALID` or
   `NOMINAL_TIME_FIELD_INVALID` for a schedule), after a replay of an existing run of the same idempotency key has been
   looked for. A creator nobody waits for MUST record a visible, value-free outcome and MUST NOT retry what cannot pass:
   a trigger audits `run.input_refused` and counts it; a schedule records an `INPUT_INVALID` action and does not keep
   the time owed; a child fails its parent step with `CHILD_INPUT_INVALID`; a continuation fails its run with
   `CONTINUATION_INPUT_INVALID`.
7. From 0.3.0 a new version whose `inputSchema` does not compile MUST NOT be published, annotated or not.

What design 043 leaves to the owner (a published schema that does not compile, a lever for existing users, the repeat of
a pre-0.3.0 run) is decided there (O043-001 to O043-003) and does not change these rules, except that rule 6 can be
relaxed to "count and admit" by the `report` mode if O043-001 adopts it.

## Considered options

1. **One port called by every creator, a type that only the port builds, the schema in the parsed definition** —
   chosen. The compiler and one architecture rule keep a new creator from skipping the check; no statement is added;
   the check runs outside every transaction.
2. **The check inside `PgRunStore::create`** — one call site for most creators, but it puts JSON Schema in the
   PostgreSQL adapter, compiles under the run's locks, and continue-as-new does not pass there.
3. **A check written in each creator** — what exists for the uploaded input. It drifts: that check compiles on every
   call and words its own error, and nothing stops the next creator from forgetting it.
4. **Validation in the SDKs** — six implementations of a dialect, and the engine's creators have no SDK in their path.

## Consequences

- A behaviour change for existing users: an input their version's schema refuses now fails, and a trigger on a version
  whose schema describes a manual input rather than the event envelope stops starting runs. The release notes, the
  runbook inventory and the owner's answer to O043-001 address it.
- `WorkflowDefinition` grows by the schema bytes; the definition cache's byte budget counts them.
- A second compiled-validator cache per process.
- A migration widens the schedule action outcomes; its down migration refuses while a row holds `INPUT_INVALID`.
- Runs in flight are not affected: the check runs only where a run is created. A continuation of a chain started
  before the upgrade is checked when its successor is created.
- Rolled back by downgrading the binaries; the data written (an action outcome, audit rows, failure codes) stays
  readable by the newer binaries only for the outcome filter (G043-001).

## Evidence

- Design: `docs/designs/043-run-input-validation.md`; registers: `specs/043-run-input-validation/research.md`
  (D043-001 to D043-016, G043-001 to G043-009).
- Background: G1955 and G2081 in `specs/018-schedules-bulk-operations/research.md`.
- Existing tests this builds on: `crates/infra-schema/tests/annotation_is_inert.rs`;
  `claim_check_schema_validation_refuses_external_refs_without_exposing_payloads` in `crates/infra-schema/src/lib.rs`.
- No benchmark yet (G043-006); the statement budget and the compile counters are the tests of rule 3.
