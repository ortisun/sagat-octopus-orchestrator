# Scoped leaf Finish qualification controls — handoff notes

Drafted read-only against repository HEAD `7b0ced121d5c2cb4bbc735d513be4894bebd5923` on 2026-10-18.
The repository was not modified. No Cargo command, test, formatter, review agent, commit, or external
write was run. The existing four native PostgreSQL tests and prior review status were treated only as
frozen input claims; they were not re-executed or re-claimed here.

## Installation

Copy `/tmp/sagat-scoped-finish-qualification-controls.rs` to:

`tests/integration/workflow-primitives/scoped_leaf_finish_qualification.rs`

Register it near the top of
`tests/integration/workflow-primitives/scoped_leaf_finish_native.rs`:

```rust
#[path = "scoped_leaf_finish_qualification.rs"]
mod qualification;
```

The file is intentionally a child module and starts with `use super::*;`. It reuses the private
`certified_flow`, `seed_group`, command, source snapshot, capacity snapshot, linked effect, and
header helpers. No helper visibility or production source change is needed.

## Three bounded controls

1. `qualification_distinguishes_exact_absence_from_observable_binding_mismatch`
   exercises exact tenant, namespace, group, and member absence as `Missing`; a proper owner-only
   ChildRoot member and scoped original Run, owner, and index discrepancies as `BindingMismatch`;
   and then applies and replays an exact supported native command. Every refusal checks exact source,
   group/member/receipt, audit, and outbox snapshots. The alternate Run is genuinely published and
   started in the same TestApi database. Its owner-only chain is attached only to that alternate
   Run. The selected original Run stays flat.

2. `qualification_preserves_a_same_tenant_holder_from_another_real_namespace`
   creates a second namespace through the same TestApi, publishes and starts a real Run there, and
   stores its path identity only as the nonselected holder fact. That holder has no leaf certificate.
   Finishing the genuinely certified selected member preserves the holder and epoch, returns
   `Held(foreign_holder)`, leaves the other member and alternate Run/source snapshot unchanged, and
   replays without another effect. This proves the operation has no foreign holder source/finality
   dependency. It does not instrument statement-level SELECT counts; the adapter's bounded SQL is
   still the direct evidence that holder restoration reads only the group member fact.

3. `qualification_rejects_a_group_wide_cross_kind_operation_key_then_applies_a_fresh_key`
   creates a proper owner-only ChildRoot member, alternate-Run chain, and schema-valid historical
   ChildRoot receipt in the same group. Reusing its operation key for a supported flat scoped command
   returns `FingerprintConflict` before decoding the unrelated ChildRoot result and creates no new
   audit or hint. A fresh key then retires the selected scoped member and exact replay returns the
   original result. ChildRoot rows are labelled storage decoder fixtures and make no native finality
   claim.

All three tests inherit `certified_flow`'s explicit `sagat_app` pool, genuine native leaf
certificate, source INSERT revocation before Finish, and disposable four-column scoped receipt
grant. Owner access is used only for capacity identities and the labelled alternate chain/receipt
fixtures.

## Suggested focused gate for the root

After installation and formatting, run the existing target with a private target directory and the
repository's machine guardrails. A name filter can establish RED/GREEN for this unit first:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  scoped_finish::qualification:: -- --nocapture
```

Then run the whole scoped native family and the required strict checks chosen by the root. These are
commands to run, not results from this draft.

## Explicitly outside this unit

- malformed, null, unknown, counter, possession, UUID, and timestamp receipt decoder branches;
- schema-valid out-of-range receipt time and the 9999/BC positive controls;
- wrong actual pin, future certified revision, corrupt source, and metadata-only/denied-table probes;
- raw diagnostic canaries, logs/spans, full context matrix, barriers, fault injection, COMMIT
  uncertainty, process recovery, and observation outcome coverage;
- production grants/caller, durable intent, retention/mixed-writer compatibility, creation,
  admission/accounting, nesting, or ChildRoot native finality.

No canonical task should be checked from this draft or from the eventual focused qualification run.

## Frozen inputs

- `.specify/memory/constitution.md`: `e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e`
- `specs/014-workflow-primitives/spec.md`: `5ee76107121bb0b7f4b2bea38a430cfe9ff8cd07d87ce6c24fd6fb4db5e258f0`
- `specs/014-workflow-primitives/plan.md`: `df76cdf7764ebe2f6d16293fb115205bd51466cd9671a657aeca5f3705137dfc`
- `specs/014-workflow-primitives/tasks.md`: `c25ca47fb054b6fe99cc3402d1ec7577fd3fa4f8add15d2cf2c5a5a9db230f5e`
- `tasks/project-completion/scoped-leaf-finish-native-contract.md`: `309184c1126574857230dc2aa305fcb8f4ba803ed98a895260295bce5edf9912`
- `tests/integration/workflow-primitives/scoped_leaf_finish_native.rs`: `dce6ca3080a7c7a6048346702627322aa05e6d8b8d1d726cff6cb2f15bdb46c1`
- `crates/infra-postgres/src/scoped_leaf_finish.rs`: `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780`
- `crates/infra-postgres/migrations/20261212000000_scoped_finish_binding.up.sql`: `a13861e74b7993f3411605538b6552a610db0e3afa973fa6b906f43f0002212c`

Draft code SHA-256 after the final static review:
`d8d76d0e551c9833dcacd1149bf788f5a2af91d14096098177e6574403a96544`.
The notes file's final hash is returned separately because it cannot contain its own stable hash.

During the final read-only status check, concurrent root work appeared in
`tests/integration/workflow-primitives/scoped_leaf_finish_native.rs`,
`tests/integration/workflow-primitives/scoped_leaf_finish_source_controls.rs`, and the matching
evidence directory. This draft remains frozen to the HEAD hashes above and did not inspect, depend
on, or modify those concurrent additions.
