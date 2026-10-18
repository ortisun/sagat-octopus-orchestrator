# Current execution lock graph — B0 candidate

Base56044f75. Source inventory for B1 design, not a proposed group order or a deadlock-free
platform claim. U=FOR UPDATE, NK=FOR NO KEY UPDATE, S=FOR SHARE, KS=FOR KEY SHARE;
advisory X/S denote exclusive/shared transaction locks. Arrows retain locks on the same owning
transaction. An optional resource is acquired only when applicable; an unchanged FK does not
invent a fresh reference check. Exact tenant/namespace/identity keys and modes matter.

## A. Existing execution entry and foreign aggregate boundaries

All paths below have their current source references in `crates/infra-postgres/src/`.

| Family | Ordered resources / boundary | Source |
| --- | --- | --- |
| Full execution (scheduler, worker, signals, timers, cancel, saga) | applicable member chain U → exact own run U; no head forwarding | execution.rs:RunStore::begin, lock_chain_for_run |
| Exact-path current/snapshot execution | applicable member chain U → exact own run U; SkipLocked skips chain before run | execution.rs:PathExecutionStore, lock_path_revision |
| Chain-addressed signal/cancel | exact addressed chain U → resolved current head run U; distinct from run-addressed entry | chain_progress.rs:begin_chain_head; application/chain_progress.rs |
| Chain-addressed Update | addressed chain U → receipt lookup → current head or prior receipt resolved_run_id U | declared_operations.rs:apply_update (chain address) |
| Declared Run Update | applicable member chain U → exact addressed own run U → operation receipt/state/effects | declared_operations.rs:apply_update; execution.rs:lock_member_chain |
| Collection entry/admit/advance/drain/finish | applicable member chain U → exact own run U → scheduler S when required → own scope/member/step and quota resources below | primitives.rs:lock_run, lock_drain_run |
| Loop enter/advance/drain/finish | applicable member chain U → exact own run U → scheduler S → own scope/carry/member/step and quota resources below | loop_progress.rs:lock_run |
| Approval open/expire | applicable member chain U → exact own run U → scheduler S → request/timer/receipt/effects | approval_progress.rs:lock_run_whatever_its_state |
| Approval decide | same chain/run entry → request/receipt/effects, current decision authorization rather than scheduler authority | approval_progress.rs:decide |
| Continuation | own chain U → stable original parent run KS if linked → exact expected own head U (historical on replay) → scheduler S → incoming link U → tree/active-run quota/payload/successor below | chain_progress.rs:lock_chain, hold_parent, continue_as_new, continue_evaluated |
| Finalization | own chain U → exact current own head U → scheduler S → incoming child link U → final projections/effects; no successor parent guard | chain_progress.rs:finalize_chain |
| Parent notification/settle/release | own parent chain/run entry → exact outgoing child_chain_links NK; existing linked child's runs row is never updated | execution.rs:update_child_chain_link, settle_child, release_child |
| Legacy child fallback | only when no matching link: own parent transaction → exact own-parent legacy child runs NK | execution.rs:settle_child, release_child |
| Initial child admission | reservation's exact parent run NK → tree advisory X → reservation step NK → managed admission if applicable → namespace active-run quota X → tenant active-run quota X → payload input/state insert → new child run INSERT | execution.rs:RunStore::create, claim_reservation; quotas.rs:ensure_room_for_run |
| Root admission | private/ad hoc retained-definition namespace Workflows quota X if applicable → managed admission if applicable → namespace/tenant active-run quota X → payload input/state insert → new run INSERT | execution.rs:RunStore::create |
| Ordinary terminal projection | existing own chain/run entry → own segment/chain updates → incoming child link NK; operator completion/cancel does not gain scheduler S from this suffix | execution.rs:persist, project_terminal_chain_inner |
| Approval cancellation suffix | own execution entry → exact owner step U → request NK → timer NK | execution.rs:cancel_approval |
| Initial chain setup | new uncommitted run U → new chain/segment/link inserts in the same creation transaction; later membership identity immutable | chain_progress.rs:initialize_chain_in_transaction; execution.rs:create |
| Production incident | applicable member chain U → exact own run U → binding advisory X → exact own step U → audit deduplication | scripts/incident.rs:record, ADR0038 |

