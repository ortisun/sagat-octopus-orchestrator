# SDK and protocol compatibility

Chain clients, including the Python blocking facade, have current all-five evidence for
continuation authoring and seven chain runtime scenarios (T014-038). The exact scenario list,
reviewed-golden hashes and35 independently compared runtime artifacts are recorded in
`tasks/project-completion/evidence/chain-sdk-20261017.json`. Segment waits retain their exact-run
behavior; explicit chain waits follow the final outcome. Shared release gates remain open.

The five official SDKs report version `0.2.0`. They use the additive
`sagat-octopus-orchestrator.io/v1` workflow document and offer worker protocol `1.5` by default.
These versions describe different boundaries:

- REST clients call `/v1`; the worker protocol minor does not version those HTTP calls.
- Workflow documents stay on additive v1. Publication checks whether each capability is enabled.
- Workers negotiate the highest minor both sides can execute safely. The production PostgreSQL
  worker adapter uses the enabled capability ceiling: `1.5` for scoped identity and up to `1.6`
  with managed execution enabled. Ordinary root workers retain the `1.4` ceiling while scoped
  execution is disabled. Managed tasks separately require DatabaseRunner authority and `1.6`.
- Protocol `1.5` adds scoped execution identity. Offering `1.5` does not bypass capability-specific
  publication gates or allow scoped work to fall back to an older worker.

| SDK | Package version | Default worker offer | Workflow document |
| --- | --- | --- | --- |
| TypeScript | 0.2.0 | 1.5 | additive v1 |
| Go | 0.2.0 | 1.5 | additive v1 |
| Python | 0.2.0 | 1.5 | additive v1 |
| .NET/C# | 0.2.0 | 1.5 | additive v1 |
| Java | 0.2.0 | 1.5 | additive v1 |

## Current public compatibility matrix

“Public” below means that the capability can be reached through the documented API without a
private database fixture or test-only adapter. A public route whose prerequisite resource cannot
yet be created publicly is called out separately.

| Capability | Current public state | SDK 0.2.0 client/builders | Worker 1.4 | Worker 1.5 | Evidence and remaining gate |
| --- | --- | --- | --- | --- | --- |
| Existing root workflows, run lifecycle, signals, events, timers, children and sagas | Public | Shared client and builder contract in all five SDKs | Executes root tasks without instance identity | Executes root tasks and echoes server-supplied identity | Shared runtime scenarios and the protocol-negotiation scenario |
| Ordered `switch` | Public | Builder and runtime calls covered in all five SDKs | Executes selected root activity tasks | Executes the same root tasks | Reviewed builder/runtime golden plus PostgreSQL fault tests |
| Declared state Query/Update, addressed by run | Public | Typed Query/Update clients in all five SDKs | No new worker message is required; ordinary activities remain root tasks | Same operation semantics | `state-operations` passed the shared real-PostgreSQL scenario |
| Approval decision | Public; the route a decision or expiry chooses is released by the scheduler | Typed listing and decision clients in all five SDKs | No protocol dependency | No protocol dependency | `approval-decisions`, `approval-settlement` and `approval-security` passed the shared API/scheduler/PostgreSQL scenarios in all five SDKs |
| Chain inspect, declared Query/Update, root signal and cancel | Public routes for restricted chains; direct or conditional continuation can transfer exact declared state, and an activity may lead to a signal-controlled wait before input-only continuation | Typed chain Query/Update and `waitForChainCompletion` clients exist in all five SDKs; `chain-query-handoff` covers successor Query/Update, replay, stale-head fences and segment-versus-chain waits | No protocol dependency for the HTTP command | No protocol dependency for the HTTP command | Public chain creation, stateful handoff, shared `chain-inspect-cancel`, `chain-query`, `chain-query-handoff`, `chain-final-success`, `chain-state-final-success` and `chain-commands` passed; focused HTTP/PostgreSQL tests cover Query and Update head fencing, rollover replay, RBAC, tenant opacity and audit. Wait-entry and parent propagate/detach cases also have current all-five evidence; the broader shared release gates remain open |
| Collection, loop, approval-node and continuation authoring | Restricted runtime subsets for collection, loop, root approval and continuation | All five builders emit the same reviewed additive-v1 documents | Must never receive scoped work | May execute the restricted collection and loop bodies | `loop-runtime` and `loop-semantics` passed all five SDKs for pre-test, post-test, carry checkpoints, the iteration limit and a durable delay; `loop-error-activity` also covers failed scoped work draining into a successful root recovery activity; broader body graphs and primitive combinations remain open |
| Scoped collection activity execution | Public for a restricted root `forEach`/`map` body with activity entry, up to eight activity/condition/switch nodes and one `return` | Worker hosts expose typed identity and captured item/index, and echo the immutable lease identity on all lease-bound calls | Scoped tasks are withheld; identity and item/index are absent | Claims and reports scoped tasks with tenant and lease fencing; handler identity and item/index are present | PostgreSQL and fault tests cover the bounded activity slice; `scoped-identity` and mapped-result `collection-runtime` passed all five SDKs against reviewed common goldens |

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

