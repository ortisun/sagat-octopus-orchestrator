# Validation quickstart: multi-region replication and active-passive failover

**Nothing here runs yet.** This is the demo the implementation must make reproducible (Definition of
Done): a happy path (planned switchover) and a failure path (unplanned failover with a partition and a
live former primary). Commands, files and targets are the ones `tasks.md` creates; an expected outcome
is a criterion to check, not a recorded result.

## Prerequisites

- The pinned Rust toolchain; PostgreSQL **server** binaries (`PGBIN`, as for `infra/dr/drill.sh`) or
  Docker for `infra/docker/compose.region.yaml` (pinned images, loopback-only ports, health checks).
- `nats-server` (or Docker), Node for the Studio, and the SDK toolchains for the conformance step.
- About 2 GiB of disk; a private `CARGO_TARGET_DIR` outside the repository (AGENTS.md rule 19).

## 1. Start a region pair on one host

```sh
infra/dr/region-drill.sh --up            # three PostgreSQL clusters: eu-west-1 primary, eu-west-2 in-region
                                         # standby, eu-central-1 cross-region standby, with
                                         # synchronous_standby_names = 'ANY 2 (eu-west-2, eu-central-1)'
                                         # (both acknowledge: cross-region protection and feature 010's
                                         # in-region RPO 0, research D1591); a WAN proxy with 20 ms injected
                                         # RTT, two blob stores with a replicator, two NATS, one control plane
                                         # and one engine per region, and a resolver override mapping cell
                                         # endpoints to eu-west
export DATABASE_URL=postgres://sagat_owner@127.0.0.1:55501/sagat      # owner role, eu-west-1 (eu-west-2 is 55502, eu-central-1 is 55503)
SAGAT_CELL_ID=c1 SAGAT_REGION_REGIONS=eu-west,eu-central SAGAT_REGION_ACTIVE=eu-west \
SAGAT_REGION_MODE=sync_quorum \
SAGAT_REGION_ACTOR=demo SAGAT_REGION_REASON="initialize the demo cell" control-plane region-init
```

Expected: `{"outcome":"done","epoch":1}`; from now on the runtime role is read-only by default and a
process that does not identify its region cannot write (`infra/dr/region-drill.sh --old-binary` starts
an engine without the settings and shows its lease takeover refused by the trigger); `curl -s -H "authorization: Bearer $TOKEN"
https://eu-west.local:8443/v1/region` → `role: ACTIVE, epoch: 1, protection: PROTECTED`; the same call to
`eu-central` → `role: PASSIVE`; `curl -si https://eu-central.local:8443/v1/ready` → `503` with reason
`REGION_PASSIVE`; `region_role{role="passive"} 1` on the eu-central engine's `/metrics`.

## 2. Happy path: planned switchover (US02, SC-032-001)

```sh
tests/performance/multi-region-failover/ledger.sh start   # starts runs, signals, webhooks and completions
                                                          # through the cell endpoint; records every ack
SAGAT_REGION_EXPECTED_EPOCH=1 SAGAT_REGION_ACTOR=demo SAGAT_REGION_REASON="planned switchover drill" \
  control-plane region-drain                              # admissions → 503 REGION_DRAINING; shards given back
infra/dr/region-drill.sh --stop-primary eu-west           # clean stop; prints the final LSN
infra/dr/region-drill.sh --promote eu-central
DATABASE_URL=postgres://sagat_owner@127.0.0.1:55503/sagat \
SAGAT_REGION_EXPECTED_EPOCH=1 SAGAT_REGION_TARGET=eu-central SAGAT_REGION_KIND=planned \
SAGAT_REGION_FINAL_LSN=<printed LSN> SAGAT_REGION_ACTOR=demo SAGAT_REGION_REASON="planned switchover drill" \
  control-plane region-activate
infra/dr/region-drill.sh --route eu-central               # flips the resolver override (the "DNS")
tests/performance/multi-region-failover/ledger.sh verify  # compares acks with the new authority
psql -X -At -f infra/dr/journal_digest.sql "$EU_CENTRAL_URL"   # digest of the cut, both sides
```

Expected: activation `{"outcome":"done","epoch":2,"cutLsn":"…"}`; ledger `missing: 0`; digests equal;
every run replays (`dr_verify`); `scheduler_shards_owned` sums to 64 in eu-central within one keeper
round; subscribers with `ops.region.activated` receive one notice per namespace; Studio's Operations →
Region shows epoch 2, active `eu-central`.