Continuation's parent KS is identity protection, never parent mutation authority (ADR0037).
The original parent and root identities stay immutable across segments and links. Initial child
NK excludes competing parent writes but is compatible with reference KS. At depth two, parent
and root differ. No protocol may infer a permission to mutate another existing run from metadata
KS. Legacy fallback is an existing bounded exception, not a permitted new group operation.
Full/path scheduler commands conditionally acquire their own shard S after entry. Initial child
startup takes parent shard S via holds_parent_shard only after admission/new-run insertion, on
the still-open child creation TX (scheduler/passes.rs); this distinct suffix is not own-child
execution authority. Fence time is sampled after the actual shard guard.

## B. Session, current authority, policy and admission edges

| Family / exact resource | Ordered resources / boundary | Source |
| --- | --- | --- |
| Root ordinary claim | chain/run → drain advisory S → matching session U → activity admission; binding/pool hints do not retain authority locks, no retained current-role guard in this ordinary root claim | application/worker.rs:claim_root_run; execution.rs:worker_drained, worker_session_room, current_worker_authorized |
| Scoped ordinary claim | chain/run → drain S → binding S → session U → namespace Secrets quota X → current membership S → activity admission | application/worker/scoped_claims.rs:claim_path; execution/path.rs |
| Ordinary report/heartbeat/capability | chain/run → binding S → namespace Secrets quota X → current membership S before effects; payload/pool checks follow current authority | application/worker.rs:ensure_bound; execution.rs:current_worker_authorized |
| Managed claim | chain/run → drain S → session U → tenant database-concurrency advisory X → namespace Secrets quota X → dedicated membership S → managed actor/catalog admission → activity admission | application/worker.rs:claim_root_run; execution/database.rs:database_admission |
| Managed existing report receipt replay | chain/run → receipt lookup → namespace Secrets quota X → dedicated membership S → binding S, without session S; different from live owner/capability | application/worker.rs:replayed; execution/database.rs:database_report_owner |
| Managed owner/capability | chain/run → binding S → exact owning session S → namespace Secrets quota X → dedicated membership S; database snapshot then repeats catalog admission | execution/database.rs:database_owner, database_snapshot |
| Session renewal | matching tenant/id/token session U → postlock DBclock → renewing UPDATE, one owning TX; no run/chain acquired | workers.rs:use_session |
| Session registration | binding X → current membership S → session quota X if new → session UPSERT; no run/chain acquisition | workers.rs:create_session |
| Binding replacement/revocation | binding X → binding/pool mutations → matching sessions U → affected step expiry UPDATE → session deletion → audit; no run/chain acquisition | bindings.rs:replace_binding |
| Activity quota | tenant activity-policy S → namespace activity-policy S → exact activity quota advisory X → counts/lease; multiple names sorted | quotas.rs:activity_policy_lock, lock_activity, activity_admitted |
| Primitive iteration admission | primitive-policy S → tenant collection policy S → tenant_iteration_usage U → run_instance_usage U → own scope/member/steps, under existing chain/run/scheduler | primitives.rs:tenant_active_limit, admission; loop_progress.rs |
| Managed catalog admission | namespace Secrets quota X → database tenant/namespace policy S → sorted actor memberships S → namespace S → each operation connection S, secret S, actor ACL S | database/admission.rs:check_definition; database/policy.rs |
| Connection/provider/secret-version mutation | namespace Secrets quota X before applicable current membership/catalog/secret changes; separate from ACL and pool writes | database.rs, managed_secrets.rs, secrets.rs |
| Secret ACL edit | grant INSERT/uniqueness plus referenced secret KS, or exact ACL DELETE U → audit; no namespace Secrets quota X | secrets.rs:grant_access, revoke_access |
| Secret pool edit | exact secret U → pool replacement → audit; no namespace Secrets quota X | secrets.rs:replace_secret_pools |
| Delivery pool check | secret rows S in id order → binding S; callers already retain binding S; hint=false takes neither guard | secrets.rs:pool_refusal; worker ensure_bound/managed owner callers |
| Ordinary secret delivery suffix | worker authority → secret rows S → publisher membership S → publisher ACL S; publisher may differ from worker | application/worker/secrets.rs; secrets.rs:inputs |
| Declared script retention admission | activity admission → script-retention tenant/namespace policy S → bounded READY script retention lookup/pins, including declared derived obligations | execution.rs:current_script_retention; content_usage.rs |
| Derived script retention admission | activity admission → originating lease pin read, legacy READY lookup fallback; no current script-retention policy guard | execution.rs:inherited_script_retention; execution/path.rs |

