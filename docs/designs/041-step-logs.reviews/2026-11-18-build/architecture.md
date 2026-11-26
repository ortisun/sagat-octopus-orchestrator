# Architecture review of the build: feature 041, `e7b5ac9f..24787c78` (T041-33, T041-31, T041-3, T041-4)

Reviewer: `architecture-reviewer` (`.claude/agents/`), 2026-11-19, read-only, given the diff, the design, the ADRs, the
constitution and the spec set, not the author's conclusions. Answers: `resolution.md`.

**Verdict: one Blocker, three Majors.** The code is well layered and the migration and contracts are additive; the
problem is the process: contract and schema work went ahead after the spike failed its stop rule.

## Blocker

- **B1. T041-3 and T041-4 were built after T041-31 failed, which the design and ADR-0084 forbid.** Design, Open
  Questions: "a failure stops the feature for a design change and a new review, and nothing after T041-31 starts until
  then"; tasks land in the order listed, T041-3 and T041-4 after T041-31; ADR-0084: "a failure stops the feature for a
  design change". The spike failed (validation-report.md), yet the branch carries the T041-3 contract and the T041-4
  schema. D041-087 and the report read the rule as "T041-5 and the tasks after it wait", a narrower reading of the task
  text. Consequence: the wire contract and part of the schema are fixed before the design change O041-036 asks for;
  O041-036 (d) would change the proto, O041-026 (b) the usage column; a merged migration cannot be edited. Smallest
  correction: land only the T041-31 evidence; hold T041-3 and T041-4 unmerged until O041-036 is answered and reviewed,
  or record an owner exception; correct D041-087 and the report to the design's rule.

## Major

- **M1. One branch bundles four tasks** (T041-33's record, T041-31, T041-3, T041-4), where the design asks one pull
  request per task and documentation-only ones for owner answers. Open stacked pull requests.
- **M2. O041-036 is not well formed.** It leaves out the second failed gate (one calibration needed 17 tickets against
  at most 10); the noise argument does not fit six paired failures; option (c) quotes the 8 × 128 cost only (1 × 16
  lost 25–44%); option (a) is a re-measurement, an exception to an approved gate; option (b) is a fix the design's own
  SDK rule implies, but `sdks/SPEC.md` has no step-log section yet; there is no "defer" option and no statement of the
  rework each option causes; the O041-026 exclusion compares a mean with a p99.
- **M3. The migration-order hazard with feature 022 is understated and nothing checks it.** Migrations run in version
  order, not merge order; 022's `20270130000300_flow_control_quota_resources` has the earlier stamp, so on a fresh
  database the later stamp wins whichever merges first, and re-stamping plus restating up and down is needed.
  `check-upgrade-boundary.py` compares against the maximum version only. Fitness function: a static test that the last
  up migration restating each resource list names exactly `QuotaResource::ALL` (namespace-scoped ones for
  `quota_limits_tenant_scoped`).

## Minor

- **m1.** `StepLogLevel::from_wire(i32)` and `RecordShape.level: i32` bring protobuf numbering into the domain; map in
  `api-grpc`.
- **m2.** The migration comment cites a runbook entry that does not exist yet.
- **m3.** `openapi.yaml` describes `logWithheld` as served; mark it planned (T041-29).
- **m4.** The migration test misses the usage branch of the down guard, namespace-level rows and the tenant-scoped
  refusal.
- **m5.** The NOT VALID fitness test does not check that a later migration validates each constraint.
- **m6.** The value-range correction of data-model.md and the change of T041-33's done condition (T041-34) have no
  D-number.
- **m7.** The provenance of OD-2026-11-16-11 ("received by the coordinator") is vague.
- **m8.** The spike report names "0856adad plus the uncommitted driver" and not the switch `SAGAT_PERF_STEP_LOG_ONLY`.
- **m9.** ADR-0084's evidence ("None measured yet") is stale and no Note records that its stop condition fired;
  ADR-0085's "Enforced by" does not name the built `Debug` checks.
- **m10.** `check-design-docs.py` does not enforce "Waits for"; T041-34's carrier relies on people noticing.

## Checked and found sound

The domain is pure (no serde_json, SQLx, Axum or I/O; typed `BatchInvalid` without values; the counting rule
property-tested; fields as canonical text; `minimum()` equal to the former "a policy refuses 0" rule elsewhere). The
migration is additive (`ADD COLUMN … NOT NULL DEFAULT 0` is metadata-only; CHECKs `NOT VALID` and validated apart; the
down takes `ACCESS EXCLUSIVE`, refuses with a typed SQLSTATE and restores the previous ranges and lists line by line;
the oracle moved in all three files). The 1.7 additions are purely additive; `PROTOCOL_MINOR` stays 6 and `step_logs`
is `None`; all seven implementations answer `UNIMPLEMENTED`, as an older replica would; the generated stubs record the
pinned tools; `skip_debug` in the three build scripts. The four operations are `x-planned`, the reference and the
Studio types regenerated, the quota additions match data-model §6, the roles match the design. T041-5 was not started;
the spike stays in `tests/performance` behind its switch with a scratch schema; T041-31 is unticked with honest
evidence; T041-34 is a legal follow-up task.
