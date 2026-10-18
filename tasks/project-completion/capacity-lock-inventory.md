# Capacity lock inventory — B0 working evidence

Status: incomplete source inventory, not an approved new ordering. No lock/code changes made.
Read source at2455389e (later3d877438 changes only captured-log attributes).

| Existing writer | Observed explicit prefix/resources | Source |
| --- | --- | --- |
| Root RunStore begin | optional execution_chains row→own runs row | execution.rs:lock_chain_for_run, RunStore::begin |
| Exact-path begin_current_path/begin_path | own runs row via lock_path_revision; no chain lock in that helper | execution.rs:482,530,566 |
| Collection admission | run/fence before primitive-policy shared fence→tenant policy row→tenant_iteration_usage→run_instance_usage→iteration scope | primitives.rs:tenant_active_limit and collection admission around1212 |
| Chain handoff | execution_chains row→hold_parent parent runs KEY SHARE→own head runs UPDATE→scheduler fence→child link | chain_progress.rs:lock_chain, hold_parent, continue_as_new |
| Parent settle_child | parent transaction already open→legacy child runs UPDATE→child_chain_links UPDATE | execution.rs:settle_child |

## Gaps requiring resolution before B1

- The handoff helper comment says parent-before-chain, but its actual call occurs **after** the
  chain row is locked. Preserve this distinction in any wait-for graph; comments are not evidence
  of acquisition order.
- ADR-0030§19 prohibits mixing legacy child-row UPDATEs into the new child-link protocol.
  settle_child currently executes both updates. Determine actual reachable legacy/new call paths
  and prove a deadlock counterexample or demonstrate the intended exception; do not silently
  declare this compliant. A divergence requires an ADR and dependent-document update.
- Exact-path helpers directly lock a run. Map chain-capable scoped activity/public-operation call
  paths before claiming their chain-before-run prefix; publication restrictions do not prove an
  unsupported future composition safe.
- Map implicit parent/root foreign-key KEY SHARE, triggers/deferred quota finalization, tree
  admission locks, worker binding/session/task quota locks and cancellation incoming/outgoing links.
  The current table is not a complete DAG and cannot yet justify inserting group locks.
- New group eligibility/settlement may not lock a second existing run or assume source terminal
  rows can be reopened under scheduler authority. Group/member CAS is subordinate to the verified
  current owning authority; exact link/group/counter ordering is still pending B1.

No executable lock tests run for this inventory yet. Existing current full suites pass, which does
not prove missing lock cycles impossible. The new model is not runtime activation evidence.