Namespace Secrets quota is the exact advisory key `quota:{Secrets}:{tenant}:{namespace}`
(`quotas::lock_secrets`), not an abstract catalog row. Database concurrency is tenant-wide;
activity resources include tenant/namespace/name. Membership/namespace/ACL S uses narrow
SECURITY DEFINER functions (20261126000000_authority_read_locks), no new runtime UPDATE grants.
Database admission iterates definition operations, not sorted connection IDs: namespace Secrets
X serializes all such catalog access in that namespace; different namespaces have disjoint
connection/secret keys, with shared actor membership guards. Do not substitute a global ID-order
claim. Namespace policy editors take tenant S → namespace X; tenant editors take tenant X.
Exact-activity edits take tenant activity-policy S → namespace activity-policy S → exact activity
quota X. They do not acquire run/catalog/secret rows afterwards.

Claim loops retain each task's admission locks until the entire execution TX commits, then
repeat the suffix for the next task. Thus activity(A) X → managed connection(B) S and declared
script-policy S → activity(B) X are real retained edges. Sorting activity names prevents only
activity-to-activity inversions; it does not remove these cross-resource edges. Managed admit_node
checks one selected node, not every connection in advance.

Root ordinary claim does not call current_worker_authorized in its run transaction; transport
authentication reads membership before service entry. This inventory does not claim a retained
role fence during ordinary root claim. Scoped claims and ordinary report/capability do call it.
B1 must preserve or explicitly resolve this distinction, not infer a missing shared lock.

All three session checks now sample database time after acquiring their actual row guard:
renewal, full/path claim-room U, and managed owner S. They establish validity at that sample,
not continuously after later waits or until commit. Counters are protected by owning TXs; no
pool connection request occurs while retaining a transaction just to perform these checks.

## C. Implicit references, deferred writes and auxiliary transactions