The shared conformance fixtures for established root-only behavior pin protocol `1.4` explicitly.
Their reviewed worker logs intentionally lack an instance field. Primitive runtime fixtures use
`1.5` where they test scoped identity, while `protocol-negotiation` exercises both versions in one
scenario. This keeps old-worker compatibility observable after the SDK default advances.

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
| `SAGAT_SDK_SCENARIO=collection-ordering cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed without bless in all five SDKs; 17 items, concurrency 3, iteration 2 first and 1 and 0 last, duplicates kept at separate indices, aggregate in input order, never more than 3 active, empty list succeeds |
| `SAGAT_SDK_SCENARIO=loop-semantics cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed without bless in all five SDKs; false `while` runs no body, `repeatUntil` runs one, three carry checkpoints, `maxIterations` ends the run `FAILED`/`ITERATION_LIMIT`, a delay elapses while no worker runs |
| `SAGAT_SDK_SCENARIO=approval-settlement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed without bless in all five SDKs; expiry route, decision before and after the deadline, `ALREADY_DECIDED`, replay and changed-body conflict after the run ended, raw signals refused |
| `SAGAT_SDK_SCENARIO=approval-security cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed without bless in all five SDKs; initiator, viewer, worker, runner and revoked operator cannot decide, refusals carry the caller's trace and are audited, a secret-shaped input value appears nowhere |
| `cargo test -p sagat-application --lib scoped_identity_negotiation_is_gated_by_storage_support --locked` | Passed; stores without scoped support retain ceiling 1.4 and the production scoped store accepts 1.5 |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -- identity_storage::old_worker_candidate_query_does_not_offer_scoped_ready_steps --exact` | Passed against real PostgreSQL; a legacy claim did not receive the scoped candidate |

The scoped hosts key a task in flight by its task id and its instance path, because the legacy task
id (`run/node/generation`) is shared by iterations of one scope that are leased together. The Java host
used to drop the second lease as already in flight, the TypeScript host counted one slot for two; the
`collection-ordering` scenario (three iterations active at once) found the first, and both SDKs now have a
unit test for it.

The item-aware TypeScript observation was reviewed before the same normalized result passed without
bless in all five languages. The fixture preserves duplicate equal items as distinct indices and a
third distinct value to prove that captured item bytes, not only identity paths, reach each handler.

## Reviewed version and capability matrix (T014-041)

The original matrix was assembled on 2026-10-17 at commit `a49b228` plus the T014-041 tests.
The publication cells below are reconciled with the retained public scenarios on 2026-10-18;
the historical pre-014 reader probe and 44-case run remain explicitly dated. Every cell
names the test behind it, or says **not tested**. A cell is never inferred from a neighbouring one.
Goldens were not blessed (`SAGAT_SDK_BLESS` was not set in any run). The five SDKs run the same
scenarios and must give identical normalized results, so the SDK dimension is one row of results per
scenario (table 4), not five copies of the other tables: where a cell depends on an SDK, table 4 names it.

Actual tagged `0.1.0` SDK clients from release commit `6ede1c7849c4ac5a209c08da8ad25294fbdd7795`
have five-language lifecycle evidence against the combined server (see the archived-client section).
That archived-client run covers lifecycle only; other features and other client versions remain
untested.
Current SDKs offer `1.5` by default; negotiation down to an older server is covered separately
(table 1).

Test names are `file::name` in the `integration_workflow_primitives` target unless another target or
a scenario is named. The new T014-041 tests live in
`tests/integration/workflow-primitives/compatibility.rs` (`compatibility::`).

### 1. Worker protocol minor against the server

`scoped-capable` in this table is the production PostgreSQL adapter with managed execution
disabled, which negotiates up to `1.5`. Managed DatabaseRunner execution is a separate opt-in `1.6`
capability, covered by `database-connections`; these 1.5 cells do not describe that configuration.
`root-only` is a store
that has not declared scoped claim and report; it caps every offer at `1.4`.

| Worker offer | Agreed, scoped-capable adapter | Agreed, root-only adapter | Root tasks | Scoped collection/loop tasks | Run-input blob tasks | Task stream |
| --- | --- | --- | --- | --- | --- | --- |
| `1` and `1.0` | `1` (`compatibility::old_worker_minors_negotiate_and_are_never_offered_scoped_work`, scenario `protocol-negotiation`) | `1` (pure function: `worker::tests::the_newest_version_both_sides_speak_is_agreed`; adapter not tested) | Served, with heartbeats that carry no step identity (`protocol-negotiation`, worker `old`) | Never offered (same compat test) | Withheld (`protocol_compatibility::mixed_workers_only_lease_run_input_blobs_to_protocol_1_3` registers `1.2`; offer `1` not tested separately) | Refused `UNSUPPORTED_PROTOCOL`, poll still works (`protocol::task_stream_credit_is_validated_and_old_sessions_cannot_lease`, target `contract_grpc_durable_execution`) |
| `1.1`, `1.2` | `1.1`, `1.2` (compat test above, `protocol-negotiation`) | Not tested at adapter level | `1.2`: served (`mixed_workers_only_lease_run_input_blobs_to_protocol_1_3`); `1.1`: not tested with a live worker | Never offered (compat test) | `1.2`: withheld (`mixed_workers_only_lease_run_input_blobs_to_protocol_1_3`); `1.1`: not tested | Not tested for `1.1`/`1.2` explicitly; the `1` case above is the refusal that is tested |
| `1.3` | `1.3` (compat test, `protocol-negotiation`) | Not tested at adapter level | Served (`mixed_workers_only_lease_run_input_blobs_to_protocol_1_3`, current worker) | Never offered (compat test) | Leased (`mixed_workers_only_lease_run_input_blobs_to_protocol_1_3`) | Not tested for `1.3` explicitly |
| `1.4` | `1.4` (compat test, `protocol-negotiation`) | `1.4` (`compatibility::a_root_only_adapter_caps_a_newer_offer_at_1_4_and_withholds_scoped_work`) | Served (most root scenarios pin `1.4`, e.g. `run-lifecycle`, `switch-builders`) | Never offered, concurrently with a `1.5` worker that is leased the same tasks (compat test) | Leased (`blobs`, `run-input-blobs`) | Allowed (`protocol::a_task_stream_spends_credit_across_successive_frames`, scenario `worker-stream-faults`) |
| `1.5` | `1.5` (`scoped_worker::register_scoped`, `protocol-negotiation`) | Capped at `1.4`, no scoped task (`compatibility::a_root_only_adapter_caps_a_newer_offer_at_1_4_and_withholds_scoped_work`) | Served and the server identity is echoed (`protocol-negotiation`, `scoped-identity`) | Leased with tenant and lease fencing (compat test positive control, `collection-runtime`, `loop-runtime`) | Leased (same code path as `1.4`; not tested separately for `1.5`) | Allowed (not tested separately for `1.5`) |
| `1.99`, any newer minor of major 1 | Agreed at the server's `1.5` (`worker::tests::the_newest_version_both_sides_speak_is_agreed`) | Agreed at `1.4` (compat test, offer `1.99`) | as the agreed minor | as the agreed minor | as the agreed minor | as the agreed minor |
| `2`, `0.9`, `1.x`, `1.05`, empty | Refused `UNSUPPORTED_PROTOCOL` (compat test for the root-only adapter; scenario `protocol-negotiation` probes malformed and incompatible offers through the production adapter in all five SDKs) | Refused (compat test) | none | none | none | none |

### 2. Workflow-document capability against reader, writer and worker

IR is additive v1. "Publication" is what `JsonIrCodec::parse` (the publication gate) accepts on this
build. "Pre-014 reader" is the real build at commit `07d673f`, the last commit before any primitive
code: its `JsonIrCodec::parse` was run on the nine documents of the SDK builder goldens
(`collection-builders`, `loop-builders`, `approval-builders`, `switch-builders`,
`state-operation-builders`, `continuation-builders`) and on the eight pre-014 documents of
`compatibility.rs`. That probe is a manual procedure (check out the commit, add a test that calls
`parse`), not a test in this tree; its output is quoted below. The automated tests inject a codec that
returns the same `Schema` error where production injects its codec (`DefinitionCache`).

| IR feature | Publication on this build | Worker `1.0`–`1.4` | Worker `1.5` | Pre-014 reader (build `07d673f`) |
| --- | --- | --- | --- | --- |
| Only pre-014 nodes (activity, timer, waitEvent, condition, parallel, child, saga) | Publishes with the same canonical bytes and digest as the pre-014 build (`compatibility::documents_with_only_pre_primitive_nodes_validate_identically_in_every_reader`, `compatibility::pre_primitive_documents_keep_the_digest_the_pre_014_build_computed`) | Runs (`run-lifecycle`, `saga-compensation`, `signals-and-events`, scenarios pinned to `1.4`) | Runs (`protocol-negotiation`) | Accepted; the eight digests are identical to this build's |
| Run started before the primitives | Not applicable | Finishes on the new engine (`compatibility::an_active_pre_014_run_survives_the_migrations_and_finishes_on_the_new_engine`) | not tested (the run is root-only; no `1.5` worker is part of the test) | Not applicable |
| `forEach` collection | Restricted subset: an optional ordinary root source-activity prefix, then a collection chain ending at `end`, body of 1 to 8 activities then `return`; `collection-step-source` covers a committed activity result and `collection-blob-source` its owned blob. No `triggers` or `chainPolicy`. An activity after a collection remains refused with `UNSUPPORTED_NODE` (`compatibility::combinations_the_reader_does_not_enable_are_refused_with_a_typed_reason_and_store_nothing`: collection followed by activity, trigger, chain policy; `collections_http::publication_keeps_unproven_collection_bodies_closed` on the wire) | Iteration tasks withheld, the run waits and is not failed (`compatibility::old_worker_minors_negotiate_and_are_never_offered_scoped_work`) | Executes (`collection-runtime`, `collection-ordering`, `collection-compensation`) | Refused `SCHEMA`: `unknown node type 'forEach'`, `Additional properties are not allowed ('bodies' was unexpected)`. An already running run is left untouched by a reader that cannot parse it (`compatibility::a_reader_without_the_capability_leaves_an_active_structured_run_untouched_until_it_returns`); the poll answers `Internal` (G460) |
| `while` / `repeatUntil` loop | Restricted subset: a loop chain ending at `end`, same activity/return body rule; a configured error edge may drain failed scoped work and route to a root recovery activity (`loop-error-activity`). Broader shapes remain refused (`loops_http::publication_keeps_unproven_loop_bodies_closed`) | Same claim gate as collections; **not tested with a loop run** | Executes (`loop-runtime`, `loop-semantics`, `loop-compensation`) | Refused `SCHEMA`: `unknown node type 'while'` and `'repeatUntil'` |
| `approval` node | Publishes alone. With `parallel`: refused (`compatibility::combinations_the_reader_does_not_enable_...`, kind `parallel`). With declared state: publishes since D295 (`approval_combinations::an_approval_with_declared_state_publishes_and_the_decision_routes_the_run`, scenario `approval-state`); in a collection or loop body: refused `UNSUPPORTED_NODE` with `details.dependency = SCOPED_COMPENSATION` | The approval node produces no worker task; a downstream activity under an old worker: **not tested** | **not tested** with a worker | Refused `SCHEMA`: `unknown node type 'approval'` |
| `switch` | Publishes (`switch-builders`) | Executes selected root tasks (`switch-builders` pins `1.4`) | **not tested** | Refused `SCHEMA`: `unknown node type 'switch'` |
| Declared `state` with Query/Update | Publishes as a plain run or as a direct continuation that transfers the state; with `approval` it publishes (D295, `approval-state`) | State operations have no worker message: **not tested** with a worker (`state-operations` declares no worker) | **not tested** with a worker | Refused `SCHEMA`: `Additional properties are not allowed ('state' was unexpected)` |
| Chains (`chainPolicy` + `continueAsNew`) | Restricted shapes: activity entry followed by direct, conditional or event-wait continuation; public wait entry may continue input-only directly or through one condition choosing continuation/end (`chain-root-wait`). With `bodies` or `triggers` refused in `parse_published`; with `forEach` refused (`compatibility::...`, chain-policy case; `continuation::publication_rejects_unproven_continuation_combinations`, `continuation::publication_keeps_other_waiting_continuation_shapes_closed` in `sagat-infra-ir`) | **not tested** | Executes all chain scenarios (`chain-three-segments`, `chain-final-success`, `chain-query-handoff`, `chain-child`, ...) | Refused `SCHEMA`: `unknown node type 'continueAsNew'`, `'chainPolicy'` and `'state'` unexpected |

Probe output of the pre-014 build (one line per document, trimmed): `approval--default-exclusion`,
`approval--role-and-subject`: `unknown node type 'approval'`; `collection--for-each`, `collection--map`:
`unknown node type 'forEach'`, `'bodies' was unexpected`; `continuation--input-only`,
`continuation--input-and-state-maximum`: `unknown node type 'continueAsNew'`, `'chainPolicy'` (and `'state'`)
unexpected; `loop--pre-and-post-test`: `unknown node type 'while'`, `'repeatUntil'`;
`state-operation--decimal-and-utc`: `'state' was unexpected`; `switch--ordered`: `unknown node type 'switch'`.
All nine were refused with `IrError::Schema`, which the API reports as `details.reason = SCHEMA` with the
issue paths. Nothing was stored by the probe.

### 3. Upgrade and rollback behaviours

| Situation | What happens | Test |
| --- | --- | --- |
| Active pre-014 run, schema migrated forward | Run, pinned version and journal prefix unchanged; finishes on the new engine; consistent replay | `compatibility::an_active_pre_014_run_survives_the_migrations_and_finishes_on_the_new_engine` and `identity_storage::upgrading_populated_storage_keeps_root_rows_and_does_not_rewrite_the_journal` |
| New schema, old worker minor polls while scoped work is ready | Old worker receives nothing and leaves no lease or attempt on the scoped rows | `compatibility::old_worker_minors_negotiate_and_are_never_offered_scoped_work`, `identity_storage::old_worker_candidate_query_does_not_offer_scoped_ready_steps` |
| Publication of a combination the runtime has not enabled | Typed `UNSUPPORTED_NODE` (`Unsupported{kind}` or `UnsupportedDeclaration{field}`); no version row is written | `compatibility::combinations_the_reader_does_not_enable_are_refused_with_a_typed_reason_and_store_nothing` |
| Rollback to a reader that cannot parse a pinned structured definition | Run row, steps and journal are byte-for-byte unchanged; nothing is leased; the poll fails `Internal` (G460); the capable reader resumes the same run | `compatibility::a_reader_without_the_capability_leaves_an_active_structured_run_untouched_until_it_returns` |
| Roll back with a `1.5` worker connected to an adapter that no longer negotiates it | Capped at `1.4`, scoped task withheld | `compatibility::a_root_only_adapter_caps_a_newer_offer_at_1_4_and_withholds_scoped_work` |

### 4. Historical 44-case five-SDK conformance results — 2026-10-17

Command, run from the repository root with no environment switch other than the build directory
(`SAGAT_SDK_BLESS` is not set, goldens are read-only):

```sh
cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
```

**Final run of that historical snapshot, 2026-10-17 07:08 to 07:21 (790 s, host load average 6 to 8): 44 scenarios, five languages
each, 220 scenario runs, all passed**, normalized results identical across the five languages and equal to
the reviewed goldens of that snapshot. At that time `chain-root-wait.json.pending` was excluded.
It has since become a registered public scenario with retained all-five SDK58/59 results; the
historical exclusion does not describe the current publication path.

**First full run, same commit, 05:27 to 05:48 (1,256 s) with the host at load average 30 to 47 because of other
agents' work: three scenarios failed.** `run-input-blobs` (Java deadline `createdAt+59s` against `+60s`),
`scoped-identity` (Go: `the worker log has 3 entries, not 6`; Python: status `RUNNING` against `WAITING`) and
`secrets` (TypeScript and Go: event 3 was `step.completed` where the golden has `step.started`). Each
passed when rerun alone minutes later (17 s, 42 s and 22 s, `SAGAT_SDK_REUSE=1 SAGAT_SDK_SCENARIO=<name>`),
and the final full run above passed all of them. They look like observation races under load; the cause
was not investigated (G467), and the failures are reported rather than hidden.

| Scenario | TypeScript | Go | Python | C# | Java |
|---|---|---|---|---|---|
| `approval-builders` | pass | pass | pass | pass | pass |
| `approval-decisions` | pass | pass | pass | pass | pass |
| `approval-security` | pass | pass | pass | pass | pass |
| `approval-settlement` | pass | pass | pass | pass | pass |
| `blobs` | pass | pass | pass | pass | pass |
| `builders` | pass | pass | pass | pass | pass |
| `cancellation` | pass | pass | pass | pass | pass |
| `chain-child` | pass | pass | pass | pass | pass |
| `chain-commands` | pass | pass | pass | pass | pass |
| `chain-final-success` | pass | pass | pass | pass | pass |
| `chain-inspect-cancel` | pass | pass | pass | pass | pass |
| `chain-query-handoff` | pass | pass | pass | pass | pass |
| `chain-query` | pass | pass | pass | pass | pass |
| `chain-state-final-success` | pass | pass | pass | pass | pass |
| `chain-three-segments` | pass | pass | pass | pass | pass |
| `collection-builders` | pass | pass | pass | pass | pass |
| `collection-compensation` | pass | pass | pass | pass | pass |
| `collection-ordering` | pass | pass | pass | pass | pass |
| `collection-runtime` | pass | pass | pass | pass | pass |
| `continuation-builders` | pass | pass | pass | pass | pass |
| `errors` | pass | pass | pass | pass | pass |
| `failures-and-retries` | pass | pass | pass | pass | pass |
| `lease-lost` | pass | pass | pass | pass | pass |
| `loop-builders` | pass | pass | pass | pass | pass |
| `loop-compensation` | pass | pass | pass | pass | pass |
| `loop-runtime` | pass | pass | pass | pass | pass |
| `loop-semantics` | pass | pass | pass | pass | pass |
| `lost-responses` | pass | pass | pass | pass | pass |
| `protocol-negotiation` | pass | pass | pass | pass | pass |
| `run-input-blobs` | pass | pass | pass | pass | pass |
| `run-input-early-success` | pass | pass | pass | pass | pass |
| `run-lifecycle` | pass | pass | pass | pass | pass |
| `saga-compensation` | pass | pass | pass | pass | pass |
| `scoped-identity` | pass | pass | pass | pass | pass |
| `secrets` | pass | pass | pass | pass | pass |
| `serialization` | pass | pass | pass | pass | pass |
| `signals-and-events` | pass | pass | pass | pass | pass |
| `state-operation-builders` | pass | pass | pass | pass | pass |
| `state-operations` | pass | pass | pass | pass | pass |
| `switch-builders` | pass | pass | pass | pass | pass |
| `worker-registration` | pass | pass | pass | pass | pass |
| `worker-shutdown` | pass | pass | pass | pass | pass |
| `worker-stream-faults` | pass | pass | pass | pass | pass |
| `worker-transport-faults` | pass | pass | pass | pass | pass |

`pass` means the language produced the normalized result of the golden.

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


## Actual v0.1.0 client compatibility on the combined branch

Released SDK source commit `6ede1c7849c4ac5a209c08da8ad25294fbdd7795` passed the reviewed current
`run-lifecycle` scenario in TypeScript, Go, Python, C# and Java against the real current platform,
without blessing: one aggregate / five languages, 145.91 s. These are clients compiled from the
actual tagged source and its own shared contracts. `scripts/test-released-sdk-compat.sh` archives
those inputs and selects them through `SAGAT_SDK_SOURCE_ROOT`; current scenarios/goldens and server
semantics are retained. This focused old-client proof does not close T014-041's complete current
primitive/runtime matrix. Full source identity, commands, manifests, process upgrade and old Studio
Chromium evidence are in [the compatibility report](../tests/integration/payload-classification/compatibility-report.md).

## Retained 58-case aggregate and native upgrade refresh — 2026-10-18

The full current-client suite passed all 58 registered scenarios in TypeScript, Go, Python, C# and
Java in 1055.10 s (harness 1054.80 s), using actual sagat_app runtime connections and five preparation
scripts without SAGAT_SDK_REUSE, language/scenario/source-root selection or blessing. All 1238 tracked
Rust/SDK/shared-contract inputs retained identical before/after hashes. A documentary-only binding
commit during execution changed none of those inputs; execution base is dc212855.

This is 290 language/scenario plays, including 30 build-only plays and 5 publication-only plays;
255 plays perform live workflow/protocol work. Authoring is compared separately across languages
and to reviewed canonical documents. All 260 runtime/publication normalized artifacts were retained
and independently compared as complete JSON documents to the exact reviewed golden. Each language
runs on an independent real platform/database. No build-only depth8 case implies nested execution.

The 8 native `compatibility::` tests also passed in 15.02 s (harness 12.53 s) as the runtime role. They
refresh the existing old-schema active-run migration/pin/journal test, old-worker minor withholding
with current-worker positive control, unchanged pre-primitive documents/digests, unsupported-reader
refusal, preservation of an active unsupported run and fairness among compatible candidates. These
native fixtures are not presented as a newly executed historical engine binary. The separate
v0.1.0 archived-client result above remains its own previously retained evidence.

Current aggregate evidence, exact commands, toolchains, source hashes and complete normalized
artifacts: [sdk58-current-20261018](../tasks/project-completion/evidence/sdk58-current-20261018/reproduce.md).
Broader nested capacity, C0/source-producer preservation, resource ceilings and release gates stay
separate. The table reports only each scenario's declared phase and the observed five SDK result.

| Scenario | Phase | TypeScript | Go | Python | C# | Java |
| --- | --- | --- | --- | --- | --- | --- |
| `approval-builders` | authoring only | pass | pass | pass | pass | pass |
| `approval-chain` | authoring/live protocol | pass | pass | pass | pass | pass |
| `approval-decisions` | authoring/live protocol | pass | pass | pass | pass | pass |
| `approval-security` | authoring/live protocol | pass | pass | pass | pass | pass |
| `approval-settlement` | authoring/live protocol | pass | pass | pass | pass | pass |
| `approval-state` | authoring/live protocol | pass | pass | pass | pass | pass |
| `blobs` | authoring/live protocol | pass | pass | pass | pass | pass |
| `builders` | authoring/publication | pass | pass | pass | pass | pass |
| `cancellation` | authoring/live protocol | pass | pass | pass | pass | pass |
| `chain-cancel-parent` | authoring/live protocol | pass | pass | pass | pass | pass |
| `chain-child` | authoring/live protocol | pass | pass | pass | pass | pass |
| `chain-commands` | authoring/live protocol | pass | pass | pass | pass | pass |
| `chain-final-success` | authoring/live protocol | pass | pass | pass | pass | pass |
| `chain-inspect-cancel` | authoring/live protocol | pass | pass | pass | pass | pass |
| `chain-query-handoff` | authoring/live protocol | pass | pass | pass | pass | pass |
| `chain-query` | authoring/live protocol | pass | pass | pass | pass | pass |
| `chain-rerun` | authoring/live protocol | pass | pass | pass | pass | pass |
| `chain-root-wait` | authoring/live protocol | pass | pass | pass | pass | pass |
| `chain-state-final-success` | authoring/live protocol | pass | pass | pass | pass | pass |
| `chain-three-segments` | authoring/live protocol | pass | pass | pass | pass | pass |
| `classification` | authoring only | pass | pass | pass | pass | pass |
| `collection-blob-output` | authoring/live protocol | pass | pass | pass | pass | pass |
| `collection-blob-source` | authoring/live protocol | pass | pass | pass | pass | pass |
| `collection-builders` | authoring only | pass | pass | pass | pass | pass |
| `collection-compensation` | authoring/live protocol | pass | pass | pass | pass | pass |
| `collection-limits` | authoring/live protocol | pass | pass | pass | pass | pass |
| `collection-ordering` | authoring/live protocol | pass | pass | pass | pass | pass |
| `collection-runtime` | authoring/live protocol | pass | pass | pass | pass | pass |
| `collection-step-source` | authoring/live protocol | pass | pass | pass | pass | pass |
| `continuation-builders` | authoring only | pass | pass | pass | pass | pass |
| `database-connections` | authoring/live protocol | pass | pass | pass | pass | pass |
| `errors` | authoring/live protocol | pass | pass | pass | pass | pass |
| `failures-and-retries` | authoring/live protocol | pass | pass | pass | pass | pass |
| `lease-lost` | authoring/live protocol | pass | pass | pass | pass | pass |
| `loop-builders` | authoring only | pass | pass | pass | pass | pass |
| `loop-cancel` | authoring/live protocol | pass | pass | pass | pass | pass |
| `loop-compensation` | authoring/live protocol | pass | pass | pass | pass | pass |
| `loop-error-activity` | authoring/live protocol | pass | pass | pass | pass | pass |
| `loop-runtime` | authoring/live protocol | pass | pass | pass | pass | pass |
| `loop-semantics` | authoring/live protocol | pass | pass | pass | pass | pass |
| `lost-responses` | authoring/live protocol | pass | pass | pass | pass | pass |
| `protocol-negotiation` | authoring/live protocol | pass | pass | pass | pass | pass |
| `run-input-blobs` | authoring/live protocol | pass | pass | pass | pass | pass |
| `run-input-early-success` | authoring/live protocol | pass | pass | pass | pass | pass |
| `run-lifecycle` | authoring/live protocol | pass | pass | pass | pass | pass |
| `saga-compensation` | authoring/live protocol | pass | pass | pass | pass | pass |
| `scoped-identity` | authoring/live protocol | pass | pass | pass | pass | pass |
| `secrets` | authoring/live protocol | pass | pass | pass | pass | pass |
| `serialization` | authoring/live protocol | pass | pass | pass | pass | pass |
| `signals-and-events` | authoring/live protocol | pass | pass | pass | pass | pass |
| `state-operation-builders` | authoring only | pass | pass | pass | pass | pass |
| `state-operations` | authoring/live protocol | pass | pass | pass | pass | pass |
| `switch-builders` | authoring/live protocol | pass | pass | pass | pass | pass |
| `worker-registration` | authoring/live protocol | pass | pass | pass | pass | pass |
| `worker-shutdown` | authoring/live protocol | pass | pass | pass | pass | pass |
| `worker-stream-faults` | authoring/live protocol | pass | pass | pass | pass | pass |
| `worker-transport-faults` | authoring/live protocol | pass | pass | pass | pass | pass |
| `workflow-instance-limits` | authoring/live protocol | pass | pass | pass | pass | pass |

At that 58-case checkpoint, bounded acceptance review kept T014-041 open: shared WP-13 expression/list boundaries and
unsupported object literals, plus WP-12 switch cases at 32/excess 33, still need five-SDK coverage.
The successful current aggregate is retained without claiming those absent scenarios.

### Switch 32/33-case focused refresh

After the full aggregate, `switch-builders` was extended and rerun in all five SDKs: 32 cases
publish/execute; 33 fail public builder validation. Focused aggregate passed in 28.14 s with
unchanged complete runtime golden and reviewed canonical/error fixtures. Evidence:
[switch case boundary](../tasks/project-completion/evidence/sdk-switch-case-boundary-20261018/reproduce.md).
The previously recorded full aggregate remains its own earlier snapshot; WP-13 gaps stay open.

### Expression limits focused refresh

The new `expression-limits` scenario passed in all five SDKs in 28.46 s (harness 28.13 s), with
reviewed canonical/runtime goldens and no blessing. It separates shared HTTP publication refusals
from public SDK query execution: exact 1,024 UTF-8 bytes, literals at 64/excess 65, constructed
lists at 4,096/excess 4,097 and unsupported object literals. Refused publication leaves no version;
a refused query preserves Run/state revisions. The ordinary `state-operations` regression also
passed in all five in 19.05 s with its unchanged complete golden. Registry preflight passed.
Evidence: [expression limits](../tasks/project-completion/evidence/sdk-expression-limits-20261018/reproduce.md).
This is focused evidence for the 59th registered case, not a new full 59-case aggregate.

## Retained complete 59-case SDK aggregate — 2026-10-18

The complete current matrix passed in all five SDKs in 1,142.10 s: 59 scenarios, 295 language/case
plays, including 30 authoring-only, 5 publication-only and 260 live workflow/protocol plays.
All 265 complete normalized outputs equal the exact reviewed golden, with every 53 server-phase
case/language pair present once and all 1,241 frozen input hashes unchanged. No blessing, reuse,
archived source or scenario/language filtering. The source base is bea961c2; a later documentary
schema contract changed none of those inputs. The earlier eight native compatibility tests retain
identical executable inputs and a separately recorded conformance documentation delta.

At that 59-case checkpoint, public-contract mapping left T014-041 open for active sibling drain after failure,
Query result-schema rejection with unchanged revisions, and successful upper bounds of 32 approval
principals/queries/updates. These are separate from the open nested/C0/resource gates. Drafts for
those public observations are outside the tested snapshot and cannot be counted as passing yet.

Evidence: [complete SDK59 aggregate](../tasks/project-completion/evidence/sdk59-current-20261018/reproduce.md).

### Final three public observations: focused all-five refresh

Registry now contains 60 cases. Approval principal boundary passed in all five (16.11 s,
authoring-only); expression declaration/schema observations passed in all five (20.38 s); new
active sibling drain passed in all five (22.33 s). All ten complete live outputs equal reviewed
goldens. No blessing or runner/server change. At that focused checkpoint, a complete current 60-case aggregate remained a
separate integration gate; it is completed below. Evidence: [public contract gaps](../tasks/project-completion/evidence/sdk-public-gaps-20261018/reproduce.md).


## Complete current 60-case SDK acceptance — 2026-10-18

T014-041 closes on the complete, unfiltered current five-SDK aggregate: 60 scenarios, 300
language/scenario plays, 30 authoring-only, 5 publication-only and 265 live workflow/protocol.
All 270 complete normalized outputs independently equal the reviewed golden; all 1,672 frozen
inputs are unchanged. The aggregate passed in 1,067.94 s (harness 1,067.57 s), without blessing,
prepared-runner reuse or archived source. Eight actual native compatibility tests pass on
identical executable inputs, covering active-old-run and old-worker-minor compatibility.

The first full attempt failed during managed-secret PUT with an unexplained 503, before the
five database scenario outputs. Its same-source focused retry and this subsequent full pass
are retained separately; no inferred provider cause or speculative fix is claimed. The third
and final supported-public mapping review found no further missing observation after the
last focused fixtures; stale matrix wording was corrected. Actual tagged v0.1.0 lifecycle
coverage remains separately scoped, with other historical features/versions untested.

Evidence: [complete SDK60 aggregate](../tasks/project-completion/evidence/sdk60-current-20261018/reproduce.md).
This evidence does not accept general nested runtime, C0, production ceilings or final release gates.

| Scenario | Phase | TypeScript | Go | Python | C# | Java |
| --- | --- | --- | --- | --- | --- | --- |
| `approval-builders` | authoring-only | pass | pass | pass | pass | pass |
| `approval-chain` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `approval-decisions` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `approval-security` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `approval-settlement` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `approval-state` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `blobs` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `builders` | publication-only | pass | pass | pass | pass | pass |
| `cancellation` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `chain-cancel-parent` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `chain-child` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `chain-commands` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `chain-final-success` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `chain-inspect-cancel` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `chain-query-handoff` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `chain-query` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `chain-rerun` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `chain-root-wait` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `chain-state-final-success` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `chain-three-segments` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `classification` | authoring-only | pass | pass | pass | pass | pass |
| `collection-blob-output` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `collection-blob-source` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `collection-builders` | authoring-only | pass | pass | pass | pass | pass |
| `collection-compensation` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `collection-fail-fast-drain` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `collection-limits` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `collection-ordering` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `collection-runtime` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `collection-step-source` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `continuation-builders` | authoring-only | pass | pass | pass | pass | pass |
| `database-connections` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `errors` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `expression-limits` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `failures-and-retries` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `lease-lost` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `loop-builders` | authoring-only | pass | pass | pass | pass | pass |
| `loop-cancel` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `loop-compensation` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `loop-error-activity` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `loop-runtime` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `loop-semantics` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `lost-responses` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `protocol-negotiation` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `run-input-blobs` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `run-input-early-success` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `run-lifecycle` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `saga-compensation` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `scoped-identity` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `secrets` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `serialization` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `signals-and-events` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `state-operation-builders` | authoring-only | pass | pass | pass | pass | pass |
| `state-operations` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `switch-builders` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `worker-registration` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `worker-shutdown` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `worker-stream-faults` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `worker-transport-faults` | live-workflow-or-protocol | pass | pass | pass | pass | pass |
| `workflow-instance-limits` | live-workflow-or-protocol | pass | pass | pass | pass | pass |


## 0.3 focused collection-recovery checkpoint

`collection-local-recovery` uses the existing activity `onError` option, additive IR v1
and worker minor 1.5 in all five SDKs. Initial observation produced five identical complete
normalized outputs; the aggregate correctly failed because the two new reference files
were absent. The workflow documents, ten worker calls, original compensation target,
ordered `[1, 2]` result, `RECOVERY_FAILED` drain and concurrency bound were inspected before
recording those files. The subsequent all-five run without a bless switch passed in
36.39 s. Native tests separately exercise selective recovery, sibling isolation, exact
lifetime retirement, rejected handled-error drains and full/frontier journal equivalence.
This focused scenario adds to the historical 60-scenario evidence; it is not a new complete
61-scenario release run. The final recovery rerun after the native drain/saga corrections
also passed all five in41.59s. Nested capacity and production ceilings remain pending.

## 0.3 focused local-control checkpoint

`collection-control-routing` uses the existing condition/switch body builders and worker
minor1.5 in all five SDKs. An activity entry is followed by a local decision: positive items
notify, negative items return directly. The two Runs both yield `[1,-1]`; six actual worker
calls, two recorded choices per Run, qualified graphs and skipped index1 notifications are
identical across languages. The authored12-instance/depth1 ceiling remains in both reviewed
documents and their canonical forms.

The initial all-five observation failed only because its two new reference files did not
exist. After reading the complete business observations and matching normalized outputs,
the references were recorded and the all-five no-bless verification passed1 aggregate,
0 failures,43.02s test execution. This raises the scenario registry to62; it is not a new
complete62-scenario acceptance run. Native tests separately prove failures, local recovery,
rollback, malformed state and prepared-decision CAS. Control entry, waits, children and
nested scopes remain outside the accepted public body subset.
