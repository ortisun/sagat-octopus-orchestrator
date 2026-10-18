# Capacity lock inventory — B0 working evidence

Status: incomplete source inventory, not an approved new group ordering. Group persistence and
runtime eligibility remain unimplemented. Initial source at2455389e; expanded against3f302a9d.

| Existing writer | Observed explicit prefix/resources | Source |
| --- | --- | --- |
| Root RunStore begin | optional execution_chains row→own runs row | execution.rs:lock_chain_for_run, RunStore::begin |
| Exact-path begin_current_path/begin_path | optional execution_chains UPDATE→own runs UPDATE; contended chain skips before own-run acquisition under SkipLocked | execution.rs:lock_chain_for_run, lock_path_revision, PathExecutionStore impl |
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
- The mixed legacy UPDATE/link path has the focused correction below. ADR-0037 separately
  accepts only the early stable-parent identity KEY SHARE under immutable initial/head/link
  ancestry; it does not grant foreign aggregate mutation authority or accept the complete graph.
- Exact-path storage now takes the applicable chain guard in the same transaction before the
  run (focused evidence below). Collection/loop/approval adapters still have separate direct
  run-lock helpers, and declared Update by Run has a direct run prefix. Map each before claiming
  general composition compliance; publication restrictions do not prove a future shape safe.
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
without claiming a complete graph. The subsequent ADR-0037 clarification below resolves only
that identity-guard exception; B0 remains incomplete and unchecked.

Final focused regression passed fault92, durable178 and chain-filtered workflow72 tests. Both
chain-child and chain-cancel-parent matched unchanged goldens in every SDK, including detach.
Strict workspace/all-target Clippy, formatting and architecture passed. Exact results/hashes:
evidence/child-link-only-20261018/manifest.json. These bounded results do not close B0.


## 2026-10-18 — Stable parent identity guard (ADR-0037)

The existing handoff keeps chain→parent KEY SHARE→own head→incoming link. ADR-0037 permits
this narrow identity protection under immutable initial/head/link ancestry; it grants no parent
business-state or mutation authority. Parent commands still require explicit own-run FOR UPDATE.
No executable production change, migration, SDK/API change or nested activation is introduced.

Removing only the early guard as a reversible negative control made the existing ordered race
fail 0/1: handoff committed but parent cancellation returned Unavailable. That is an observed
public error, not proof of PostgreSQL SQLSTATE/deadlock cause. Original source bytes were restored
exactly and the same two-order race passed 1/1 (3.64s). Added positive ancestry assertions passed
for three direct child segments/link (3.05s) and a depth-two handoff with distinct parent/root
(2.85s). Two independent scoped reviews resolved an unstated ancestry precondition, then found
no remaining concrete defects. This is a current-writer invariant, not a stronger FK guarantee.

Exact controls, hashes and review: tasks/project-completion/evidence/parent-fk-guard-20261018/.
The complete B0 graph, B1/group persistence, compatibility fencing and production resource/release
gates remain open. No canonical checkbox closes from this clarification.


## Exact-path chain prefix — 2026-10-18

The existing begin_current_path and begin_path helpers previously took only their own run lock.
Both now invoke the same tenant-scoped optional chain guard as full RunStore::begin, in the same
transaction before lock_path_revision. A contended chain returns None for SkipLocked; Wait keeps
the chain guard until path commit. Snapshot CAS, database-time sampling and exact-path mutation
bounds are unchanged. The additional membership lookup uses chain_segments_run_unique; it does
not load historical members. Retaining the SQL guard grants no new chain finality authority.

Four real storage-protocol tests use a trusted scoped fixture plus real singleton chain
initialization before loading the snapshot. General public chain+collection composition remains
closed. Both entry points fail the pre-fix SkipLocked and Wait oracles (0/4, 3.40s): contended chains
were ignored. With the fix, all eight bounded fence tests pass (6.51s). Actual pg_blocking_pids
barriers prove Wait blocks on chain before holding run; independent NOWAIT probes prove both
locks last through commit and are then released. Skip tests prove None/nonblocking and no
retained run lock, with ordering established by the corresponding Wait test.

Two isolated reviews identified fixture deadline/revision and lock-lifetime oracle requirements,
then found no concrete final defects. An earlier prototype RED and a fixture import compile
failure are retained separately from the corrected-fixture behavioral RED. Exact records:
tasks/project-completion/evidence/path-chain-prefix-20261018/. B0 remains incomplete; this does
not accept the other primitive/direct-Update prefixes, group transactions or general nesting.

## Additional source gaps to resolve (not accepted new ordering)

- `primitives::{lock_run,lock_drain_run}`, `loop_progress::lock_run` and
  `approval_progress::{lock_run,lock_run_whatever_its_state}` take own-run UPDATE directly.
  Current supported public shapes exclude their chain composition; their storage prefixes still
  need alignment/proof before that composition can be enabled.
- `declared_operations::apply_update`, OperationAddress::Run, takes own-run UPDATE directly;
  the Chain branch explicitly takes chain before its resolved run. A direct run-addressed Update
  of a chain member therefore requires a separate narrow prefix correction. Queries are reads;
  do not silently forward a run address while aligning mutation locks.
- The collection, loop, approval and chain-specific `check_scheduler_fence/check_fence` helpers
  still put `expires_at > clock_timestamp()` in the locking SELECT. Full/exact-path run entry uses
  `execution::locked_scheduler_time` to sample expiry after acquiring SHARE. A predicate evaluated
  before an unchanged-row wait is not the same proof. Add real expiry-during-wait barriers for
  each primitive authority before claiming all runtime scheduler fences are equivalent.

These are source observations and pending tests, not already reproduced public failures or a
complete B0 graph. Existing suite passes do not settle these missing barriers.
