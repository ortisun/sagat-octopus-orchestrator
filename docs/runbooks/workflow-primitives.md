# Workflow primitives: operating and validating the current build

The public workflow document remains additive `sagat-octopus-orchestrator.io/v1`. Bounded
`switch` and run-addressed declared Query/Update are available with the existing basic root
nodes. Collection, loop, human approval and continuation documents still fail publication.
Their builder fixtures and private integration tests do not authorize publishing those nodes.
The worker adapter advertises minor 1.4; scoped 1.5 execution is enabled only in the test
harness. Keep these gates closed until each primitive's fault, security, compatibility and
recovery evidence is complete.

The scheduler does discover and advance collections already created through its internal durable
port. This is exercised with real PostgreSQL, restart and quota tests. The public parser still
refuses collection documents, and scoped work is never offered by the production 1.4 adapter.

## Check the running system

Run the contract and PostgreSQL suites from the repository root with the pinned toolchain:

```sh
cargo test -p sagat-tests --test contract_workflow_primitives --locked
cargo test -p sagat-tests --test integration_workflow_primitives --locked
cargo test -p sagat-tests --test fault_workflow_primitives --locked
```

The PostgreSQL test harness starts real infrastructure or fails if it cannot. A passing private
transaction test shows the asserted journal/projection/outbox boundary only; it is not evidence of
a public workflow round trip. See
[`validation-report.md`](../../specs/014-workflow-primitives/validation-report.md) for commands,
counts and remaining gates from the latest run.

For authoring parity, run the shared five-language SDK scenarios individually:

```sh
SAGAT_SDK_SCENARIO=collection-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=approval-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=continuation-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=state-operations cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
```

The four builder scenarios compare normalized documents with reviewed goldens and do not call
the server's runtime APIs. `state-operations` publishes a supported document and exercises
Query/Update through the real server in all five languages. Do not replace a golden merely
because an output changed: inspect the contract and intended compatibility behavior first.

## Diagnose a stuck or duplicated transition

Identify the tenant, namespace, run, workflow version, instance path and last journal sequence.
Inspect the run journal and projection under the same tenant scope; keep the trace ID and request
ID, but omit payloads, credentials and decision policy values from logs. The journal is the
authority. A mismatch with a step, timer, subscription, collection, loop or approval projection
means the transition must not be retried blindly. Preserve the run and investigate the failed
transaction or projection verifier.

For a worker task, compare the attempt and lease fence as well as the instance path. An old
worker minor must never receive a scoped task. A stale completion should leave journal, quota and
outbox unchanged. For a delayed loop or approval, verify that the due record is durable and no
worker lease is retained while waiting; duplicate delivery must produce one committed result.
Broker delivery is a hint, so use reconciliation of PostgreSQL state rather than assuming the
broker delivered exactly once. External business effects require stable idempotency keys or a
reconciliation procedure; neither a journal event nor an SDK retry makes an external effect
exactly once.

For declared Update, check the operation name, immutable definition version, expected state
revision, request hash and update key. `POST /runs/{runId}/updates/{operationName}` rechecks
current authorization even on receipt replay. A durable rejected receipt replays the same
business result. `POST /runs/{runId}/queries/{operationName}` reads one committed snapshot and
adds an access audit without appending a workflow event or scheduling work. Both responses must
carry `Cache-Control: no-store`; a foreign tenant's run remains opaque. A stale state revision
is a conflict, while a schema or declared-business rejection has a typed reason. If an outbox
write fails, the state, receipt and journal must all roll back before retrying the same update
key.

For chain commands, use `GET /namespaces/{namespaceId}/chains/{chainId}` to inspect the current
head. `POST` to the chain's `signals/{signalName}` or `cancel` path requires `expectedRunId` in
the body and `Idempotency-Key` in the header. `CHAIN_HEAD_CHANGED` means the caller should
inspect again before choosing a new command. Replaying the same key and body returns the same
receipt; a different body is a conflict. These endpoints can operate on an existing retained
chain, but a published workflow cannot yet create or hand off a chain. Exact-run commands remain
bound to their segment and do not forward silently.

## Rollback and compatibility

Disable publication and starts of any newly enabled primitive before rolling back its writer.
Keep readers, journal codecs, projections and worker capability gates for already active runs
until they drain. Do not remove migrations or delete chain/receipt rows while an active run or
retained idempotency receipt references them. Revert only after validating old active runs, old
worker minors and all five SDKs against the target build. The current PostgreSQL worker adapter
advertises minor 1.4 because scoped claim/report transactions are not yet enabled; its capability
gate must remain closed until those transactions and recovery tests pass.
