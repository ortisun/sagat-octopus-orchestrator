# TMP-only D2 cancellation-first handoff

## Base and ownership

- Drafted from the committed D1/C2 versions of the three target files at `22fe7c2680ab834f1aac6b55f47230256b2311c7`.
- `git apply --check` also passed at repository HEAD `d30f5a74320bdb3f1c250277ac9e2698585007bd`; the three target files were unchanged there.
- Root remains the sole repository writer and executor. This agent wrote only the three mirrors under `/tmp/sagat-legacy-cancellation-first-draft/`, this patch, and this notes file.

## Patch scope

The patch changes exactly:

1. `tests/integration/workflow-primitives/legacy_collection_inflight_report_v1.rs`
2. `scripts/test-legacy-collection-inflight-report-v1.sh`
3. `tests/fixtures/legacy-collection-inflight-report-v1/README.md`

It adds only the ignored D2 consumer `old_ordinary_success_after_cancellation_drain_preserves_the_original_stop` and runner selector `--case cancellation-before-report`. The immutable-old producer and integration `main.rs` are not patched.

## D2 evidence encoded by the test

- Uses the public `POST /runs/{run}/operations` route as fresh current `alice`/`acme`, namespace `payments`, with the exact stable key, exact reason, and actual pre-cancellation journal version.
- Verifies HTTP 202, accepted `CANCELLING`, deterministic operation ID, the one exact cancellation audit and HTTP trace, and the complete command receipt identity and SHA-256 length-prefixed request hash.
- Proves cancellation commits while `c10` remains OPEN and the original old lease remains live and uncancelled.
- Uses only normal `Keeper` ownership and `Scheduler::tick` to start the `CANCELLED` drain, preserving the public stop and choosing cancellation before the genuinely old `E > limit` data-limit path.
- Verifies exact stop/drain/cancel order, `drain_sequence` at the cancellation event, a live-member wait with unchanged reservations and run/tenant iteration usage, zero data-limit drain audit, and one `CANCELLED` drain outbox.
- Submits and replays the genuine old success. It remains `SUCCEEDED/CANCELLING`, retains no forward output, emits no report outbox, records one successful-output-refusal audit, and preserves the exact scoped report receipt.
- Replays the public cancellation after the journal advances and requires the current status/version while retaining the deterministic operation ID and original operation receipt.
- Polls and completes the real `line.release` compensation against the original target/key. While leased, Run and saga remain `COMPENSATING`, cause `RUN_CANCELLED`, with no terminal event.
- Requires final Run/scope/owner `CANCELLED` with null failure code, saga `DONE/COMPENSATED`, original forward `SUCCEEDED` without output, and no reconciliation task.
- Runs cold `verify_run` checks through a fresh store/cache at the public stop, drain, report, public replay, and final state.

## Review split in the draft consumer

- Public operation fingerprint, deterministic ID and receipt: lines 527-626.
- Shared successful-refusal parameterization: lines 750-856.
- Public cancel identity/audit/receipt oracle: lines 958-1124.
- Cancellation drain/live lease oracle: lines 1444-1621.
- Shared pending compensation cause and terminal helper parameterization: lines 1622-1755.
- New D2 test: lines 2586-3297.
  - setup and public cancel: 2586-2808
  - normal cancellation drain and live wait: 2809-2872
  - original report, both replays, and receipt preservation: 2873-3054
  - real compensation, pending state, and final cancellation: 3055-3297
- Coordinator selector: script lines 8-37; existing manifest/exact execution/log/cleanup path remains at lines 331-454.
- Documentation: README lines 3-85.

## Static checks performed

- `git diff --no-index --check` reported no whitespace errors for each mirror.
- `git apply --check /tmp/sagat-legacy-cancellation-first.patch` passed.
- `git apply --stat` reports only the three authorized target files.

No formatter, compiler, build, test, container, or database command was run. No runtime result is claimed.

## Root apply and gates

```sh
git apply --check /tmp/sagat-legacy-cancellation-first.patch
git apply /tmp/sagat-legacy-cancellation-first.patch
git diff --check
cargo fmt --check
```

First retain the focused D2 result as real evidence, including any RED public-route failure:

```sh
CARGO_TARGET_DIR=/absolute/private/target \
SAGAT_TEST_DATABASE_URL='postgres://…/postgres' \
SAGAT_Q10C_OWNED_POSTGRES=1 \
scripts/test-legacy-collection-inflight-report-v1.sh --case cancellation-before-report
```

Then rerun both existing selectors to protect the committed D1/default behavior:

```sh
scripts/test-legacy-collection-inflight-report-v1.sh --case drain-before-report
scripts/test-legacy-collection-inflight-report-v1.sh
```

Use the repository's normal locked Clippy, architecture, and broader regression gates after the focused fixture passes.

## Limits

This D2 case establishes only ordinary no-secret `E > limit` cancellation-before-report ordering and public engine compensation. It does not prove an external provider effect or business undo, exactly-once behavior, Applied reconciliation, `E <= limit < E + candidate`, a post-ACK process restart, output-dependent compensation, secret or blob policy, or the remaining authority, race, and fault matrix. If the first real public CANCEL run fails, keep that failure RED and localize the genuine production prerequisite; do not replace the route with direct SQL state fabrication.
