# SDK and protocol compatibility

The five official SDKs report version `0.1.0`. They use the additive
`sagat-octopus-orchestrator.io/v1` workflow document and offer worker protocol `1.5` by default.
These versions describe different boundaries:

- REST clients call `/v1`; the worker protocol minor does not version those HTTP calls.
- Workflow documents stay on additive v1. Publication checks whether each capability is enabled.
- Workers negotiate the highest minor both sides can execute safely. The production PostgreSQL
  worker adapter accepts `1.5` and retains root-only negotiation through `1.4`.
- Protocol `1.5` adds scoped execution identity. Offering `1.5` does not bypass capability-specific
  publication gates or allow scoped work to fall back to an older worker.

| SDK | Package version | Default worker offer | Workflow document |
| --- | --- | --- | --- |
| TypeScript | 0.1.0 | 1.5 | additive v1 |
| Go | 0.1.0 | 1.5 | additive v1 |
| Python | 0.1.0 | 1.5 | additive v1 |
| .NET/C# | 0.1.0 | 1.5 | additive v1 |
| Java | 0.1.0 | 1.5 | additive v1 |

## Current public compatibility matrix

“Public” below means that the capability can be reached through the documented API without a
private database fixture or test-only adapter. A public route whose prerequisite resource cannot
yet be created publicly is called out separately.

| Capability | Current public state | SDK 0.1.0 client/builders | Worker 1.4 | Worker 1.5 | Evidence and remaining gate |
| --- | --- | --- | --- | --- | --- |
| Existing root workflows, run lifecycle, signals, events, timers, children and sagas | Public | Shared client and builder contract in all five SDKs | Executes root tasks without instance identity | Executes root tasks and echoes server-supplied identity | Shared runtime scenarios and the protocol-negotiation scenario |
| Ordered `switch` | Public | Builder and runtime calls covered in all five SDKs | Executes selected root activity tasks | Executes the same root tasks | Reviewed builder/runtime golden plus PostgreSQL fault tests |
| Declared state Query/Update, addressed by run | Public | Typed Query/Update clients in all five SDKs | No new worker message is required; ordinary activities remain root tasks | Same operation semantics | `state-operations` passed the shared real-PostgreSQL scenario |
| Approval decision | Public | Typed listing and decision clients in all five SDKs | No protocol dependency | No protocol dependency | `approval-decisions` passed the shared API/scheduler/PostgreSQL scenario in all five SDKs |
| Chain inspect, declared Query/Update, root signal and cancel | Public routes for restricted chains; direct continuation can transfer declared state | Typed chain Query/Update and `waitForChainCompletion` clients exist in all five SDKs; `chain-query-handoff` covers successor Query/Update, replay, stale-head fences and segment-versus-chain waits | No protocol dependency for the HTTP command | No protocol dependency for the HTTP command | Public chain creation, direct stateful handoff, shared `chain-inspect-cancel`, `chain-query`, `chain-query-handoff` and `chain-final-success` passed; focused HTTP/PostgreSQL tests cover Query and Update head fencing, rollover replay, RBAC, tenant opacity and audit. Broader race/fault matrix remains open |
| Collection, loop, approval-node and continuation authoring | Restricted runtime subsets for collection, loop, root approval and continuation | All five builders emit the same reviewed additive-v1 documents | Must never receive scoped work | May execute the restricted collection and loop bodies | `loop-runtime` passed all five SDKs for one pre-test and one post-test iteration; broader body graphs, delay/error routing and primitive combinations remain open |
| Scoped collection activity execution | Public for the restricted `forEach`/`map` Activity→Return body | Worker hosts expose typed identity and captured item/index, and echo the immutable lease identity on all lease-bound calls | Scoped tasks are withheld; identity and item/index are absent | Claims and reports scoped tasks with tenant and lease fencing; handler identity and item/index are present | PostgreSQL and fault tests cover the bounded activity slice; `scoped-identity` and mapped-result `collection-runtime` passed all five SDKs against reviewed common goldens |

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
2. A worker offering `1.5` to the production adapter is accepted at `1.5`. It echoes only the
   immutable identity supplied on the lease; that value does not replace session, tenant or lease
   authorization.
3. A scoped task is not flattened into a root task and is not leased through polling or streaming
   to a worker negotiated below `1.5`.
4. Production negotiation of `1.5` does not itself open structured workflow publication.

The shared `protocol-negotiation` scenario probes offers `1`, `1.0` through `1.5`, `1.9`, malformed
text and incompatible majors in every SDK. It also runs one root workflow through a current `1.4`
worker and through a worker offering `1.5`; both complete under the production adapter, with the
second worker reporting accepted protocol `1.5`. The legacy protocol-1 heartbeat case remains in the
same scenario so additive changes cannot silently remove the oldest accepted path.

The production store also has real-PostgreSQL tests that keep scoped candidates away from old workers,
validate the exact execution identity for reports, secrets and blobs, and reject tenant or lease-fence
mismatches. This is a fail-closed compatibility boundary, not a promise that all scoped node kinds are ready.

## Verified evidence

The compatibility golden must be reviewed for every protocol offer, negotiated worker version,
terminal run, attempt and worker log before it is retained.

| Check | Result |
| --- | --- |
| `SAGAT_SDK_SCENARIO=protocol-negotiation cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed against the production adapter in all five SDKs; 1.5 identity is present and the retained 1.4 path omits it |
| `SAGAT_SDK_SCENARIO=scoped-identity cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed without bless in all five SDKs; runtime golden `b89c6351…26d9`, workflow golden `b0a307dc…5318` |
| `SAGAT_SDK_SCENARIO=loop-runtime cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed without bless in all five SDKs; one `while` and one `repeatUntil` body completed under worker 1.5 with separate scoped paths |
| `SAGAT_SDK_SCENARIO=collection-runtime cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed without bless in all five SDKs; mapped results and two scoped paths reached a terminal graph under worker 1.5 polling |
| `cargo test -p sagat-application --lib scoped_identity_negotiation_is_gated_by_storage_support --locked` | Passed; stores without scoped support retain ceiling 1.4 and the production scoped store accepts 1.5 |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -- identity_storage::old_worker_candidate_query_does_not_offer_scoped_ready_steps --exact` | Passed against real PostgreSQL; a legacy claim did not receive the scoped candidate |

The item-aware TypeScript observation was reviewed before the same normalized result passed without
bless in all five languages. The fixture preserves duplicate equal items as distinct indices and a
third distinct value to prove that captured item bytes, not only identity paths, reach each handler.

## Release and rollback rules

Before public scoped workflow publication expands beyond the restricted Activity→Return body, the
five-worker scoped identity runtime scenario must remain green. The PostgreSQL, fault and SDK unit
gates must continue to cover polling and streaming, report replay, heartbeat, lease loss, secrets and
blobs.
The old-worker test must run concurrently with scoped-ready work and prove that it receives only
root-compatible tasks. Crash/restart and lost-broker tests must show that a scoped task remains
addressable and is never flattened after recovery.

Rollout opens primitive publication only after its feature-specific transaction, authorization,
fault and runtime-conformance gates pass. Rollback first closes new publication and starts, then
keeps the additive readers, chain/state APIs and compatible worker negotiation path until every run
that used the capability drains. An infrastructure skip or a build-only golden does not satisfy a
runtime compatibility gate.
