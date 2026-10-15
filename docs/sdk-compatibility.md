# SDK and protocol compatibility

The five official SDKs report version `0.1.0`. They use the additive
`sagat-octopus-orchestrator.io/v1` workflow document and offer worker protocol `1.4` by default.
These versions describe different boundaries:

- REST clients call `/v1`; the worker protocol minor does not version those HTTP calls.
- Workflow documents stay on additive v1. Publication checks whether each capability is enabled.
- Workers negotiate the highest minor both sides can execute safely. The production PostgreSQL
  worker adapter currently accepts at most `1.4`.
- Protocol `1.5` adds scoped execution identity. Offering `1.5` does not enable scoped workflow
  publication or allow scoped work to fall back to an older worker.

| SDK | Package version | Default worker offer | Workflow document |
| --- | --- | --- | --- |
| TypeScript | 0.1.0 | 1.4 | additive v1 |
| Go | 0.1.0 | 1.4 | additive v1 |
| Python | 0.1.0 | 1.4 | additive v1 |
| .NET/C# | 0.1.0 | 1.4 | additive v1 |
| Java | 0.1.0 | 1.4 | additive v1 |

## Current public compatibility matrix

“Public” below means that the capability can be reached through the documented API without a
private database fixture or test-only adapter. A public route whose prerequisite resource cannot
yet be created publicly is called out separately.

| Capability | Current public state | SDK 0.1.0 client/builders | Worker 1.4 | Worker offering 1.5 today | Evidence and remaining gate |
| --- | --- | --- | --- | --- | --- |
| Existing root workflows, run lifecycle, signals, events, timers, children and sagas | Public | Shared client and builder contract in all five SDKs | Executes root tasks | Negotiates 1.4 and executes the same root tasks | Shared runtime scenarios and the protocol-negotiation scenario |
| Ordered `switch` | Public | Builder and runtime calls covered in all five SDKs | Executes selected root activity tasks | Negotiates 1.4 | Reviewed builder/runtime golden plus PostgreSQL fault tests |
| Declared state Query/Update, addressed by run | Public | Typed Query/Update clients in all five SDKs | No new worker message is required; ordinary activities remain root tasks | Negotiates 1.4 | `state-operations` passed the shared real-PostgreSQL scenario; chain-addressed operations remain unavailable |
| Approval decision | Public route; no public workflow path opens the request yet | Typed decision client in all five SDKs | No protocol dependency | Negotiates 1.4 | Unit/API/PostgreSQL evidence exists; shared runtime scenario remains pending until approval creation is public |
| Chain inspect, root signal and cancel | Public routes; no public workflow path creates a chain yet | Typed chain clients in all five SDKs | No protocol dependency for the HTTP command | Negotiates 1.4 | Unit/API/PostgreSQL evidence exists; shared runtime scenario remains pending until chain creation is public |
| Collection, loop, approval-node and continuation authoring | Build-only | All five builders emit the same reviewed additive-v1 documents | Must never receive scoped work | Negotiates 1.4, so it also cannot receive scoped work | Publication/start remains closed; build-only or pending scenarios are not runtime evidence |
| Scoped collection activity execution | Test-only adapter path | No public runtime claim | Scoped tasks are withheld | A test-only store can explicitly negotiate 1.5 | Real PostgreSQL tests cover a bounded inline-activity slice; public publication, full node coverage and cross-language runtime evidence remain open |

The shared build-only scenarios compare canonical v1 documents from all five SDKs with reviewed
goldens. They prove authoring parity only. They do not prove that a document can be published or
executed. Likewise, a public approval or chain command route does not prove end-to-end usability
until the corresponding resource can be created through the public workflow path.

## Worker 1.4 and additive 1.5

Protocol `1.5` adds `ExecutionInstanceIdentity` to task leases and lease-bound calls. Root work has
an empty path; scoped work carries its definition node and ordered scope segments. The safety rule
is asymmetric:

1. A `1.4` worker continues to execute root work with the existing task, lease, heartbeat and report
   semantics.
2. A worker offering `1.5` to the production adapter is accepted at `1.4` and executes only work
   safe for that negotiated minor.
3. A scoped task is not flattened into a root task and is not leased through polling or streaming
   to a worker negotiated below `1.5`.
4. The test-only scoped adapter may negotiate `1.5`; that evidence does not change the production
   default or open structured workflow publication.

The shared `protocol-negotiation` scenario probes offers `1`, `1.0` through `1.5`, `1.9`, malformed
text and incompatible majors in every SDK. It also runs one root workflow through a current `1.4`
worker and through a worker offering `1.5`; both complete under the production adapter, with the
second worker reporting accepted protocol `1.4`. The legacy protocol-1 heartbeat case remains in the
same scenario so additive changes cannot silently remove the oldest accepted path.

The production store also has real-PostgreSQL tests that keep scoped candidates away from old workers
and reject scoped reports unless the store explicitly enables protocol `1.5`. This is a fail-closed
compatibility boundary, not a promise that all scoped node kinds are ready.

## Verified evidence

The compatibility golden was reviewed for every protocol offer, negotiated worker version, terminal
run, attempt and worker log before it was retained.

| Check | Result |
| --- | --- |
| `SAGAT_SDK_SCENARIO=protocol-negotiation cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed one shared scenario in TypeScript, Go, Python, C# and Java; 51.67 s |
| `cargo test -p sagat-application --lib scoped_identity_negotiation_is_gated_by_storage_support --locked` | Passed; production ceiling 1.4 and explicit scoped ceiling 1.5 remain distinct |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -- identity_storage::old_worker_candidate_query_does_not_offer_scoped_ready_steps --exact` | Passed against real PostgreSQL; a legacy claim did not receive the scoped candidate |

The reviewed runtime golden has SHA-256
`7e4309e73de7d6bb29679c9bdecfdb1b1493844593348d17e87fd2a6902c2ce1`.

## Release and rollback rules

Before changing the default SDK offer or the production adapter ceiling to `1.5`, all five workers
must pass the scoped identity scenario for polling and streaming, report replay, heartbeat, lease
loss, secrets and blobs. The old-worker test must run concurrently with scoped-ready work and prove
that it receives only root-compatible tasks. Crash/restart and lost-broker tests must show that a
scoped task remains addressable and is never flattened after recovery.

Rollout opens primitive publication only after its feature-specific transaction, authorization,
fault and runtime-conformance gates pass. Rollback first closes new publication and starts, then
keeps the additive readers, chain/state APIs and compatible worker negotiation path until every run
that used the capability drains. An infrastructure skip or a build-only golden does not satisfy a
runtime compatibility gate.
