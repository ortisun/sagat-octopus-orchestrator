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
  run-lock helpers, and declared Update by Run now takes the membership chain first (below). Map each before claiming
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
  of a chain member now has the narrow prefix correction below. Queries are reads;
  do not silently forward a run address while aligning mutation locks.
- The collection, loop and approval-specific scheduler checks now share transaction-typed
  execution::locked_scheduler_time. Independent unchanged-row barriers cover collection/loop
  entry and approval opening/genuinely due expiry. This resolves that predicate gap only;
  applicable primitive chain prefixes and complete B0 ordering are still pending.

These are source observations and pending tests, not already reproduced public failures or a
complete B0 graph. Existing suite passes do not settle these missing barriers.


## Direct run Update chain prefix — 2026-10-18

Refs:T014-040/044, FR-014-014, ADR-0030§18. PgDeclaredOperationStore::apply_update now
locks a run's tenant/namespace-scoped membership chain before its exact run. A Run address
never resolves to a newer head. Chain-addressed behavior, request hashes, current state CAS,
receipt replay, audit and outbox boundaries remain unchanged. No migration or wire change.

The public HTTP regression addresses a successor distinct from the chain ID. A real chain lock
barrier must precede own-run acquisition; a test-only receipt trigger exposes guard lifetime
until commit. Before correction, the test failed because the chain wait was never observed
(0/1,7.03s); this is not a reproduced PostgreSQL deadlock. After correction, the10 HTTP operation
cases passed (5.56s), including earlier-segment RUN_CONTINUED, terminal refusal, rollback/retry,
permissions and tenant opacity. The new case checks successor mutation, unchanged predecessor,
one durable receipt/state event/outbox, two trace-bearing audits for apply+replay, and foreign
tenant NOT_FOUND. A replay is a separately audited access, not a second mutation.

Two bounded reviews required a non-singleton successor and correct replay audit counts, then
found no production defect. The second suggested a stronger SHARE probe for the chain mutation
guard; its final focused verification is recorded in the evidence manifest. Remaining primitive
adapter prefixes, post-wait authority checks and complete B0/group ordering stay open. Exact
source, failures and commands: tasks/project-completion/evidence/direct-update-chain-prefix-20261018/.


## Chain scheduler expiry after row-lock wait — 2026-10-18

Refs:T014-040, FR-014-014/015. The chain port's expiry WHERE predicate could accept an authority
that expired while waiting for an unchanged scheduler_shards tuple. Three real PostgreSQL
barriers now cover legacy handoff, evaluated continuation and finalization. Each observes the
actual runtime-role wait while the lease is live, keeps the tuple unchanged until DB-clock
expiry, then releases it. On unchanged production all three committed under expired authority
(behavioral RED:1 pass for the overflow compatibility case,3 failures,6.47s). Earlier fixture API and
slug setup failures are retained separately and do not count as three behavioral counterexamples.

The shared execution::locked_scheduler_time helper remains transaction-typed and becomes
crate-visible. Chain checks use its lock-then-clock validation, retain own-shard rejection and
their existing oversized-generation Storage(Internal) conversion. Evaluated continuation keeps
its separate transaction_now after the incoming link lock for deadline/wait timestamps. No
lock order, wire capability, deadline policy, migration or external-effect guarantee changes.

GREEN passed4/4 (6.70s). Failed authority leaves independent chain/segments/run/journal/outbox/
audit/retained-usage snapshots unchanged. Fresh own authority succeeds, exact replay is detected
and journal consistency holds. The private chain fixture now honors the runtime-role option via
world.execution_pool. Its older owner-pool tests remain real results but did not prove runtime
privileges merely because that environment option was set; the existing explicit least-privilege
chain test was separate coverage. New wait barriers require sagat_app when the option is active.

Two bounded reviews identified generation-error compatibility, transaction typing and fixture
pool preconditions; the actual five-file review found no concrete defects. Reviewers ran no
tests. Evidence: tasks/project-completion/evidence/chain-post-wait-fence-20261018/. Collection,
loop and approval post-wait checks, their applicable chain prefixes and complete B0/group
ordering remain open. No canonical checkbox closes from this chain correction.

## Collection and loop scheduler expiry after row-lock wait — 2026-10-18

