# Ordinary unchained-child source baseline draft handoff

## Integration target

- Baseline source tree: `f7c0ef59e972b61dfed93c4e3f98914a77f9e51b`, branch `feature/016-project-completion`, clean when this task began.
- During drafting, the shared tree advanced cleanly to reviewed decision commit `9b61b3e40a6e5bcb8bac93a3543abf9771f6cc44`. The production and test sources used by this draft retain their baseline hashes below.
- Proposed repository path: `tests/integration/workflow-primitives/leaf_certification_child.rs`.
- Register as a child of `tests/integration/workflow-primitives/leaf_certification_native.rs`:

  ```rust
  #[path = "leaf_certification_child.rs"]
  mod child_source;
  ```

- Proposed focused filter after integration: `cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::child_source::`.
- The draft is standalone test code. It changes no production code, migration, runtime grant, SDK, DTO, or public contract. The existing public workflow DSL already expresses the parent, child, and ordinary collections used here.

## Owned draft

- `/tmp/sagat-ordinary-child-source-baseline.rs`
- Amended SHA-256: `9cd522210e8a6ed8ae8ab135694c8a707e5f3fec59ffaf43bdd2b762aee2ce42`.

Sealed pre-amendment/C1 inputs are retained for provenance:

- Owned source: `7c0e198ae9b6159ad69e71d08e899c59621ec179a64f68de6f26d959229410d0`.
- Owned notes: `4ba2ff36b97d554fc2a3126a16eca73dc2de4a38a331e7fe19a92ccb9b717030`.
- Root's rustfmt-integrated child at the initial compile boundary: `c5d0788281d6445404833b3bf4ae89ba60dcd013e2d524e6d426b7229e05e0b1`.
- Root's registered `leaf_certification_native.rs` at that boundary: `679089900793c3e1949c834747aea66900f419f0116a508db61350b6cbb574c7`.

Root reported that both fresh C1 review slices finished and that its initial compile-only command
passed in 36.22 seconds with 1,104 unchanged inputs. That is root's evidence; this drafting task did
not rerun the compile or any test.

## Post-C1 amendment diff

- Replaced the sequence/type-only record check with an independent projection of the actual
  qualified `lines/0` scope, iteration, checkpoint, returned step, owner step and pinned workflow
  document. The projection compares certificate scope, body, logical Return identity, both result
  digests and all five sequence references to those actual rows.
- Independently selects the five retained `lines` events and checks their root owner instance id,
  root instance path and complete event details: admitted entry/item digest, returned identity,
  checkpoint identity/digest, successful aggregate digest and routed `later` successor.
- Added a pure detector by cloning the returned record, flipping one checkpoint digest byte and
  proving the independent projection no longer compares equal. No retained row is modified and no
  production source-proof implementation is reproduced or called by the oracle.
- After legitimate later work and public parent settlement, now requires the child Run/journal,
  child steps and parent/relationship snapshots all to change while selected source and certificate
  remain equal. After terminal canonical replay, it compares the entire exact snapshot to the
  terminal snapshot, proving replay changed none of those fields.
- The amendment adds no parent race, incoming-policy claim, source behavior, production code or
  public contract.

The module deliberately uses `use super::*` so it can consume the existing private `document`, `command`, and `selected_sources` test helpers. It does not broaden their visibility or refactor the shared harness.

Reusable child-module API:

- `ordinary_child_source_boundary(label) -> OrdinaryChildSourceBoundary`: starts and verifies a terminal bootstrap, enables the real child scheduler pass, publishes and starts a public parent and ordinary child, closes `lines/0`, leases `later/0/work`, and discovers the source command after that lease advances the child revision.
- `OrdinaryChildSourceBoundary`: exposes `flow`, `bootstrap`, `parent`, `child`, the leased `later` task, and the exact `source` command to later sibling tests within the parent module.
- `ordinary_child_source_snapshot(flow, parent) -> OrdinaryChildSourceSnapshot`: records exact selected `lines/0` facts, selected certificate, child Run plus journal, all child steps, and ancestry/cancel/settlement plus parent status/child-step flags.

The positive child is created only through public workflow publication and public Run start. Its actual `ParentLink`, tenant, namespace, depth/root, default `PROPAGATE` policy, waiting parent step, and absence from all three chain tables are asserted. No topology or certificate is forged.

## Tests and controls

### `native_certifier_characterizes_a_real_ordinary_unchained_child_source`

