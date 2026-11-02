# Quickstart: worker deployments and dispatch (planned demo)

**Nothing here runs today.** This is the reproducible demonstration the implementing delivery must make work
(Definition of Done: happy and failure scenario). Commands marked *(new)* do not exist yet; every other command
exists at `70f09435`. The demo uses the local development stack of
[docs/quickstarts/README.md](../../docs/quickstarts/README.md), which is **not for production**.

## 0. Prerequisites

```sh
cp infra/docker/.env.example infra/docker/.env
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env --profile seed run --rm seed
cargo install --path apps/cli --locked
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
sagat dev token --out .sagat-token && export SAGAT_TOKEN_FILE=.sagat-token   # dev-admin of tenant dev
sagat doctor                     # must report a worker protocol maximum of 1.N+2 and the `features` list (new)
```

Example workers live in `examples/worker-deployments/` *(new)*: a TypeScript worker for `payments.charge` and
`payments.refund` whose build id comes from `SAGAT_BUILD_ID`, a pinned `settlement` workflow, an auto-upgrade
`invoice` workflow, a `chunked-import` activity that checkpoints, a `review` activity that detaches, and a local
HTTPS push function (`examples/worker-deployments/push-function/`, self-signed certificate trusted only by the demo
engine). The loopback target is reachable only through the named development-only push egress exception (D1077,
new), which the engine refuses at startup outside the development profile; production images are tested to refuse it.
In `async` mode the function uses its own development machine identity with `task:complete` (D1058).

## 1. Happy path — roll a build out, keep pinned runs, retire safely (US01, US02)

```sh
sagat worker bind payments.charge payments.refund
sagat deployments create payments --pattern 'payments.*'                      # (new) current = UNVERSIONED
SAGAT_BUILD_ID=2026.10.1 npm --prefix examples/worker-deployments/typescript start &   # registers INACTIVE
sagat deployments route payments --current 2026.10.1 --reason "first versioned build"   # (new)
sagat workflow publish examples/worker-deployments/workflows/settlement.json   # dispatch.versioning = pinned
sagat workflow publish examples/worker-deployments/workflows/invoice.json      # autoUpgrade
for i in $(seq 1 20); do sagat workflow run settlement --input '{"amount": 10}'; done
SAGAT_BUILD_ID=2026.10.2 npm --prefix examples/worker-deployments/typescript start &   # second build, INACTIVE
sagat deployments route payments --ramp 2026.10.2 --ramp-percent 10 --reason "canary"   # (new)
for i in $(seq 1 100); do sagat workflow run invoice --input '{"amount": 5}'; done
sagat deployments route payments --current 2026.10.2 --reason "promote"        # (new)
sagat deployments show payments                                                # 2026.10.1 DRAINING, open pins N
```

Expected evidence:

- `sagat workflow inspect <run>` shows, per attempt, `dispatchMode pull` and the build that served it; every
  `settlement` run started before the promotion lists only `2026.10.1`; `invoice` runs list the build their bucket
  selected at each claim.
- The share of `invoice` runs served by `2026.10.2` during the ramp equals the share of their buckets below 1,000
  (the e2e test computes buckets from the run ids; there is no tolerance).
- When the last `settlement` run ends: `sagat deployments retire payments 2026.10.1 --reason "drained"` *(new)*
  succeeds; restarting a `2026.10.1` worker is refused with `VERSION_RETIRED`.

## 2. Happy path — checkpoint, detach, fairness, push (US03–US06)

```sh
sagat workflow run chunked-import --input '{"rows": 100000}'   # kill the worker mid-way, restart it
sagat workflow inspect <run>          # attempt 2 lists checkpoint {attempt: 1, size}; values only with disclosure
sagat workflow run review --input '{"doc": "d-1"}'             # the worker detaches and prints the token to the demo console
node examples/worker-deployments/external/complete.mjs <token>   # authenticates as a principal with task:complete
sagat push-endpoints create thumbnails --kind https --target https://127.0.0.1:18443/render \
  --activity thumbnails.render --auth signature-only --mode sync     # (new)
sagat workflow run thumbnails --input '{"image": "i-1"}'          # completed by the push function
```

Fairness: `tests/e2e/worker-deployments-dispatch/fairness.rs` *(new)* starts 1,000 runs with key `A` and 50 with
key `B` (weight 3) and 10 priority-1 runs, and asserts the claim order recorded in the journal.

## 3. Failure scenarios (must be shown, not only the happy path)

| Scenario | Command | Expected |
|---|---|---|
| Crash during a routing change | `cargo test -p sagat-tests --test fault_worker_deployments_dispatch --locked -- routing_change_crash` *(new)* | no lease to an unrouted version; one routing revision |
| Retire a version with open pins | `sagat deployments retire payments 2026.10.1 --reason x…` before drain | `409 VERSION_NOT_DRAINED` with counts; nothing changes |
| Oversized checkpoint | worker records 100 KiB | lease renewed, `details_status = REFUSED_SIZE`, `heartbeat_details_total{outcome="refused_size"}` incremented, previous checkpoint kept |
| Token of a reclaimed attempt | let the detached timeout expire, then complete | `409 STALE_ATTEMPT`; audit `task.token.denied`; step retried |
| Foreign or forged token | complete with a token from tenant `other`, or one built from known ids with a guessed secret | `404`, indistinguishable from a missing one; no receipt or status disclosed |
| Token replayed on gRPC | call `ResolveSecrets` or `CompleteStep` with the decoded lease | `ABORTED STALE_ATTEMPT` (`DETACHED`) |
| Async function faster than the invoker | the function completes through REST before the invoker records its `202` | completion applied once; the invoker records nothing |
| Push function times out after receiving | function sleeps past `requestTimeoutMs` | `OUTCOME_UNKNOWN`; reconciliation or manual review |
| Push endpoint to the metadata address | create with `https://169.254.169.254/` | `422 PUSH_DESTINATION_DENIED`, audited |
| Engine restart with detached and pushed attempts | `docker compose restart execution-engine` | detached attempts keep their expiry; in-flight push leases expire and are retried with the same key |

## 4. Tests that close the demo

```sh
cargo test -p sagat-tests --test e2e_worker_deployments_dispatch --locked          # (new)
cargo test -p sagat-tests --test fault_worker_deployments_dispatch --locked        # (new)
cargo test -p sagat-tests --test integration_worker_deployments_dispatch --locked  # (new)
cargo test -p sagat-tests --test contract_worker_deployments_dispatch --locked     # (new)
cargo test -p sagat-tests --test sdk_conformance --locked -- deployment-versioning heartbeat-details \
  async-completion dispatch-priority push-envelope                                  # new scenarios
cargo test -p sagat-tests --test contract_rest_security_multitenancy --locked       # battery incl. new routes
```

The results, environment and any skipped case go in the delivery's verification record; a skipped infrastructure
test is not passing evidence. Real AWS Lambda and Cloud Run invocations are deployment-gated and reported apart.
