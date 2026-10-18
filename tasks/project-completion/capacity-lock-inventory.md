# Capacity lock inventory — B0 working evidence

Status: incomplete source inventory, not an approved new group ordering. Group persistence and
runtime eligibility remain unimplemented. Initial source at2455389e; expanded against3f302a9d.

| Existing writer | Observed explicit prefix/resources | Source |
| --- | --- | --- |
| Root RunStore begin | optional execution_chains row→own runs row | execution.rs:lock_chain_for_run, RunStore::begin |
| Exact-path begin_current_path/begin_path | own runs row via lock_path_revision; no chain lock in that helper | execution.rs:482,530,566 |
| Collection admission | run/fence before primitive-policy shared fence→tenant policy row→tenant_iteration_usage→run_instance_usage→iteration scope | primitives.rs:tenant_active_limit and collection admission around1212 |
| Chain handoff | execution_chains row→hold_parent parent runs KEY SHARE→own head runs UPDATE→scheduler fence→child link | chain_progress.rs:lock_chain, hold_parent, continue_as_new |
| Parent settle_child/release_child | own parent transaction→matching child_chain_links UPDATE; only a missing link falls back to legacy child runs UPDATE with exact own-parent predicate | execution.rs:update_child_chain_link, settle_child, release_child |
| Initial child creation, before correction | payload input/initial-state INSERT→parent runs UPDATE→tree advisory→managed database admission if applicable→namespace active-run quota→tenant active-run quota→child INSERT | execution.rs:RunStore::create, claim_reservation; quotas.rs:ensure_room_for_run |
| Evaluated handoff, after chain/run/link | tree advisory if child→namespace active-run quota→tenant active-run quota→input/state payload INSERT→successor INSERT | chain_progress.rs:continue_evaluated, ensure_replacement_capacity, retain_payload |
| Scoped worker claim | exact-path own run→drain shared advisory→binding shared advisory→session UPDATE→current role/authority checks→activity policy/quota admission in sorted activity order | application/worker/scoped_claims.rs:claim_path; execution.rs:worker_drained, bound_patterns, worker_session_room; quotas.rs:lock_activity |
| Worker registration | exclusive binding advisory→current grants→worker-session quota if new→session upsert | workers.rs:create_session |
| Binding replacement | exclusive binding advisory→binding/pool rows→sessions UPDATE→held step lease expiry UPDATE→audit | bindings.rs:replace_binding |
| Activity admission | tenant policy shared advisory→namespace policy shared advisory→exact activity advisory→count/check→lease under the already-held run transaction | quotas.rs:activity_policy_lock, lock_activity, activity_admitted |
| Deferred retained quota finalization | positive content/log delta→content policy shared advisory→log policy shared advisory if needed→tenant_retained_usage UPDATE; tenants visited in tenant order | 20261024000000_durable_quotas.up.sql:sagat_quota_flush_deltas; content_usage.rs:allocate_logs |

`runs_parent_same_tenant_fk` and `runs_root_same_tenant_fk` in
`20261009000100_execution.up.sql` acquire key-share protection on referenced rows at child/successor
INSERT. At depth two, immediate parent and root differ. Explicit `FOR UPDATE` conflicts with those
key-share locks; `FOR NO KEY UPDATE` still excludes competing parent writes while allowing stable
key references. Modes belong in the graph: collapsing every row lock to one exclusive vertex
would conceal this distinction. Payload `INSERT ... ON CONFLICT` can also wait on an uncommitted
same-tenant digest; immutable contents do not mean the uniqueness check is nonblocking.

## Focused admission correction — 2026-10-18

The narrow correction concerns existing run creation, independently of new groups:
keep parent-first/tree admission, use `NO KEY UPDATE` for the reservation's parent lock only, and
move both payload retention blocks after active-run admission. Root commands and every other
run-lock helper keep their current modes. This adds no second existing-run authority or group
activation. Two real barrier tests target the root-FK and quota/payload cycles separately.
Before the change both failed: root-child creation returned an explicit PostgreSQL deadlock;
the new-root public API returned `Unavailable` in the controlled quota/payload competition.
After the change both passed (8.59s total). The first drops its admitted transaction, checks that
no child survived rollback, then creates it through the real scheduler and checks the journal
oracle. The second commits the new root and checks both run histories. Test setup/compile failures
are retained distinctly and are not behavioral RED. Broader runtime-role regression passed709 tests with the long10k reference-source case
ignored; strict workspace/all-target Clippy, format/diff and24-crate architecture checks passed.
Exact source/commands/failures: evidence/child-root-fk-20261018/manifest.json.

