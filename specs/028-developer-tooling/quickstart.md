# Quickstart — feature 028 (planned demo)

Nothing below runs today: the commands, files and outputs are the demo the implementation must make reproducible
(Definition of Done). Each step names the success criterion it demonstrates. When a step cannot run on a machine (no
Docker, no provider credentials, no five toolchains), the demo report says so instead of claiming it.

## Prerequisites

- The pinned Rust toolchain, a PostgreSQL the tests can create databases in (`SAGAT_TEST_DATABASE_URL`), Node 20+ for the
  TypeScript SDK and the extension; the other four toolchains for the five-language run.
- `cargo build -p sagat-cli -p sagat-test-server --locked` (two binaries; the test server is never in the production
  image).

## 1. Thirty days in seconds (US01, SC-028-001)

```sh
sagat test-server start --database-url "$SAGAT_TEST_DATABASE_URL" --json > /tmp/sagat-test.json
# {"schemaVersion":1,"command":"test-server.start","ok":true,"exitCode":0,
#  "testServer":{"apiUrl":"http://127.0.0.1:18080/v1","grpcUrl":"http://127.0.0.1:19080",
#                "adminTokenFile":".../admin.token","workerTokenFile":".../worker.token","namespace":"test"}}
# examples/time-skipping/workflow.json (T028-027): timer P30D, a step retried twice, an approval that expires after P7D
SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_NAMESPACE=test sagat --token-file .../admin.token \
  workflow publish examples/time-skipping/workflow.json
```

```ts
// TypeScript (the other four SDKs have the same calls)
const env = await TestEnvironment.connect({ apiUrl, grpcUrl, namespace: 'test', adminToken, workerToken });
const worker = await env.worker({ activities: { 'reminder.send': flakyTwiceThenOk } });
const run = await env.client().start('reminder-30d', { customer: 'c-1' }, { deadline: { days: 60 } });
const done = await env.waitForCompletion(run.id, { skipTime: true });
// done.status === 'SUCCEEDED'; journal: timer.started … timer.fired 30 days later (workflow time);
// two retries at the policy's instants; approval.expired after 7 days.
```

Expected: the test finishes in seconds of wall time; `env.now()` is 37+ days after start; the journal's event types and
order equal those of the same workflow run with 2-second timers on real time.

### Failure scenarios

- **Blocked advance** (SC-028-004): start a run of a workflow whose first step nobody serves and call
  `env.advance('P1D')`: `CONFLICT TIME_ADVANCE_BLOCKED`, `blockedBy.readySteps = 1`, `env.now()` unchanged.
- **Waiting for the outside**: a run waiting for a signal; `waitForCompletion(skipTime)` raises `RUN_WAITING_EXTERNAL`
  with `waitingFor = [SIGNAL]`; the clock is unchanged.
- **Step vs jump** (SC-028-004): a daily cron; `sagat test-server advance --by P30D --mode STEP --json` starts 30 runs;
  on a fresh server `--mode JUMP` starts 1.
- **Never in production** (SC-028-002): `control-plane serve` with `SAGAT_ENV=prod` and the test database's URL exits 2
  with `TEST_DEPLOYMENT_DATABASE`; `python3 scripts/check-production-image.py` fails on a Dockerfile that names the
  test server; `curl -H "Authorization: Bearer $PROD_TOKEN" https://prod.example/v1/testing/clock`
  answers `404 ROUTE_NOT_FOUND`; a test-server token is `401` on production.

## 2. Interceptors (US02, SC-028-005)

```ts
const gateway: ClientInterceptor = { interceptCall: (call, next) => { call.headers.add('X-Gateway-Tag', 'ci'); return next(call); } };
const client = env.client({ interceptors: [recorder('A'), recorder('B'), gateway] });
await client.start('hello', { name: 'Ada' });       // A→B→gateway→(031 converter, if any)→transport, and back; once
```

Interceptors observe and decorate; transforming payload bytes is feature 031's data converter (decision C5).

