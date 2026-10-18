# B1b.2b.10 scoped hint vocabulary draft

Base: `4f3b6ac0ead1162151e9c909d0de63331f63925f` in
`/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`.
The repository was read only. Its pre-existing untracked
`tasks/project-completion/evidence/sdk60-current-20261018/` directory was not touched.

## Four source-file drafts

- `specs/001-platform-foundation/contracts/events.asyncapi.yaml`
  - Adds only `capacityScopedMemberFinished`,
    `publishCapacityScopedMemberFinished`, and `CapacityScopedMemberFinished`.
  - Address/event type: `sagat.v1.capacity.scoped_member.finished.{tenant}` /
    `capacity.scoped_member.finished`.
  - Closed payload contains the ten required scoped identity/result fields and optional nullable,
    lower-hex `traceId`; it contains no chain/head, payload, credentials, operation key, or status.
- `specs/001-platform-foundation/contracts/events.baseline.json`
  - Hand-authored one new `CapacityScopedMemberFinished` entry; no bless/regeneration command ran.
- `tests/integration/workflow-primitives/hint_contract.rs`
  - Retains `capacity_finish_hint_is_closed_control_metadata` unchanged.
  - Enables UUID format checks in the existing hint validator.
  - Adds `capacity_scoped_finish_hint_accepts_the_closed_reference_payload` and
    `capacity_scoped_finish_hint_rejects_each_invalid_shape_independently`.
- `crates/domain/src/audit.rs`
  - Broadens only the `CapacityMemberFinished` doc comment. The variant and
    `capacity.member.finished` string are unchanged.

No producer, caller, privilege, native write, metric, audit action/string, or release switch is added.
The schema bounds `epoch`; it does not establish any association with an actual producer, fence,
source certificate, receipt, or committed group state.

## Read-only preservation checks run

- AsyncAPI comparison: insertion-only diff with 9 channel lines, 4 operation lines, and 24 message
  lines. Every old line remains byte-identical and in its prior order.
- Baseline comparison: insertion-only 84-line entry. Every byte-line of all 20 old entries remains in
  order and each parsed old JSON subtree is semantically equal. The existing
  `CapacityMemberFinished` subtree is exactly equal.
- The new hand-authored baseline entry was compared to the reviewed type/required projection used by
  the Rust baseline collector: header `Nats-Msg-Id`, eleven payload properties, ten required fields,
  nullable string `traceId`.
- JSON parsing of the draft baseline succeeded.

## Existing registration and later commands

`tests/integration/workflow-primitives/main.rs` already has `mod hint_contract` and
`tests/Cargo.toml` already registers `integration_workflow_primitives`. The two new schema tests need
no registration edit. The baseline checker is already registered as
`contract_asyncapi_events_webhooks_messaging`.

Expected focused commands after the SDK run releases the tree:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked capacity_scoped_finish_hint_
cargo test -p sagat-tests --test integration_workflow_primitives --locked capacity_finish_hint_is_closed_control_metadata
cargo test -p sagat-tests --test contract_asyncapi_events_webhooks_messaging --locked
```

Run compatibility without `SAGAT_UPDATE_BASELINE`. The checked-in baseline entry is deliberate.

## Validation gaps retained for native work

- Per instruction, no Rust compile, test, formatter, contract validator, Cargo command, or baseline
  bless command ran. Python in this environment has neither YAML nor JSON Schema packages, so no
  substitute YAML/schema execution is claimed.
- The current Rust AsyncAPI compatibility helper checks channel message references and baseline
  shapes. It does not independently walk operation references; the canonical AsyncAPI validation gate
  must compile the new operation/channel/message graph.
- The baseline collector records only property types, enums, and required paths. The two new focused
  schema tests must prove formats, patterns, integer/string bounds, nullable/omitted trace, and
  `additionalProperties: false`.
- A later native scoped Finish unit must validate an actually produced outbox row against the
  canonical message and prove the exact audit reason/actor/resource/tenant, receipt IDs,
  replay/refusal counts, atomic rollback, and unknown-COMMIT reconciliation.
- Later native observation tests must cover all six outcomes, filtered-span parent safety,
  redaction/no identifier labels, and cancellation accounting. Existing ChildRoot event and
  observation regressions remain required.
