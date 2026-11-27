# Review: feature 018, the automatic handover (ADR-0091) — architecture

- **Doc:** `docs/adr/ADR-0091-a-one-shot-job-hands-the-cron-triggers-over-after-a-deployment.md` (the spec), with `docs/constitution.md`, ADR-0082, ADR-0083 and `docs/engineering/architecture-principles.md`
- **Reviewed:** the diff of `crates apps infra tests .github` against the merge base with `develop` (`3268f621`), the working tree of `feature/018-schedules-default-on` on 2026-11-15, **after** the fixes of the security and correctness reviews of the same day
- **Reviewer:** `architecture-reviewer` (independent read-only agent, given the diff and the ADRs, not the author's conclusions)
- **Recorded by:** the author of the slice, from the reviewer's full report (summarised by finding, every finding kept)
- **Answered in:** the commit that adds this file (D2311, G2313 to G2315)

## Findings

| Finding | Severity | How it is answered |
|---|---|---|
| M1 — the ADRs' text lags the build: "the runtime role MUST get no new privilege" while `sagat_app` reads the attempt rows and the audit count; ADR-0082 still calls ADR-0091 proposed; the owner's floor, the cap of sixteen runs, the third alert, `FailJob` on exit 3 and the optional Secret keys are not in the ADR | Major | **Fixed in text.** ADR-0091 decision 1 and an amendment of 2026-11-15; ADR-0082's entry of 2026-11-14 |
| M2 — nothing holds that only the definers may write the attempt rows the wrapper trusts | Major | **Fixed.** `trigger_auto_handover::the_runtime_role_may_only_read_the_handover_row_the_attempt_rows_and_the_audit_count` (every privilege of `sagat_app` on the handover's tables: three `SELECT`s and nothing else) |
| M3 — the released-v0.2 scenario never sees `ENGINE_UNREGISTERED` | Major | **Open (G2313)**: it runs only in the CI job of the released upgrade |
| m1 — the activation's bounds are set by the client only | Minor | **Open (G2314)** |
| m2 — the operator's `--apply` matches error text | Minor | **Said in ADR-0091's amendment**: decision 4's typed reasons are the job's |
| m3 — the refusal call-site test checks a named test exists, not that it reaches the site, and misses `ConfigError::Invalid` and direct `RunError`s | Minor | **Open (G2315)** |
| m4 — SCRAM computed by hand; `postgres-protocol` has it with SASLprep | Minor | **Open (G2315)** |
| m5 — compose's `${SAGAT_HANDOVER_MIN_LIVE:-1}` resets a higher floor at every `migrate` | Minor | **Fixed.** Empty by default, which writes nothing |
| m6 — `helm upgrade` with 0.2 values fails at render until the Secret is named | Minor | **Fixed in the release notes** (`docs/releases/unreleased.md`) |
| m7 — the build is on two branches | Minor | **Answered**: `feature/018-auto-handover-build` was merged into this branch (`9a235fc1`) |
| Nits — every `54000` read as `TOO_MANY_RUNS`; stale `target/debug` binaries in a local run; `sha2` and `zeroize` not workspace dependencies | Nit | Recorded (G2315) |

## Checked by the reviewer and found sound

The layers (the application crate gains only closed enums, a view and one port method with a default; SQL in `infra-postgres`; the loop and the signals in `apps/control-plane`; no empty mappers); the database decides and the wrapper recomputes under the epoch lock with the exact `session_user` and a fresh round; ADR-0082 decision 1 widened for `sagat_handover` alone, held by the privilege audit; ADR-0083 decision 4 as amended; one short transaction per round and nothing held across a wait; the migration additive, backfilling `OPERATOR`, its `down` refusing once a mode exists; the catalog test; the `--auto` path typed by SQLSTATE and its exit codes; closed label sets at zero and the one-set test of the reasons; the promtool tests of every rule file in CI; the plain Job, its name, `podFailurePolicy`, service account and NetworkPolicy; the compose one-shot service waiting for healthy dependencies; every CI shard building the binaries the process tests run.