Refs:T014-040, FR-014-014/015. Both private scheduler checks now use the owning transaction's
locked_scheduler_time helper after preserving wrong-shard rejection and oversized-generation
Storage(Internal). All seven collection and six loop callers own their transaction. No new lock
prefix, deadline clock, capability, migration or nested execution eligibility is introduced.

Two runtime-role PostgreSQL entry barriers reproduced expired-authority commits after an
unchanged scheduler row wait. Clean RED was2 compatibility passes/2 failures (6.54s); GREEN
was4/4 (6.16s). Rejection leaves an atomic snapshot of17 tenant tables unchanged. The loop carry
is newly retained data, explicitly absent before entry. Fresh authority succeeds and exact
replay leaves persisted state unchanged. Collection success requires exactly one traced audit.
The initial collection attempt instead exposed a duplicate audit INSERT: enter_collection and
finish_version wrote the same record. Removing the entry INSERT retains the single atomic
finish_version audit. That audit-only correction preceded the clean RED; the earlier masked
audit failure and a SQLx static-SQL compile failure are recorded separately.

Two bounded reviews required non-vacuous new payload/audit oracles, then found no concrete
defects. New stores explicitly use runtime pools; historical owner fixtures do not acquire
least-privilege evidence from an environment flag. Existing scheduler mapping reports fenced
collection/loop outcomes with scheduler_actions_total and scheduler_fenced_total; these private
port barriers do not assert telemetry counters. Approval fencing, primitive chain prefixes,
complete B0 and all nested-capacity runtime/release gates remain open. Exact source variants,
commands and limits: tasks/project-completion/evidence/scope-post-wait-fence-20261018/.

Final focused regression passed15 collection transaction cases (10.64s),38 loop cases (22.18s)
and all92 fault cases (82.63s). Counts overlap the focused barriers. Strict locked workspace/
all-target Clippy (91.92s), formatting, diff and24-crate architecture gates passed. Exact tested
source, commands and staged evidence hashes are recorded in the scope-post-wait manifest.
No canonical task closes from this adapter correction.

## Approval scheduler expiry after row-lock wait — 2026-10-18

Refs:T014-040, FR-014-008/014. Approval open/expire now use the existing transaction-typed
lock-then-clock scheduler check. Own-shard rejection and stored_i64 generation conversion
retain their existing error precedence; user decide authorization and all business clocks stay
unchanged. The shared scope snapshot now includes telemetry_pending and all three approval
tables (21 tenant tables in one atomic SELECT).

Three new private tests use explicit runtime stores and the actual sagat_app wait oracle.
Opening and genuinely due expiry both committed after authority expiry before correction:
RED1 compatibility pass/2 failures (65.16s). The expiry fixture waits for its real60-second
business deadline using bounded75-second asynchronous PostgreSQL-clock polling, then waits
for scheduler expiry under an unchanged tuple. Its actual due timestamp supplies audit and
occurred_at together. It does not pretend a future occurred_at is a real due-time barrier.
Fresh authority opens/expires; opening success has one traced audit and expiry has one traced
audit/event/outbox plus a FIRED timer. Opening replay retains RuntimeUnavailable with no
persisted changes, matching its existing revision contract rather than inventing replay success.

The plan review required telemetry_pending coverage, correct opening replay and a deadline
wait longer than the normal five-second eventually helper. These are implemented. This bounded
correction adds no chain prefix, nested eligibility, migration, wire or public capability.
Existing scheduler_actions_total/scheduler_fenced_total map these errors to kind=approval_open
or approval_expire; the private port barriers do not test counters. Complete B0 and all nested
capacity and release gates remain open. Evidence: tasks/project-completion/evidence/approval-post-wait-fence-20261018/.

Final approval-expiry GREEN passed3/3 (67.95s, including the genuine60-second deadline).
Regression passed46 approval-filtered workflow cases (35.54s) and21 approval fault cases
(29.07s), skipping the3 already-run focused barriers. Strict locked workspace/all-target
Clippy (61.64s), formatting, diff and24-crate architecture gates passed. Two bounded reviews
ended with no concrete final defects; reviewers ran no tests. Exact source/commands/evidence:
tasks/project-completion/evidence/approval-post-wait-fence-20261018/manifest.json. No canonical
checkbox closes; complete B0, nested runtime and integrated release gates remain pending.