1. Confirms the bootstrap Run reached `SUCCEEDED`, then creates the real public child with `.with_children(DefinitionCache...)`.
2. Calls the native certifier before selected closure and requires `SourceNotClosed`, with zero exact certificate facts and audits.
3. Completes the real `lines/0/work` lease. It asserts scope `SUCCEEDED`, iteration `CHECKPOINTED`, exact Return/checkpoint/digest links, owner and both local steps succeeded/routed, released slot and reservation, and no selected saga obligation.
4. Leases but leaves `later/0/work` pending (`OPEN`/`ADMITTED`/`LEASED`, slot and token present), then discovers the source command at the resulting current child revision.
5. Temporarily grants only certificate `INSERT` to the effective `sagat_app` role, proves no update/delete/truncate authority, creates one native certificate and one truthful audit, and checks the actual definition pin, certified revision, and all five retained event references.
6. Shows certification changes only the selected certificate metadata among the source/Run/journal/steps/parent snapshots. It then revokes `INSERT` before every replay.
7. Uses a cold runtime pool, definition cache, and store. Stale own-child scheduler ownership refuses before canonical replay; current ownership returns the byte-for-byte logical original record.
8. Wrong tenant and namespace return `Missing`, neighboring index returns `SourceEvidenceMissing`, and the public parent cannot reuse the child's evidence (`UnsupportedShape`). Every negative leaves the exact snapshots and one fact/audit counts unchanged.
9. Completes the actual later task, drives the child and public parent to success, checks the one observed successful settlement order, and proves selected source/certificate immutability. The old source identity with freshly discovered current child authority replays the exact original certificate. Both child and parent journals are verified.

### `genuine_ordinary_child_certificate_still_cannot_authorize_scoped_finish`

1. Creates a fresh real child and genuine native certificate, revokes certificate `INSERT`, then completes exact canonical replay before restricting reads.
2. Owner-seeds only a test registry fixture: one active `SCOPED_PATH` member, a valid `VACANT` group header, active count 1, revision 1, epoch 1. No certificate or receipt is forged.
3. Builds a structurally valid Finish command with the actual current child Run revision and scheduler authority, and checks its 32-byte fingerprint.
4. Snapshots the full child source/certificate/Run/journal/steps/parent flags and full group/member/receipt/capacity-audit/hint state.
5. Revokes certificate and receipt `SELECT` only after all source-certification assertions. Effective `sagat_app` probes and direct statements both demonstrate SQLSTATE `42501`; certificate `INSERT` is also absent. No receipt `INSERT` grant is added.
6. Requires the existing `PgScopedLeafFinishStore` to return `UnsupportedRunContext` before either denied read and verifies every source and capacity snapshot and the one fact/audit counts remain unchanged.

## Honest scope limits

- This characterizes existing `PgLeafCollectionSuccessStore` behavior for one genuine, public, ordinary, unchained child source. It adds no new producer guarantee or behavior.
- The successful parent settlement assertion covers one ordinary success order only. It does not cover both writer orders, all incoming parent policies, cancellation/drain races, adverse terminal fanout, or complete child relationship preservation.
- It does not cover continuation chains, nested closure, effectful selected bodies, resource ceilings, old binaries, mixed versions, or production activation.
- It does not authorize scoped Finish for a child and does not enable any production source writer, Finish caller, or released grant.
- Parent flag writes are not assumed to advance the child Run revision. The only explicit revision boundary here is the actual later task lease, followed by command rediscovery.
- Cold pool/cache/store reconstruction is not described as an OS or process restart.
- No tests, builds, clippy, formatter, or commits were run for this drafting task, as required. The integrating root must execute and report the focused tests.

## Read provenance at the baseline

| Source | SHA-256 |
| --- | --- |
| `AGENTS.md` | `51a8e97403a6eb1b043321dc92e4c3fcbfc90d8f487b59ca49a157ac3095b222` |
| `.specify/memory/constitution.md` | `e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e` |
| `specs/014-workflow-primitives/spec.md` | `5ee76107121bb0b7f4b2bea38a430cfe9ff8cd07d87ce6c24fd6fb4db5e258f0` |
| `specs/014-workflow-primitives/plan.md` | `df76cdf7764ebe2f6d16293fb115205bd51466cd9671a657aeca5f3705137dfc` |
| `specs/014-workflow-primitives/tasks.md` | `c25ca47fb054b6fe99cc3402d1ec7577fd3fa4f8add15d2cf2c5a5a9db230f5e` |
| `crates/infra-postgres/src/leaf_certification.rs` | `d39f67bc5a41fdb00b2e2d3eff05c1705f27e22583abcc9f1fb7ecfcbe744669` |
| `crates/infra-postgres/src/scoped_leaf_finish.rs` | `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780` |
| `tests/support/scope_flow.rs` | `c6722902a332ae5c693f06598b0b21d3652afa5ace63a626aba3931a532fa821` |
| `tests/integration/workflow-primitives/leaf_certification_native.rs` | `468e0505b935e3696e5ee0aa28d32f51d338e23eda5241b0d5bb7361b454f14d` |
| `tests/integration/workflow-primitives/scoped_leaf_finish_contexts.rs` | `7222e366728f2fcb5ca1381186e6e830c2aa96ddf018131c97d13acb66af2326` |
| reviewed proposal `/tmp/sagat-next-capacity-relationship-unit.md` | `943bb65dec4d9f5fa9147413d67ea3b1fb8a4c514bed831ef7cfa93c0bf8c0e7` |

