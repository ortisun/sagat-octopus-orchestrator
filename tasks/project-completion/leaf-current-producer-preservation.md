# Current flat ordinary-leaf producer preservation

Refs:T014-014/017/040; local B1b.2a.1d.3b. Source snapshot:1bb9eed6.
Scope: current binaries, fixed pinned OrdinaryLeafV1, already certified selected depth1 path.
This is a source inventory linked to existing actual tests; no new test execution is claimed.

## Mutation families and proof

| Family | Selected source exclusion or preservation | Actual positive barrier |
| --- | --- | --- |
| Admission and checkpoint | Require OPEN scope and absent/ADMITTED iteration; selected source is SUCCEEDED/CHECKPOINTED, no slot or remaining reservation. Replays do not mutate it. | Actual checkpoint then successful closure, both observed Run queue orders, producer replay: leaf-certification-closure-20261018. |
| Successful closure | Creation proof serializes after the actual original Run and current scheduler fence. A successful finish replay is metadata-only. | Same closure evidence; certifier-first refuses unclosed source, closure-first requires fresh CAS. |
| Drain/reservation | Require DRAINING/ADMITTED or positive remaining reservations; selected source has no drain and reserve0. | Later cancellation/failure exercises reachable other-owner planning in leaf-certification-later-work-20261018. |
| Exact addressed reports/steps | Exact tenant/run/namespace/node/UUID/path; reducers reject terminal routed selected nodes. Heartbeat needs live lease; expiry/retry/rehint discovery requires LEASED/RUNNING/RETRY_WAIT/READY. | Stale selected completion/failure, both orders: leaf-certification-late-reports-20261018. Accepted later completion/failure, both orders: leaf-certification-later-work-20261018. |
| Full Run/operator/scheduler | Persist only changed exact instances, append immutable journal and CAS Run head. Pause/resume/deadline/declared operations cannot rewrite selected source by changing Run metadata. | Actual READY cancellation and later terminal scheduling: leaf-certification-later-work-20261018. |
| Saga/reconciliation/compensation | Selected fixed-profile activities have no companions/effect policy. Saga header/entries are exact owner/path/index; reachable later companions cannot address selected source. | Actual later saga header/entry creation and compensation completion, fail/cancel and both observed planning orders: leaf-certification-saga-20261018. |
| Ambient binding | Broad writer has no Run lock but only expires LEASED/RUNNING steps belonging to revoked sessions. Selected source steps are SUCCEEDED without a live lease. | Actual replacement expires a later lease before proof and during gated certifier COMMIT: leaf-certification-binding-20261018. |
| Ambient dispatch/hints | Dispatch updates WAITING/EVENT only; fixed definition permits no waits/approval/loop/child controllers. Hint renewal discovers READY and writes an exact addressed instance. | Static exclusion for dispatch; addressed later-write family exercised above. |
| Identity/payload/journal | Actual version pin/admission identity immutable; payloads and existing journal rows immutable. Appends cannot change the five exact certified event keys. | Native proof and later legitimate Run/journal growth/replay in the above suites. |

## Source anchors

- `crates/infra-postgres/src/primitives.rs`: entry/admission/checkpoint/drain/finish lifecycle checks.
- `crates/infra-postgres/src/execution.rs`: consume_scoped_reservation, exact root/scoped steps,
  changed-instance persistence and exact scope saga headers/entries.
- `crates/infra-postgres/src/execution/path.rs`: exact addressed mutation/reservation writers.
- `crates/application/src/worker/scoped.rs`, PostgreSQL `maintenance.rs`: live heartbeat,
  retry/expiry/READY discovery predicates.
- PostgreSQL `bindings.rs`, `dispatch.rs`: broad lease expiry and WAITING/EVENT predicate.
- Domain `workflow/collection_shape.rs`: immutable fixed shape and activity companion exclusions.
- PostgreSQL `leaf_certification.rs`: complete selected step set, routed terminal state, absent
  lease/retry/wait, reserve0, exact five journal keys and selected saga absence.
- Migrations20261009000000/20261009000100/20261125000000/20261211000000:
  immutable version/journal/payload/admission identity and retained source lifetime.

## Acceptance boundary

The inventory covers distinct current mutation families. Heartbeat/expiry/retry/rehint/review and
unknown-effect reconciliation do not require a separate command-name race: they either cannot
select these certified rows or use the already exercised exact later-path writer. No independent
native race for those names is claimed. The shared oracle fingerprints selected scope, iteration,
checkpoint, owner, all selected local steps and selected-owner saga absence; actual positive later
work proves these are not merely idle no-op tests.

Closing current flat producer preservation does not close old binaries/mixed deployment C0,
chain-source acceptance, arbitrary privileged source UPDATE, nested relationship settlement,
production activation, release INSERT, discovery/fairness, source-retention compatibility or the
whole B1/source-writer gate. The certifier remains dormant and released SELECT-only.
