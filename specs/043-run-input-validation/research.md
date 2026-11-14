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

- **D043-001** (2026-11-14) One port, `RunInputValidator`, in `crates/application`, implemented in
  `crates/infra-schema`. Every creator calls it **before** the transaction that creates the run opens. The creation
  inputs (`NewRun.input` and the input of a `ContinuationSelection`) become a `CheckedRunInput`, a type only the port
  builds, so a new creator that skips the check does not compile. An escape constructor for tests is allowed only under
  `tests/` and `#[cfg(test)]`, checked by `scripts/check-architecture.py`. Rejected: putting the check in
  `PgRunStore::create` (infra layer, inside the creation transaction, and continue-as-new does not pass there).
- **D043-002** (2026-11-14) The parsed `WorkflowDefinition` carries the canonical bytes of its `inputSchema` (an opaque
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
- **D043-005** (2026-11-14) Signal-with-Start, Update-with-Start and a start with `USE_EXISTING` validate the input
  before admission, also when an open holder will answer and no run is made. Otherwise the answer would depend on a race
  with the holder's end.
- **D043-006** (2026-11-14) Dialect: the existing compile options (D043-001's adapter reuses `compile` of
  `crates/infra-schema/src/lib.rs:74`): `$schema` honoured, 2020-12 otherwise, `format` not asserted, no external
  reference, linear regex. Changing any of them is a separate decision.
- **D043-007** (2026-11-14) What a refusal says. The first violation only: `details.reason`, `details.rule` (the
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
- **D043-009** (2026-11-14) A schedule action whose input is refused is recorded with a new outcome `INPUT_INVALID`
  and the reason (`SCHEDULE_INPUT_INVALID` or `NOMINAL_TIME_FIELD_INVALID`). The time is acted on, not owed: today a
  `NotAllowed` defers the time and retries it forever (`crates/application/src/schedule_firing.rs:1203`), which would
  loop on an input that can never pass. No remaining action is used up, as for `BUSINESS_ID_CONFLICT`
  (`record_held`, :1333). The schedule pauses when its `pause_on_failure` policy is set, as for a run it started that
  failed. A backfill stops at its first `INPUT_INVALID`. Trigger-now answers `422` and records nothing, as for its other
  admission refusals.
- **D043-010** (2026-11-14) A trigger whose run input is refused starts nothing, writes a value-free audit record
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

## 3. Gaps

| ID | Gap | Reason / closure | Status |
|---|---|---|---|
| G043-001 | A control plane of 0.2.x running beside new engines during the upgrade meets the new outcome `INPUT_INVALID`. The action listing reads the outcome as a string by design (`crates/application/src/schedules.rs:634-635`, `crates/infra-postgres/src/schedules.rs:547`), so it passes through; the outcome filter parses the name strictly (`crates/api-http/src/routes/schedules.rs:275`) and an old binary answers `422` to a filter on `INPUT_INVALID`. | Read in the code, not yet tested: T043-10 adds the test of an action list with an unknown outcome. The filter case is stated in the release notes. | Open |
| G043-002 | A trigger refusal can be audited twice when the dispatcher crashes after the audit and before the event is marked processed, and the event is redelivered. | Accepted as at-least-once (constitution VII); the audit says the same thing twice. | Open (accepted by design, to be confirmed in review) |
| G043-003 | `details.field` names property names declared in the schema. A caller with `run:start` learns them even without the permission to read the version. | The names are the workflow's input contract, which a caller needs to start it. Confirmed or refused by the security review. | Open |
| G043-004 | A published version with a non-manual trigger and an `inputSchema` that describes a manual input (and not the event envelope `{event, data}`) refuses every triggered start after the upgrade. | The inventory of T043-18 lists versions with both; it cannot tell which schemas accept the envelope without real events. O043-001 decides the protection. | Open |
| G043-005 | `format` is not asserted, so a schema that relies on `format` to refuse an input does not refuse it. | D043-006; documented in the runbook. | Open (documented) |
| G043-006 | The CPU cost of validating a 256 KiB input on the API path is not measured. | Counters, not timers, prove the compile count; a benchmark is not a target of this design. | Open |
| G043-007 | Inputs stored before the upgrade (schedules, pending resets) are not checked at the upgrade; they are refused at action time. | By design: the action time is the authority (D043-008). Stated in the release notes. | Open (documented) |
| G043-008 | Signals, Updates, worker results and activity outputs are not validated against a schema by this feature. | Out of scope: OD-2026-11-14-2 is about starts. | Open (out of scope) |
| G043-009 | `infra-schema::validate` (scripts, feature 005) reports instance paths that can hold caller-chosen keys. | Observed while reading the code; not changed here (another feature's contract). Reported to the owner. | Open |

## 4. Owner questions

None answered yet. OD-2026-11-14-2 already decided that validation is on in 0.3.0 for every start; none of these asks
that again.

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
