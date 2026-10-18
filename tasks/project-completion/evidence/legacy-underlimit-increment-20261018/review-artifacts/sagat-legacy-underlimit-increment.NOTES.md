# Q10c under-limit increment TMP draft

## Source pin

- Frozen source baseline: `5ad34370e9bec4ebfe5aab6e8b802f6a670c73f8` (`test(primitives): preserve old success after public cancel`).
- The four owned paths at the shared worktree's later HEAD were byte-identical to this baseline when the patch was generated.
- Immutable-old engine pin retained by the phased fixture: `6c44a0e49670cf19660792cd1518d241e6c99b3b`.
- Accepted design input: `/tmp/sagat-legacy-underlimit-increment-plan.md`; its fresh decision review reported no findings.

| Owned source | Baseline SHA-256 | Draft SHA-256 | Draft lines | Diff + / - |
| --- | --- | --- | ---: | ---: |
| `tests/fixtures/legacy-collection-inflight-report-v1/producer.rs` | `c12a5986c7561fb27c711c505c1fad1da17d2e751fefeb2301d2231cbca616e4` | `37396041360c7e75cfc90b4b86601db691658cfee4462c921ba0a74b88858441` | 622 | 152 / 19 |
| `tests/integration/workflow-primitives/legacy_collection_inflight_report_v1.rs` | `29adb90e4efc5dc76062d417eac3fed023ad90c22f47f005123e78bd6a3bee41` | `38b7cfb4e7473ac752e304ccb3c598b0be0bd610926365fa7b835768d38e99d7` | 3829 | 566 / 44 |
| `scripts/test-legacy-collection-inflight-report-v1.sh` | `5e62d14358e2439f23f363ebd5c8907ddec0fa6cefb840822c96c9c4ed7a076a` | `6a82384ef438c13b03a8e1072688de1e6ba7e77c3861f719ef38b990f542d1de` | 462 | 12 / 4 |
| `tests/fixtures/legacy-collection-inflight-report-v1/README.md` | `fafd8b6e660d12f43c720d0431ff89680995eb4d99be7b2d7318474e46c9acc6` | `0b215ff8417344f1c53e878d194523b300fd0699e7dd8fd4edab9dba514fac6c` | 110 | 39 / 14 |

Patch: `/tmp/sagat-legacy-underlimit-increment.patch`, SHA-256 `7b64876c6fb61658278c23de6a65ecb6a34ad27fe206b9efb9f3838545c52d30`, 55,499 bytes, 1,217 lines.

## Draft contract

- Adds only the closed selector `--case under-limit-increment-before-drain`, exact named old/current tests, format, artifact namespace and log label requested by the accepted plan.
- Keeps the original no-argument, drain-first and public-cancellation-first mappings and their ordinary 210,000-byte source / candidate `1` preset. The 19-field private receipt and old defaults remain unchanged.
- The new producer uses ten actual 200,000-byte JSON string outputs and 200,002-byte aggregates. It asserts `L=2,000,000`, `V=2,000,020`, `E=4,000,020`, candidate `d=250,000`, `L+d=2,250,000`, and `E+d=4,250,020`; each value stays below 262,144 bytes.
- Before reporting, the current consumer reads the complete production `RunTx::data` overlay, checks the production effective scalar, proves no durable backfill, excludes `c10` from discovery, gets `CollectionAdvance::Waiting`, observes a zero-advance tick, and requires an exact fenced native `ExistingOutputs` command to fail with `InvalidTransition` while the complete execution footprint stays unchanged.
- It submits the retained original report and preserves the existing successful-refusal, receipt, replay, source, output, payload, audit and live-capability checks. Added ordering checks require the first FAILED/RUN_DATA_LIMIT stop before the successful completion and no alternative forward outcome or derived effect.
- After the report, it drops the first scheduler and reconstructs a fresh pool, store, definition cache, placement, Keeper and Scheduler. Only normal `tick` starts the drain. Exact drain evidence requires `stopped_for_run_data_limit`, zero `existing_outputs` and `aggregate` drain audits, the public FAILED outbox reason, unchanged E, and stable stop/drain identity.
- It polls and holds the real compensation, proves normal scheduling cannot finalize the leased obligation, completes `line.release`, reaches `COMPENSATED`, and replays the original forward report from `stored_report_receipt` after terminal state without using the current-head verification helper.
- No production, migration, grant, lockfile, support, main registration, canonical spec/task, or evidence path is part of the patch.

## Review units

1. Producer preset and arithmetic: review the closed enum, unchanged Ordinary branches, exact serialization, unique labels and unchanged 19-field receipt.
2. Consumer fixture validation and pre-report controls: review preset parsing, full overlay reader, fresh scheduler construction, exact native rejection and unchanged-footprint hashes.
3. Consumer report/drain/compensation sequence: review the extracted first-case helper, Ordinary conditional branches, exact report/drain evidence, pending compensation barrier and terminal stored-receipt replay. This is the large unit: the consumer diff is 566 additions / 44 deletions and should receive a dedicated review before execution.
4. Coordinator and README: review the fourth closed mapping and narrow documented claim; all coordinator build, ownership, timeout, credential and cleanup logic remains the baseline logic.

## Intended root gates

1. Reconfirm baseline/path ownership, inspect the complete patch, run the repository formatter/check mode and shell syntax check, and review any formatter diff before accepting it.
2. Run strict integration-target Clippy and the architecture/agent-config checks with a private absolute `CARGO_TARGET_DIR`, after `df -h`, with at most two heavy builds.
3. Run the new phased selector in owner and runtime roles against root-owned repository-declared PostgreSQL infrastructure. Require one exact old test and one exact ignored current test per fresh database and verify cleanup/manifest evidence.
4. Independently rerun the default, drain-first and cancellation-first selectors in both roles, preserving all three existing mappings and D2 public-cancellation audit, receipt and original-stop assertions.
5. If helper integration exposes a concrete risk, run the focused native ExistingOutputs/StoppedForRunDataLimit regression. Treat every failure as a defect to investigate; do not retry away or weaken an oracle.

## Execution status and limits

No formatter, shell parser, compiler, test, SQL, container, architecture check or repository write was run by this draft agent. The patch is a source-reviewed hypothesis for root to apply and execute. It does not claim a process death restart, external provider effect or undo, exactly-once behavior, Applied reconciliation, output-dependent compensation, or closure of Q10c / feature 014 release tasks.
