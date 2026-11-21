# Architecture final review of design 041 (owner answers of 2026-11-15)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 35070fe9 (the owner's answers built in), with the diffs b1b2c249..69e0f828 and 69e0f828..35070fe9
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: one fresh `architecture-reviewer` agent, given the current files, the two diffs and its `architecture-reaudit.md`, not `resolution.md`; no shell. Recorded here condensed. The author's answers are in `resolution.md` section 7.

## Re-audit findings

M1 and N2-N10 answered; N11 partly ("the five resources" in T041-3 and the runbook line; there are six).

## Blocker

None.

## Major

1. ADR-0027 is amended in place although it is accepted and implemented (`docs/adr/README.md:10`, Governance); OD-2026-11-15-4 covered ADR-0084 and ADR-0085 only; the amendment has no options or consequences (a tenant's default diagnostic storage doubles to 2 GiB); O041-024 offers no "new ADR" option.
2. ADR-0084, now in force, contradicts the design: decision 8 says `log`, decision 11 lists the content and log fences while the ingest takes the step-log fence alone; the 2026-11-15 heading still says "proposed" and the body "Not in force until the owner accepts it" after "Revision accepted" (ADR-0085 too).
3. Inside a collection only one iteration of a script step gets a record and a marker: `script_executions_step_uniq` is on (tenant, run, node, attempt) (`20261014000000_scripts.up.sql:277`) and `node_of` gives every iteration the same node (`lib.rs:1309-1315`); research.md's "G041-007 narrows" and the section 7 test say otherwise.

## Nit

1. The classified trigger is not `ENABLE ALWAYS` (the other `workflow_versions` triggers are, `20261009000000_definitions.up.sql:77-78`).
2. The amendment's accounting is incomplete: `sagat_quota_row_delta` routes every non-content class to logs (`durable_quotas.up.sql:231-238`), `sagat_quota_add_delta` has a four-argument caller in the blob trigger (`:268, :273`), the manifest test filters `content|log`.
3. `tenant_step_log_quota_exceeded` needs a `classify` arm (`identity.rs:787-791`); the usage arm (`quotas.rs:1219`) and `RetainedUsage` (`content_usage.rs:147-173`) are in no task's Files.
4. Owner-gate bookkeeping: T041-5 cites OD-2026-11-13-18, whose Covers does not name it; T041-13 lacks "Waits for: O041-024"; T041-22 does not cite OD-2026-11-15-9; T041-3 publishes `tenant_step_log_bytes` before O041-024.
5. Constitution 4.5.0 does not name "the step-log suites"; `specs/.*/contracts/` is in `SHARED_INPUTS` (`select-test-targets.py:79`); the report understates the cost for later `worker.proto` changes.
6. The marker's identity is the runner's word: no lease token in the report, and the `step.leased` read filters by node only.
7. The marker rules sit in adapter SQL; a pure domain decision, as `plan_batch`, is the pattern.
8. "five resources" at two places.

Counts: Blocker 0, Major 3, Nit 8
