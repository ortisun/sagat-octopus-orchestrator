# Build review of 2026-11-19: resolution

The `security-reviewer`, `correctness-reviewer` and `architecture-reviewer` read the diff `e7b5ac9f..24787c78` of
`feature/041-step-logs-server` (T041-33 record, T041-31 spike, T041-3 contracts, T041-4 domain and quota resources)
with the design, the ADRs and the spec set; their reports are `security.md`, `correctness.md` and `architecture.md` in
this folder. Each real finding was proven by a failing test where code was involved, then fixed; the rest are
answered here. Folder name: the build began on 2026-11-18; the reviews ran on 2026-11-19.

| Finding | Answer | Evidence |
|---|---|---|
| Security Major 1, correctness M1: `canonical_fields` renames colliding keys in cubic time | Fixed: a set of taken names and a next suffix per key, linear | `step_logs::tests::many_colliding_keys_are_renamed_in_linear_time` (40,000 copies of one key within 5 s) failed with `Timeout` before the fix, passes after |
| Security Minor 2: `Permission::ALL.len()` still 53 in `crates/domain/tests/identity.rs` | Fixed (55). My earlier domain run stopped at the first failing binary and hid it | `cargo test -p sagat-domain --no-fail-fast`: every binary passes |
| Security Minor 3: the step-log usage is hidden by `run:logs:read` only, ADR-0085 rule 6 asks for the PII grant too | Built as the design decides (D041-066); the conflict with ADR-0085 rule 6 goes to the re-review that O041-036 asks for, registered as G041-018 | research.md G041-018 |
| Security Minor 4: `FieldValue` derives `Debug` | Fixed: `Debug` names the variant only | `step_logs::tests::the_debug_of_a_field_value_prints_no_value` |
| Security N1, architecture m3: `logWithheld` described as served | Fixed: the three descriptions say "planned by T041-29, not served yet" | openapi.yaml; contracts.md "Changes recorded after T041-3" |
| Security N2, correctness m7, architecture m4: migration test gaps (usage guard, namespace scope, 1 GiB and 2^50 ceilings) | Fixed | `step_log_quotas_migration::the_step_log_resources_are_accepted_and_a_rollback_keeps_them` |
| Security N3, correctness N3: `deleteRunLogs` and a foreign `stream`; no 400/422 | Fixed: `stream` says another stream is `404`; `400` and `422` declared | openapi.yaml |
| Security N4: Go and Python stubs print the request | Left to the SDK tasks T041-15 to T041-20, whose redaction scenarios search the SDK's captured output | — |
| Correctness M2, architecture M2.7: O041-026 excluded by comparing a mean with a p99 | Corrected: O041-026 is not settled by the spike; measured again with O041-036 | research.md D041-087; validation-report.md |
| Correctness M3: the driver re-takes its own ticket for the final batch | Fixed: a yield before admission; rerun recorded | validation-report.md, rerun section |
| Correctness M4: the fan-out ran at a tenth of the load | Fixed: 30 runs, the scheduler ticks again at once while it advances collections; rerun recorded | validation-report.md, rerun section |
| Correctness m1: the cut test never stepped back | Fixed: an ASCII byte first, the cut asserted at 8,191 bytes | `step_logs::tests::text_is_cut_at_a_character_boundary` |
| Correctness m2: the property checked the plan against its own counters | Fixed: the rows `keep` names are counted and compared | `step_logs::tests::stored_plus_dropped_is_the_last_sequence` |
| Correctness m3: run order biases later rows | Each row now runs the baseline again at its end; the rerun reports both | validation-report.md |
| Correctness m4: the cap was not clamped | Fixed: cap at most half the pool and at least the tickets | main.rs |
| Correctness m5: S left out lock-timeout transactions | Fixed: every transaction that ran counts | step_log_spike.rs |
| Correctness m6: departures of the spike from T041-31 | Stated in the report | validation-report.md |
| Correctness m8: the `view` path of D041-066 untested | Not fixed: its callers are managers, who hold `run:logs:read`; registered for T041-27 | — |
| Correctness N1: saturation breaks the counting rule | The module states the rule below 2^53, as SC-041-002 does | step_logs.rs module doc |
| Correctness N2: a dead branch in the gap split | Kept: harmless; T041-6 revisits it when the ingest reads the stream row | — |
| Correctness N4, security (admission), architecture: `AppendStepLogs` falls in the discovery class until T041-7 | Accepted: it answers `UNIMPLEMENTED` at once | — |
| Correctness N6: the 1 × 16 rate measures added latency | Put to the owner in those terms | O041-036 |
| Architecture B1: T041-3 and T041-4 built after T041-31 failed | Agreed. They are not ticked, the pull request is a draft, and what becomes of them is the owner's O041-037 (recommended: hold unmerged until O041-036 is answered and reviewed). D041-087 and the report now quote the design's rule | research.md O041-037, D041-087 |
| Architecture M1: one branch for four tasks | The coordinator asked for one draft pull request; its body lists the four tasks and their commits so each can be reviewed or reverted alone | PR body |
| Architecture M2: O041-036 not well formed | Restated: the ticket gate, the six paired ratios, both rows' cost for (c), a defer option, the rework of each option, (a) named as an exception | research.md O041-036 |
| Architecture M3: the migration-order hazard with feature 022 | Comment corrected (re-stamp and restate up and down); a fitness test that the last restatement of each list names every resource | `migrations::the_last_restatement_of_each_quota_resource_list_names_every_resource`, seen failing with a name removed |
| Architecture m1: the wire level number in the domain | Fixed: `RecordShape.level` is `Option<StepLogLevel>`; the adapter maps the enumeration | step_logs.rs |
| Architecture m2: the migration cites a runbook not written | Fixed: cites design section 10 and the runbook of T041-10 and T041-25 | migration comment |
| Architecture m5: the NOT VALID test does not check a later VALIDATE | Fixed | `migrations::a_constraint_added_not_valid_is_validated_by_a_later_migration` |
| Architecture m6: two changes without a D-number | D041-088 (value ranges), D041-089 (T041-34 carries T041-6's gate) | research.md |
| Architecture m7: provenance of OD-2026-11-16-11 | The row says how the answer reached this session | owner-decisions.md |
| Architecture m8: reproducibility of the spike | The report names the commits and the switch `SAGAT_PERF_STEP_LOG_ONLY` | validation-report.md |
| Architecture m9: ADR "Enforced by" and evidence | Notes of 2026-11-19 in ADR-0084 (the stop condition fired) and ADR-0085 (the Debug checks built) | the ADRs |
| Architecture m10: "Waits for" is not checked | Not changed; T041-34 carries the gate | — |