Two isolated review cycles covered the remedy. The first found that changing only the parent
mode leaves payload uniqueness inversions: payload-before-tree on child creation, and
payload-before-active-quota even across trees/root starts. Moving both retention blocks after
admission addresses both edges. The second reviewed the actual patch/schema/barriers and found
no substantive issues. No additional external review was invoked, as selected by the owner.
These reviews do not accept the complete B0 graph or the group transaction design.

## Gaps requiring resolution before B1

- The handoff actually takes chain→parent KEY SHARE→own head/link. Its old parent-before-chain
  comment is corrected. The early guard remains a cross-run metadata lock, not a proof of the
  strict own-run-only protocol; include it and the implicit successor FKs in any wait-for graph.
- The mixed legacy UPDATE/link path has the focused correction below. The existing handoff
  parent KEY SHARE guard still obtains another existing run row and therefore remains a strict
  ADR-0030§18–19 deviation requiring a separate decision before B1 can claim that order compliant.
  Correcting the legacy mutation path does not accept this guard or the complete graph.
- Exact-path helpers directly lock a run. Map chain-capable scoped activity/public-operation call
  paths before claiming their chain-before-run prefix; publication restrictions do not prove an
  unsupported future composition safe.
- Complete the implicit-FK/trigger, root-versus-scoped worker, current-role/session/activity,
  managed database/script, and cancellation incoming/outgoing-link edges beyond the prefixes above.
  The current table is not a complete DAG and cannot yet justify inserting group locks.
- New group eligibility/settlement may not lock a second existing run or assume source terminal
  rows can be reopened under scheduler authority. Group/member CAS is subordinate to the verified
  current owning authority; exact link/group/counter ordering is still pending B1.

Prior current full suites pass, which does not prove missing lock cycles impossible. The new
model is not runtime activation evidence. B0 remains unchecked.


## Focused chain-link settlement correction — 2026-10-18

Both settle_child and nonsettling release_child previously updated the first child run's legacy
flags before its chain link, even though initialization sets parent_settled=true and maintenance
already discovers cancellation/finality through the link. The actual public cancellation and
parent-notification paths reach these writes. Three real PostgreSQL trigger regressions rejected
any UPDATE OF parent_settled,parent_cancel on the already-linked first segment: both Propagate
and RequestCancel returned Internal, while final notification reported one failed scheduler
action (0/3 passed). These are proof of the mixed writes, not a reproduced database deadlock.

The correction updates the exact tenant/own-parent/child link first and returns when it matches.
Only absent links fall back to legacy flags, also restricted to the exact own parent. Parent
creation serialization excludes concurrent initial link insertion. The three focused cases now
pass (2.54s), preserve legacy metadata, check final/cancellation latch state, journal consistency
and exact cancellation audit counts. Automatic final notification preserves its existing audit
behavior; its journal completion appears once. No new audit action is invented.

Two read-only reviews covered this small remedy. The first required explicit audit_records
assertions; the second found no concrete source/test issues. Reviewers ran no tests. Actual
source/commands/failed and final regression evidence live in evidence/child-link-only-20261018/.
The handoff comment and contract now name its actual chain→parent KEY SHARE→head/link order,
without accepting that existing metadata-guard deviation. B0 remains incomplete and unchecked.

Final focused regression passed fault92, durable178 and chain-filtered workflow72 tests. Both
chain-child and chain-cancel-parent matched unchanged goldens in every SDK, including detach.
Strict workspace/all-target Clippy, formatting and architecture passed. Exact results/hashes:
evidence/child-link-only-20261018/manifest.json. These bounded results do not close B0.