Failure: the conformance harness drops the first answer to `POST /runs`; the server receives two byte-identical bodies
with the same `X-Gateway-Tag` under one key and returns one run. An interceptor that tries to replace the input or set
`Idempotency-Key` is refused with `PROTECTED_FIELD` before anything is sent. An interceptor that throws before sending yields `INTERCEPTOR_FAILED` and no request;
one that throws after the answer yields `INTERCEPTOR_FAILED` with `AFTER_RESPONSE`.

## 3. Replay before an upgrade (US03, SC-028-006 — needs feature 020's export)

```sh
sagat history replay ./histories/orders-2026-10-20.json --json   # against the new build's test server
# {"ok":true,"replay":{"compatible":true,"events":214,"divergences":[],"unsupportedCapabilities":[]}}
```

Failure: change one `step.completed` into `step.failed` in a copy; the report has one divergence `STEP_STATUS` at that
sequence, `compatible: false`, exit 1. `sagat history answers` on a history whose outputs were not disclosed lists the
activities it could not answer and writes no value.

## 4. VS Code (US04, SC-028-007)

Open `examples/time-skipping/workflow.json`; remove an edge target; on save the Problems view shows `GRAPH` at the node with the help
line of `docs/quickstarts/errors.md`. "Sagat: Open graph preview" shows the graph and its table. "Sagat: Publish to dev
and run" publishes and follows the run. In a namespace of environment `prod` the command refuses unless
`sagat.allowNonDev` is set in user settings. Open the folder as untrusted: only validation and preview are available.

## 5. Git sync (US05, SC-028-008/009)

```sh
# a namespace secret holds the GitHub App private key (OpenBao), another the webhook secret
curl -X PUT .../v1/namespaces/$NS/git-binding -H 'Idempotency-Key: bind-orders-1' -d '{"settings":{
  "provider":"GITHUB","apiBaseUrl":"https://api.github.com","repository":"acme/workflows","branch":"main",
  "manifestDir":"orders","auth":{"kind":"GITHUB_APP","appId":12345,"installationId":678,"privateKeySecret":"gh-app-key"},
  "webhookSecret":"gh-webhook"}}'
```

Publish `orders` v4 in the Studio → a pull request from `sagat-sync/orders` with `orders/orders.json` and `orders/sagat.yaml`.
Merge a change to `orders.json` in GitHub → the draft of `orders` shows "from commit 1a2b3c4". With
`DRAFT_AND_REQUEST` on a branch whose rules require a review, a publication request is opened that shows the commit
author as unverified and needs the namespace's approvals plus one; on an unprotected branch the import reports
`GIT_REVIEW_NOT_VERIFIED` and opens none. Edit the draft in the
Studio and push another change to the same file → the Studio shows a conflict with the three documents; choose "keep
draft".

Failures (provider stub in `tests/fault/developer-tooling/`): crash after the provider created the commit → one branch,
one pull request after restart; provider down during publish → the publication commits, the export retries and is
`DEAD` after 12 attempts, redrive succeeds; a webhook dropped → the head reconciliation imports within one interval; a
request with another binding's signature → `401`.

## 6. A curated template (US06, SC-028-010)

```sh
sagat template list --kind WORKFLOW --json
sagat template import order-routing --version 1.0.0 --name order-routing --json
# toPrepare: {"activities":["order.validate","order.notify"],"secrets":[]}, draftRevision 1
```

Failures: serve an index with a lower `version`, one past `expiresAt`, one signed by a key not pinned, one whose file
digest differs — each import is refused with its `TEMPLATE_*` reason and creates nothing.

## Validation commands (planned targets)

```sh
cargo test -p sagat-tests --test integration_developer_tooling --locked
cargo test -p sagat-tests --test fault_developer_tooling --locked
cargo test -p sagat-tests --test contract_developer_tooling --locked
SAGAT_SDK_SCENARIO=time-skipping cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=interceptors  cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
npm --prefix frontend run test --workspace @sagat/vscode-extension
python3 scripts/check-architecture.py
```
