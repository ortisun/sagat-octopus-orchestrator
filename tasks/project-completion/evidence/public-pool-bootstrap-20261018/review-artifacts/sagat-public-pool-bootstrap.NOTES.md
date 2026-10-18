# TMP draft: supplied PgPool API bootstrap (P0 only)

## Base and artifacts

- Requested repository base: `d30f5a74320bdb3f1c250277ac9e2698585007bd`.
- The two source paths are unchanged between that base and the current documentation/test-only root commits at the time this draft was sealed.
- Patch: `/tmp/sagat-public-pool-bootstrap.patch`
  - SHA-256: `cd12615a8113f9e1a0d4d6a4c2de1bd36606eec24c74bef85519d1afaa2093a6`
  - `git apply --check`: passed against the shared worktree.
  - Stat: 2 files, 382 insertions, 13 deletions.
- Mirror: `/tmp/sagat-public-pool-bootstrap-draft/tests/support/api.rs`
  - SHA-256: `154548a466fbe2e0db5eb3b5400d0386dd501c6f7e109083d6d0bcc4a7f22f68`
- Mirror: `/tmp/sagat-public-pool-bootstrap-draft/tests/integration/workflow-primitives/api_reopen.rs`
  - SHA-256: `98db6fb1cdecc26bf9dd7faca29b5556bff4a78edbd32672e85bc87436cfd5a1`

No repository source file was written. No test, build, formatter, SQL session, container operation, commit, external write, or subagent was run for this draft. The only executed patch gate was the read-only `git apply --check`.

## Modified source paths

1. `tests/support/api.rs`
2. `tests/integration/workflow-primitives/api_reopen.rs`

No production path, migration, grant, proxy implementation, contract, or fixture outside those two files changes.

## Review unit 1: exact supplied-pool bootstrap

`start_on_db_with_pool(db, options, api_pool)` delegates into the same existing startup body as `start`, `start_with`, and `start_on_db`. The supplied pool becomes the single pool cloned into:

- the counting identity store and authenticated REST state;
- `PgRunStore`, execution services, governance services, and API admission;
- the public gRPC `WorkerService`;
- REST run/publishing/disclosure paths;
- approval and declared-operation stores; and
- the public `TestApi.api_pool`/execution handles.

Normal constructors retain their existing runtime/owner URL selection, eight-connection pool, five-second acquisition timeout, and `Options.statement_timeout` application.

The supplied constructor deliberately does not reinterpret `Options.runtime_role` or apply `Options.statement_timeout` to a pool it did not create. Its caller owns pool role, connection count, server session settings, and lifecycle.

## Review unit 2: actual public backend evidence

The added test is:

`api_reopen::supplied_single_connection_pool_backs_actual_public_completion_after_reopen`

The setup uses the existing owned disposable database and existing public fixture to persist one workflow, run, worker membership/binding/session, and original lease. After dropping the setup API, it uses `hold_server` plus `attach_existing_db`, and starts the new API with the existing unarmed `CommitAckLossProxy.pool`. The reopen path performs no migration, tenant/namespace provisioning, registration, grant, binding write, lease replacement, or ID recreation.

The caller explicitly configures finite 30-second `statement_timeout` and `idle_in_transaction_session_timeout` on the already-authenticated one-connection relay pool. The test then checks:

- pool maximum is exactly one;
- `current_database` is the exact owned database;
- `current_user` is `sagat_app`;
- application name is the existing relay application;
- both explicit session timeouts are present;
- durable tenant, namespace, membership, binding, sole worker session, original run, and original lease-qualified receipt scope survive unchanged; and
- current public JWT authentication performs fresh tenant/membership lookups.

A test-only BEFORE INSERT trigger is bound to the exact receipt primary-key identity: tenant, root lease-qualified scope, and original idempotency key. An owner transaction holds its advisory lock. The test captures the one proxied backend PID before the report, issues the exact original public `CompleteStepRequest`, and observes through the owner pool that the same PID is active, waiting behind the exact owner gate, and executing `INSERT INTO command_dedupe`. This is the public execution witness; an idle PID/equality check cannot satisfy it.

