# Normal scheduler discovery with genuine old retained outputs

Current base: `3710a5b7` plus the 1,121 frozen native inputs in the matching manifests.
The unchanged coordinator builds immutable old production `6c44a0e49670cf19660792cd1518d241e6c99b3b`
with its original test overlay, restores and compares every old source/migration byte, builds
the current executable, then runs old/current processes separately for two independent databases.
Both executable hashes and overlay identities are in `MANIFEST`. No old production byte changes.

## Reproduce

Use the owned PostgreSQL 16.15 server and build environment from
[the composition evidence](../collection-finalization-integration-20261018/reproduce.md), with:

```sh
export SAGAT_Q5_OWNED_POSTGRES=1
export SAGAT_Q5_ARTIFACT_ROOT=/tmp/sagat-project-completion-q9-green
scripts/test-legacy-collection-output-view.sh
```

The complete coordinator passed in **225.70 seconds**. Owner: old producer 1/1 in 11.82 seconds,
current matrix 1/1 in 30.35 seconds. Runtime: old producer 1/1 in 11.05 seconds, current matrix
1/1 in 30.36 seconds. Each current matrix has six ordered logical cases, not six tests.
The old public producer uses `sagat_app` in both phases; owner/runtime distinguishes the
current store pool. Private receipts and both coordinator-owned databases were removed.

## Exact scope

After the old process exits, the current process adopts the same recorded scheduler owner
through normal Keeper renewal. It does not assume process exit expires a database lease.
The genuine target has L=2,100,000, missing-owner V=2,100,020 and E=4,200,020, with an OPEN
full-concurrency collection and still-live RUNNING worker. A normal scheduler tick starts
ExistingOutputs drain before expiry, records FAILED/RUN_DATA_LIMIT and requests cancellation.
The test waits for the real retained deadline; normal maintenance then cancels the lost
worker, drains the member and finishes FAILED/RUN_DATA_LIMIT without releasing a successor.

The source witness checks all old closed c0..c9 scopes, members, item sources, checkpoints,
owner/member rows, journal prefix, referenced payloads and source certificates. Ten old owner
outputs remain NULL; L/V remain unchanged; no result or historical owner output is repaired.
Journal, fixed private audit, public FAILED hints and `verify_run` all match. The earlier
overlay/corruption/fitting-write reader cases still pass. The fitting case runs before global
scheduler maintenance. No lease, status, ledger or scheduler authority was fabricated.

The candidate SQL uses length metadata only as a hint before shard-local/global queue limits.
It counts each missing owner separately, exposes missing joined metadata and excludes valid
fitting legacy histories. Shared repeatable-read verification and native fenced/CAS validation
remain authoritative. This test is not a complete fairness, corruption-watcher or scan-cost
benchmark. No constant-cost discovery claim is made.

The earlier actual discovery counterexample and its separate waiting-oracle correction are in
[the RED evidence](../collection-discovery-red-20261018/reproduce.md). Both 1,121-input current
manifests match; complete old source/migration manifests match. Strict Clippy, formatting and
architecture on these same sources passed in `collection-result-composition-20261018`.

This closes this dormant-live-worker discovery case only. It does not establish old in-flight
success reporting, both report/drain orders, Applied reconciliation across process upgrade,
cold report recovery, uncertain COMMIT, source/scoped Finish activation, general nesting,
or the complete control-plane rollout/mixed-writer/downgrade/release gate.
