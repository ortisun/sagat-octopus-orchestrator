# Legacy ordinary drain-first D1 handoff

## Artifacts

- Patch: `/tmp/sagat-legacy-ordinary-drain-first.patch`
- Patch SHA-256: `0fad17f65c67b5d1514bf4362adc10701ca24db69e29c72e5ffc4d0fc2587d75`
- Draft mirror: `/tmp/sagat-legacy-ordinary-drain-first-draft/`
- Consumer SHA-256: `1c8c97012d5202383ffbe2e4f77d82468fad70a98371f69ea1dca405b47640eb`
- Coordinator SHA-256: `e72e92b8821b0b9a851192b1f47468988d70dbf0d407a092b94c654263d7843c`
- README SHA-256: `ae3e7e303ccbcadb0dce67983df1878ed6cf51990b5b95cc7c2bf8bc837673c9`
- Patch size: 66,722 bytes; three files; 983 insertions and 43 deletions.

The original assignment named `f026e928fe5f9ed3db341c3c65c54a24cb69d76d` as the fixture base. Root confirmed that later unrelated commits did not change these three target files. Patch generation and applicability inspection observed clean branch `feature/016-project-completion` at `25a197df58bac3b2640c774f5662d5ec92f09446`.

## Changed files

1. `tests/integration/workflow-primitives/legacy_collection_inflight_report_v1.rs`
   - Adds exact ignored test `legacy_collection_inflight_report_v1::old_ordinary_success_after_drain_preserves_the_report_and_compensation`.
   - Uses the existing immutable-old receipt, persisted worker authority, current JWT authentication, Keeper ownership, normal `Scheduler::tick`, and real worker completion APIs.
   - Proves initial old state, ExistingOutputs drain, the still-live member wait, exact original success and replay, one real compensation target/key, no terminality while that compensation is leased, and final `COMPENSATED` state.
   - Freezes the first stop/drain event, scope drain sequence/outbox, audit and cancellation identity across report, replay and final compensation.
   - Checks output/blob reference preservation, relative report outbox count, exact tenant+scope+key receipt fingerprint/resource/created-at replay, source/authority preservation, no reconciliation or End, and cold native journal reconstruction.
   - Tightens the shared report receipt lookup to exact tenant plus canonical full scope plus key. This is required because the producer's collection task ID, and therefore key material, is shared across iterations.
   - Keeps the existing report-before-drain test and its behavior.
2. `scripts/test-legacy-collection-inflight-report-v1.sh`
   - Keeps no-argument execution as `report-before-drain`.
   - Adds the closed `--case drain-before-report` selector, its exact current test, distinct default artifact namespace and log label.
   - Preserves old/current build-before-produce ordering, forced source freshness, private receipt modes, database ownership checks and cleanup, old archive/source/migration provenance, and fresh owner/runtime databases.
3. `tests/fixtures/legacy-collection-inflight-report-v1/README.md`
   - Documents both independently produced cases, the exact selector, artifact separation, authority boundary and limitations.

The old producer and integration `main.rs` remain unchanged. No production, migration, contract, grant, binding, registration, SDK or evidence file is included.

## Static inspection performed

- `git diff --check` against the three draft files: exit 0.
- `git apply --check /tmp/sagat-legacy-ordinary-drain-first.patch`: exit 0 on the clean observed tree.
- Patch file inventory: exactly the three authorized target paths.
- Repository status was clean after patch generation; the repository was never written by this agent.

No formatter, build, Clippy, test binary, fixture runner, container, PostgreSQL command, database mutation or commit was run. These draft assertions are source-backed and unexecuted.

## Root apply and gate sequence

Apply only after rechecking the current tree:

```sh
git apply --check /tmp/sagat-legacy-ordinary-drain-first.patch
git apply /tmp/sagat-legacy-ordinary-drain-first.patch
```

Run the new compatibility case with a root-owned private target and owned fixture PostgreSQL:

```sh
CARGO_TARGET_DIR=/absolute/private/root-owned-target \
SAGAT_TEST_DATABASE_URL='<owned fixture PostgreSQL admin URL>' \
SAGAT_Q10C_OWNED_POSTGRES=1 \
scripts/test-legacy-collection-inflight-report-v1.sh --case drain-before-report
```

That coordinator should execute four processes: old producer then exact current drain-first consumer in owner mode, followed by a fresh old producer then exact current drain-first consumer as `sagat_app`. It should therefore establish two role-specific compatibility cases, each on a new owned database.

Rerun the preserved default regression separately:

```sh
CARGO_TARGET_DIR=/absolute/private/root-owned-target \
SAGAT_TEST_DATABASE_URL='<owned fixture PostgreSQL admin URL>' \
SAGAT_Q10C_OWNED_POSTGRES=1 \
scripts/test-legacy-collection-inflight-report-v1.sh
```

Then run the bounded repository gates required by the accepted plan:

```sh
cargo fmt --check
CARGO_TARGET_DIR=/absolute/private/root-owned-target \
  cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
python3 scripts/check-architecture.py
python3 scripts/check-agent-config.py
```

Read any formatting diff before changing the draft. Retain exact process statuses, durations, source and binary hashes, and nonsecret witnesses. Do not retain the private receipt.

## Limitations and stop conditions

- D2 cancellation-first is deliberately deferred. No `POST /runs/{id}/operations` cancellation case is present.
- This covers only ordinary inline, no-declared-secret `E > 4 MiB` with a still-live original lease. It does not cover `E <= limit < E + candidate`, Applied reconciliation, unknown commit, post-ACK process death, output-dependent compensation, secret or blob policies, external provider effects, nested activation, rollout or release gates.
- The fixture proves engine compensation protocol and durable state, not a provider effect, provider undo or exactly-once execution.
- No prerequisite production defect was found by source inspection. If execution drains the live member, replaces its lease/fence, rejects the exact live success, changes the first stop/drain, duplicates receipt effects, creates reconciliation, or finalizes before real compensation, retain that first genuine failure and localize it. Do not repair fixture state with SQL or add production behavior preemptively.
- Since this draft was not formatted or compiled, root must treat syntax, formatting and SQL typing as unverified until the stated gates run.
