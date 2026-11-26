# Security review of the build: feature 041, `e7b5ac9f..24787c78` (T041-33, T041-31, T041-3, T041-4)

Reviewer: `security-reviewer` (`.claude/agents/`), 2026-11-19, read-only, given the diff and the spec set (design 041,
contracts.md, data-model.md, research.md, ADR-0084, ADR-0085, ADR-0027 with its Amendment, the constitution), not the
author's conclusions. Answers: `resolution.md`.

**Summary.** No Blocker. One Major (a CPU denial of service in the field canonicalisation, latent until T041-7 wires
the ingest), three Minors, four Notes.

## Findings

- **Major 1. A worker can pin a server CPU core by sending repeated field keys.** `crates/domain/src/step_logs.rs`
  `canonical_fields`: each copy of a key probes `k`, `k~2` … and scans the list each time, about N³/6 comparisons for
  N copies. `validate_batch` (P4) does not bound the number of fields of a record (the proto's "at most 32" is a
  comment), and P5 builds the canonical JSON over every field sent. One batch of ≤192 KiB with ~39,000 copies of one
  key costs ~10¹³ comparisons on a runtime thread. Proof: a unit test feeding 20,000 copies to `canonical_fields`
  never finishes in CI. Fix: a set of taken names and a next suffix per key; optionally bound the fields per record.
- **Minor 2. The role-matrix test binary fails.** `crates/domain/tests/identity.rs:336` asserts
  `Permission::ALL.len() == 53`; it is now 55.
- **Minor 3. The step-log usage is hidden by permission only, while ADR-0085 rule 6 requires the permission and the
  PII grant.** `crates/application/src/quotas.rs` (`usage_shown`) follows D041-066; a holder of `run:logs:read`
  without the grant of a namespace can watch `used` of `tenant_step_log_bytes` move as that namespace's runs log.
  Either require the grant in every namespace or record the residual as an accepted gap (an owner decision).
- **Minor 4. `FieldValue` derives `Debug`** (`step_logs.rs`), and `FieldValue::String` holds logged text; one `?fields`
  in a `tracing` call would copy it into the platform's logs (FR-041-007 (d), ADR-0085 rule 5). Fix: a hand-written
  `Debug` that names the variant.

## Notes

- **N1.** `openapi.yaml` describes the withholding of a run's script-execution `log` (`logWithheld`) as served, and
  nothing on the branch does it yet; say "from T041-29".
- **N2.** The down migration's usage branch (`step_log_bytes <> 0`) and the database's refusal of a namespace-level
  `tenant_step_log_bytes` row have no test.
- **N3.** `DELETE /runs/{runId}/logs?stream=` does not say what a stream of another run or tenant gets; use the `GET`
  wording ("another stream is `404`").
- **N4.** The generated Go `String()` and Python `repr` of `AppendStepLogsRequest` print tokens and line text; only the
  Rust SDK has a structural guard. The SDK tasks' redaction checks should search SDK stderr too.

## Checked and found sound

The role matrix (`run:logs:read` admin, developer, operator; `run:logs:delete` admin only; neither for viewer, worker,
runner, database-runner; PII reveal granted to no built-in role); the quota view through `usage_shown` on both `list`
and `view`, tenant-bound usage SQL, `tenant_step_log_bytes` tenant-scoped in code and in the database; the migration
(no new function, no `SECURITY DEFINER`, `sagat_app` keeps `SELECT` only on `tenant_retained_usage`, CHECKs
`NOT VALID` and validated apart, ranges equal to `maximum()`/`minimum()`, the down takes `ACCESS EXCLUSIVE` before its
guard); the `skip_debug` lists complete in the three `build.rs` files with count-only replacements and canary tests
(message, exception, key, value, session and lease tokens, `{:#?}`); the gRPC contract carries no tenant or namespace
and answers a foreign or missing attempt `LEASE`; every stub answers `UNIMPLEMENTED` and `step_logs` stays `None`;
`run.logs_read` collapses in 300 s, `run.logs_deleted` never; the domain arithmetic under `validate_batch`'s
precondition; the battery's `not_built` list and the digest's approved additions; the spike is test-only and
tenant-scoped. Not checked: the six proto copies byte for byte and the generated Go and Python code beyond N4.