## 3. Failure path: unplanned failover with a partition (US03, SC-032-002/003/004)

```sh
infra/dr/region-drill.sh --reset --mode sync_quorum
tests/performance/multi-region-failover/ledger.sh start
infra/dr/region-drill.sh --partition                      # cuts eu-west ↔ eu-central; clients still reach eu-west
sleep 15                                                  # longer than the 5 s statement timeout on purpose
infra/dr/region-drill.sh --promote eu-central
DATABASE_URL=postgres://sagat_owner@127.0.0.1:55503/sagat \
SAGAT_REGION_EXPECTED_EPOCH=1 SAGAT_REGION_TARGET=eu-central SAGAT_REGION_KIND=unplanned \
SAGAT_REGION_ATTESTATION="drill: eu-west isolated by the proxy; no other standby" \
SAGAT_REGION_ACTOR=demo SAGAT_REGION_REASON="unplanned failover drill" control-plane region-activate
infra/dr/region-drill.sh --route eu-central
infra/dr/region-drill.sh --heal                           # eu-west processes now see epoch 2 at their peer
tests/performance/multi-region-failover/ledger.sh verify
```

Expected: during the partition eu-west acknowledges nothing (`db_commit_outcome_unknown_total` grows,
clients get `UNAVAILABLE COMMIT_OUTCOME_UNKNOWN`, keyed repeats and webhook resends get
`DURABILITY_UNCONFIRMED` once the 3 s durability lease lapses, `region_durability_lease_valid 0`; with
`--restart-former` the former primary comes back and still serves nothing); after `--heal` eu-west processes report `SUPERSEDED`
within one probe interval and `SagatRegionPeerEpochAhead` fires; ledger `missing: 0`; keyed retries
created each run once. Repeat with `--mode async`: the ledger reports the missing acknowledgments and the
window, and

```sh
SAGAT_REGION_FORMER_DATABASE_URL=postgres://sagat_reader@127.0.0.1:55501/sagat \
SAGAT_REGION_REPORT_PATH=./divergence.json control-plane region-divergence-report
```

lists every one of them by identifier, with no payload (the canary strings planted by the ledger are
absent from the file); `control-plane region-emit-tombstones` with the same report sends whole-run
tombstones to a test 020 sink, which also received per-run tombstones when runs continued in the new
epoch.

## 3b. In-region failover inside the cell (D1591)

```sh
infra/dr/region-drill.sh --reset --mode sync_quorum
tests/performance/multi-region-failover/ledger.sh start
infra/dr/region-drill.sh --kill eu-west-1 --promote eu-west-2   # feature 010's procedure; epoch unchanged
tests/performance/multi-region-failover/ledger.sh verify
```

Expected: ledger `missing: 0`; `region_inregion_rpo_guaranteed 1` before the kill.

## 4. Automated suites (each named by its task)

```sh
cargo test -p sagat-tests --test integration_multi_region_failover --locked
cargo test -p sagat-tests --test fault_multi_region_failover --locked      # region loss mid-commit, partition,
                                                                           # duplicates, timeout/signal race,
                                                                           # lost broker, failed compensation, restart
cargo test -p sagat-tests --test contract_multi_region_failover --locked   # schema hazards, OpenAPI/AsyncAPI/proto
cargo test -p sagat-tests --test e2e_multi_region_failover --locked
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_multi_region_failover --locked
for s in region-passive-reconnect region-drain-handover region-failover-reregister region-stale-report \
         region-stream-reset region-commit-unknown region-replay-durability; do
  SAGAT_SDK_SCENARIO=$s cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
done
infra/dr/region-drill.sh                    # all scenarios; writes tests/performance/reports/multi-region-failover.md
```

## Gates

- Ledger `missing: 0` in every `sync_quorum` scenario with protection `PROTECTED`; an `async` loss is
  reported with its window, never hidden.
- The negative controls (fence without epoch, commit without the guard) fail as they must.
- The report names the environment and states that a one-host simulated WAN is not a region pair.
- `[x]` in `tasks.md` only with the command and its output.

## Reviews

Independent read-only reviews (security, correctness, architecture) are recorded here by T032-052, with
what each finding became.