| Resource | Source / retained edge and limitation |
| --- | --- |
| Parent/root FKs | 20261009000100_execution: new child/successor run inserts acquire referenced parent/root KS. Parent NK compatibility matters; chain continuation retains stable parent KS before head U. |
| Rerun FK | 20261015000300_operator_actions: runs.rerun_of takes existing terminal source run KS at INSERT, after managed admission/active-run quota/payload retention. Managed admission Secrets X → source KS opposes terminal cleanup source U → Secrets X; source inversion pending focused proof/correction, not a reproduced deadlock claim. |
| Chain references | 20261102000000_execution_chains: chain initial/current references runs; segment run/predecessor/successor references runs; child link references parent run and child chain. Newly created identities are owned/uncommitted in creation TX. Existing terminal/link nonkey writes preserve FK identities. |
| Deferred chain references | Segment predecessor/successor membership and chain current-head member FKs are DEFERRABLE INITIALLY DEFERRED; include referenced own-chain segment KS at commit. Handoff creates successor in owning TX and updates its predecessor/head atomically. |
| Own execution projections | run_events/steps/scope/member/timer/approval/telemetry rows reference own run and/or own step/scope, payload, namespace/version. Retain own run before execution projection inserts; stable shared catalog references are not foreign aggregate mutation authority. |
| Payload uniqueness | payload INSERT ON CONFLICT can wait on another TX's same-tenant digest. Input/state retention is after active-run admission; run effects retain own run before their payload/projection writes. |
| Blob reservation | blobs.rs:BlobCatalog::reserve takes blobs:{tenant} admission advisory X when configured → new STEP_OUTPUT blob INSERT references existing run/step KS → deferred retained quota → commit. This is separate from commit_input on an owned new run and unchanged-identity blob updates. |
| Retained quota flush | 20261024000000_durable_quotas: every nonzero delta (including deletion) takes tenant_retained_usage U; only positive growth takes content/log policy S first. Multiple tenants processed in tenant order, before commit after effects. |
| Explicit log allocation | content_usage.rs:take_log_prefix acquires content policy S → log policy S → tenant_retained_usage U before scripts.rs writes the bounded log; this is earlier than deferred finalization. |
| Audit deduplication | identity::insert_audit seed2 scoped key, then audit INSERT; denial-only seed0 TX never follows with run/chain acquisition. Audit content is bounded/redacted; do not infer no contention from immutability. |
| Terminal credential cleanup | secret_cleanup::release_terminal_credential_pins: exact terminal run U SKIP LOCKED → namespace Secrets quota X → pins delete/audit, one run per TX, no later chain/run. Read-only terminal eligibility is rechecked under row guard. No active execution authority or group ownership is assigned. |
| Provider cleanup | namespace Secrets quota X → provider intent U → audit/lease; commit before provider I/O, then fenced cleanup TX. No run acquired after provider intent. |
| Inspection/disclosure/results | payload_inspection/disclosure/database_results: exact run S → applicable namespace/membership/step S and namespace-wide reveal-grant advisory S (`payload-reveal-grants:{tenant}:{namespace}`, seed0; including absent grants) → audit, before values leave; no chain, group, next run or execution transition follows. |
| Telemetry/outbox/maintenance hints | telemetry projector updates unchanged-identity telemetry_pending without acquiring run/chain; outbox claimant owns outbox rows only. Maintenance execution discovery is read-only; actual run commands enter the applicable execution port. Purge sessions never acquires a run/chain afterwards. |
| Stable workflow/catalog identity | run/version/namespace, immutable payload/script/version and connection pins retain FK identity KS as applicable. Their mutation/purge services do not gain permission to acquire a live run after those catalog guards. |

## B1 constraints and evidence boundary

This candidate must be checked against current source before B0 closes. Prefix tables include
ordinary versus managed and root versus scoped differences. Existing scheduler and session
postwait tests, initial-child FK/quota barriers, chain/update/scope/approval/incident prefix
barriers and original fault suites are indexed in capacity-lock-inventory.md. Generic
Unavailable is not proof of PostgreSQL deadlock; only the retained actual deadlock observation
is described that way. These tests do not prove every absent edge impossible.

Two source inversions remain under focused investigation: managed receipt replay catalog→binding
versus registration binding→session→managed claim session→catalog, and managed rerun
catalog→terminal-source FK versus terminal cleanup source U→catalog. No reproduced deadlock
is claimed from these source edges. B0 remains candidate while their consequences are checked.

A future group lock cannot simply be appended after own run/link, parent KS or parent NK:
those earlier guards can block another writer that already holds the group. B1 must state its
placement before all conflicting existing-run guards and account for the chain-first boundary,
shared ancestry identity, own-run authority, outgoing/incoming links and initial creation. New
roles, backfill, mixed old binaries and eligibility are separate C/B tasks. No source observation
here enables general nesting, creates an SQL group or accepts an offer/parking/fairness protocol.