After releasing the gate, the public call must genuinely apply a valid 250,000-byte JSON output. The exact durable receipt is read, the exact original request is replayed publicly with `already_applied=true`, receipt identity/hash/result/timestamp remain unchanged, and worker authority remains unchanged.

The test finally settles the simple root workflow. This is bootstrap evidence only. It does not execute a scoped collection refusal, a rollback case, a COMMIT reply loss, or fresh refusal recovery.

## Mandatory unarmed relay cleanup

The relay remains unarmed for the entire P0 test. `CommitAckLossProxy::close()` is not used: that helper requires a resolved injected COMMIT fault and treats ordinary unarmed EOF as an invalid protocol resolution.

Cleanup order is explicit:

1. remove the exact test trigger/function using the owner pool;
2. finish public assertions;
3. drop the resumed public worker client;
4. drop the reopened API and its store/router/server pool references;
5. close the supplied pool under a five-second deadline; then
6. drop `CommitAckLossProxy`, leaving its existing abort-on-drop task guard unchanged.

No query is issued through the one-connection supplied pool while the public command owns it. All concurrent observations use the separate owner pool.

## Intended root commands

Before any build, use the repository machine guard:

```sh
df -h
export CARGO_TARGET_DIR="$(mktemp -d /tmp/sagat-public-pool-bootstrap-target.XXXXXX)"
```

Apply and inspect:

```sh
git apply --check /tmp/sagat-public-pool-bootstrap.patch
git apply /tmp/sagat-public-pool-bootstrap.patch
git diff -- tests/support/api.rs tests/integration/workflow-primitives/api_reopen.rs
```

Run the new focused bootstrap in both access invocations:

```sh
RUST_TEST_THREADS=2 cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  api_reopen::supplied_single_connection_pool_backs_actual_public_completion_after_reopen -- --nocapture
SAGAT_TEST_AS_RUNTIME_ROLE=1 RUST_TEST_THREADS=2 cargo test -p sagat-tests \
  --test integration_workflow_primitives --locked \
  api_reopen::supplied_single_connection_pool_backs_actual_public_completion_after_reopen -- --nocapture
```

Retain the existing reopen regression in both invocations:

```sh
RUST_TEST_THREADS=2 cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  api_reopen::current_api_reopens_owned_database_and_resumes_original_public_worker -- --nocapture
SAGAT_TEST_AS_RUNTIME_ROLE=1 RUST_TEST_THREADS=2 cargo test -p sagat-tests \
  --test integration_workflow_primitives --locked \
  api_reopen::current_api_reopens_owned_database_and_resumes_original_public_worker -- --nocapture
```

Then run the repository gates appropriate to the merged tree:

```sh
cargo fmt --check
cargo clippy -p sagat-tests --test integration_workflow_primitives --locked -- -D warnings
python3 scripts/check-architecture.py
python3 scripts/check-agent-config.py
```

Root should inspect the new test's same-PID/blocker observation and exact receipt/replay assertions before treating P0 as a prerequisite pass. A passing P0 only permits drafting/running the separately reviewed scoped refusal unit.

## Limitations and stop conditions

- This draft is source-level and unexecuted. It makes no passing-test, formatting, compilation, runtime, or release claim.
- The supplied pool API is test support only. It adds no production pool option or production behavior.
- The existing relay remains unchanged and unarmed. There is no fault injection or uncertain COMMIT claim here.
- The 250,000-byte completion proves that current authentication and the real public Worker stack can run through one supplied connection. It does not prove scoped refusal rollback, full refusal reopen, retained-output accounting, deferred abort, COMMIT acknowledgement loss, process restart, failover, or scheduler discovery.
- The gate is intentionally coupled to the current durable worker receipt encoding and the existing relay application name. A change to either must fail this fixture and be reviewed; the test must not fall back to PID identity alone.
- If the captured PID is not the same active receipt writer, the exact owner blocker is absent, the public completion cannot finish after release, the receipt is not stable on exact replay, or unarmed cleanup cannot close within its deadline, P0 is blocked. Do not reinterpret any of those failures as scoped-refusal evidence.

