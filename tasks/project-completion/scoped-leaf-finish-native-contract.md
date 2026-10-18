# Dormant flat OrdinaryLeafV1 native Finish transaction

Bounded design draft, 2026-10-18. No implementation or tests executed by this author.
Refs: T014-014..017/040; A1/B0/B1a; reviewed scoped-leaf-finish-{binding,schema,
application-contract,flat-context,audit-hint-contract}.md and canonical-leaf-reader-contract.md.
Source reads included the constitution, feature014 spec/plan/tasks, ChildRoot capacity_finish.rs,
leaf_certification.rs, capacity.rs, migrations20261210/11/12 and their native test fixtures.
The root is concurrently changing this worktree; freeze the integrated source before verification.

## Bounded decision and dependencies

Implement one dedicated PgScopedLeafFinishStore, owning its entire transaction and implementing
ScopedLeafFinishStore. A constructor accepting only PgPool is sufficient. No DefinitionCache,
source-certifier field, transaction escape hatch, generic capacity mutation API or profile selector.
Keep the existing ChildRoot adapter and semantics unchanged. The new store has no production caller,
public endpoint, scheduler discovery, production fixtures, migration or release INSERT grant.

Required executable prerequisites are the reviewed application API (including UnsupportedRunContext),
crate-private canonical reader extraction and canonical scoped hint vocabulary. Verify these on the
integrated tree first; documentary review and another agent's test claim are not executable evidence.
Use only actual native certifications from the current flat producer family for positive source
evidence. Owner-created capacity memberships remain explicit fixtures. A fixture certificate proves
decoder behavior only. The consumer reads certification metadata; it never invokes certification,
re-proves the source, alters certification or manufactures a chain/head identity.

The source is a settled selected path in the original Run; the Run may still execute later work or
later fail. No terminal Run condition is added. First application changes the selected contribution
to an existing group, without settling another member, moving possession to a recipient, changing
usage/accounting or claiming execution eligibility. A flat original Run is not a proof of flat group
topology or descendant settlement. No B1/B2, child/chain closure or nested-publication acceptance.

## One transaction, in this order

Use command.fingerprint() before BEGIN; it performs the reviewed structural validation. Unsupported
path/owner/index and invalid/exhausted expectations therefore refuse before any SQL. Keep its exact
domain and encoding; never recompute from formatted SQL fields or change its operation-key scope.
Use BeginSafely::begin_safely and retain all acquired guards until COMMIT/ROLLBACK.

| Stage | Exact bounded statement / interpretation |
| --- | --- |
| Discovery | SELECT one member by tenant_id, namespace_id, group_id, member_id. Project member_kind, chain_id, original_run_id, root_owner_node, iteration_index and state/finished-at-null metadata only. No FOR UPDATE; this is an identity candidate, not authority. |
| Own-chain prefix | Reuse lock_member_chain_sql(tx, tenant, namespace, originalRun). Its exact membership join locks the applicable execution_chains row FOR UPDATE; it neither follows a head nor reports nonmembership. |
| Own Run | SELECT workflow_version_id, run_version, parent_run_id IS NULL AS parentless, plus the bounded trace projection below FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE. Never lock a run discovered from a different binding. |
| Selected G | SELECT revision, epoch, active_members, possession, holder_member_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE NOWAIT. Only 55P03 from this statement maps to Busy. |
| Binding reread | Repeat the exact discovery SELECT under G and check the immutable binding again. G has no group-wide namespace. |
| Current authority | Reuse locked_scheduler_time_sql with originalRun and the command's current owner/lease; its own-shard FOR SHARE precedes the fresh server clock comparison. None maps to SchedulerFenceLost, including wrong shard, oversized generation, owner/generation mismatch and expiry after a wait. |
| Flat context | Require locked parentless=true AND NOT EXISTS(SELECT 1 FROM chain_segments WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3). Use a fresh scalar query in this same transaction. Any parent or membership gives UnsupportedRunContext. |
| Source | Call read_canonical_leaf_success(tx, &identity, actual guarded workflow_version_id, actual guarded run_version). None gives SourceEvidenceMissing. Validate supported exact metadata before consulting a Finish receipt. |
| Operation receipt | SELECT the explicit metadata columns described below FROM capacity_group_finish_receipts WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3. Never add kind/namespace/path to this key. |
| Creation CAS | Only on receipt miss: compare header revision, then epoch, then guarded original Run revision to expectations. Return the corresponding stale error in that order. |
| Selected/holder/header | Only after CAS: require the selected ACTIVE state, then restore the header with its optional exact holder fact and apply CapacityGroup::finish. |
| Writes | Update only selected G and selected member, append audit, scoped hint and receipt in this transaction, then COMMIT. |

Discovery uses namespace in WHERE: wrong/foreign tenant or namespace is Missing, as is an absent
member/Run/G. A known CHILD_ROOT binding or a well-formed different scoped originalRun/owner/index
is BindingMismatch. A malformed stored discriminator/binding is ContradictoryProjection; do not
coerce NULLs into an alternate variant. Check binding before entering a caller-supplied Run and again
under G. Discovery must not read another namespace's path and disclose it as a mismatch.

This yields precedence: structural -> discovery -> chain/Run/G -> binding -> current authority ->
flat context -> canonical source -> fingerprint/receipt -> group revision -> epoch -> Run revision ->
active/projection -> counter advance -> writes/COMMIT. Storage errors can interrupt any statement;
a rollback failure is sanitized Storage instead of the earlier semantic result. Malformed stored
fields refuse when their stage is consumed; do not eagerly validate header/member state ahead of
receipt replay. Current guarded Run pin/revision must be decodable before the source reader.

The post-guard chain probe acquires no chain lock. If it sees membership missed by prefix discovery,
refuse and roll back; never take a chain lock after Run. Parent presence and every actual membership
are refused, including a legacy child without a chain, root-chain member, historical segment, FINAL
chain and settled/detached child. Current atomic new-Run membership and immutable ancestry are the
stability preconditions; no gap/advisory lock, topology scan or old-writer proof is inferred.
The scheduler fence is valid at its post-lock sample, not promised continuously through COMMIT.

## Closed decoding and original receipt

Use fallible Row::try_get and checked conversions throughout this new adapter. Unknown state or
possession values, impossible NULL combinations, negative/out-of-range counters and malformed
fixed metadata are ContradictoryProjection, never panic, unwrap, lossy cast or default branch.
The Run's actual nonnegative i64 revision is supplied unchanged to the canonical reader; run_version=0
is legal storage, but cannot validate a positive certified revision. A malformed guarded source pin/
revision is ContradictorySource. Do not substitute the caller's expected Run revision into the reader.

Canonical-source decoding is exclusively the extracted helper's existing behavior, including profile
first, exact qualified key, NodeId/digest/sequence/record checks, actual pin equality and certified
revision <= actual Run revision. It reads no scope, checkpoint, step, payload, journal or audit row.
Preserve its guarded timestamp projection before SQLx decoding; no raw out-of-range timestamp.

Map helper UnsupportedStoredProfile, ContradictorySource and sanitized Storage one-for-one; None
maps only to SourceEvidenceMissing. The helper's Rust return type is broader than its reviewed
emitted subset. Explicitly enumerate its other existing variants (UnsupportedPath, Missing,
UnsupportedShape, SchedulerFenceLost, StaleRunRevision, SourceNotClosed, SourceEvidenceMissing,
InvalidRevision, CommitOutcomeUnknown) as an internal reader-contract violation with a fixed,
sanitized Storage diagnostic. Never panic, fabricate success, return a source-creation refusal or
use a wildcard. A new helper enum variant must fail exhaustiveness and trigger consumer review;
extending the helper's actual emitted subset needs an explicit contract change.

The receipt projection contains fingerprint, member_kind, namespace_id, member_id, chain_id,
head_run_id, accepted_chain_revision, original_run_id, root_owner_node, iteration_index,
expected_group_revision, expected_group_epoch, accepted_run_revision, resulting_revision,
resulting_epoch, resulting_active_members, resulting_possession, resulting_holder_member_id,
finished_at (guarded below), audit_id and outbox_id. Lookup keys bind tenant/G/operation.

First decode fingerprint as exactly32 bytes. A well-formed different fingerprint is
FingerprintConflict, including a receipt of the other member kind. Do not decode its unrelated
result first. Malformed fingerprint length/type is ContradictoryProjection. For matching bytes,
require the complete SCOPED_PATH variant, all three chain fields NULL, exact namespace/member/
originalRun/parsed ordinary owner/index and all three expected counters (accepted_run_revision is
the original expected Run revision). Any discrepancy is ContradictoryProjection.

Decode the immutable result against all retained schema semantics, not merely the weaker checks
in today's ChildRoot replay() implementation:

- Expected group revision/epoch and accepted Run revision are positive signed-range values;
  expected epoch <= expected revision. Result revision equals checked expected revision+1.
- Result epoch is positive, <=result revision, >=expected epoch, with delta exactly0 or1.
  Count fits u32, is nonnegative and <=result revision.
- HELD requires positive count, a holder different from the finished member and epoch unchanged.
  VACANT requires positive count and no holder, with the permitted epoch delta0 or1.
  RETIRED requires count0, no holder and epoch exactly expected epoch+1.
- finished_at is non-NULL and safely representable. Project
  CASE WHEN isfinite(finished_at) AND finished_at<TIMESTAMPTZ '10000-01-01 00:00:00+00'
  THEN finished_at END AS finished_at before SQLx receives it; NULL is contradiction. Preserve
  representable BC timestamps, as the canonical reader does. Decode audit/outbox UUIDs fallibly.

Return the stored original result/time/IDs with replayed=true. Do not compare it to today's header,
require its historical holder still ACTIVE, rerun A1, check current expected CAS or substitute current
time/counters. Do not require an audit/outbox row to remain present: delivered outbox retention is
independent and there is intentionally no such FK. Source/context/current authority still precede
this replay. A same-key replay with changed expectations conflicts through its fingerprint.

## A1 restoration, counters and exact writes

On receipt miss, decode the current positive signed-range header revision and compare it first;
then decode the positive signed-range epoch and compare it; then compare the already decoded actual
Run revision. Decode the remaining header fields only after these CAS checks. Selected state is
closed ACTIVE/FINISHED: valid FINISHED is InactiveMember;
unknown state or an inconsistent state/finished-at-null pair is ContradictoryProjection. An ACTIVE
selected row has no finished_at. Query only the timestamp's NULL boolean for selected/holder facts.

Decode possession as HELD(Some(member)), VACANT(None), RETIRED(None) only. If HELD by the selected
member, reuse its guarded row. Otherwise SELECT state, finished_at IS NULL from members by exact
tenant/G/holder: at most one extra row, no namespace filter or run lookup, because a group may span
namespaces and kinds. Require ACTIVE/no timestamp and construct the genuine CapacityMemberSnapshot.
For nonheld possession pass None. Preserve CapacityGroup::restore and finish unchanged.

Map CapacityGroupError exhaustively: CounterExhausted -> CounterExhausted; InactiveMember/Retired ->
InactiveMember; ForeignGroup, MemberAlreadyKnown, NotHolder, Occupied, InconsistentSnapshot,
StaleRevision and StaleEpoch -> ContradictoryProjection (the real CAS was already checked).
Validate every domain-to-SQL counter conversion before any DML. A u64 advance beyond i64::MAX
refuses even though pure A1 can represent it. No saturation, reset or partial header advance.

Result cases remain exactly A1: nonlast selected holder becomes VACANT and increments epoch;
nonholder preserves another holder and epoch; nonlast already-vacant group stays VACANT with the
same epoch; the last member retires and increments epoch once. Every first Finish decrements count
once and increments revision once. Historical holder identity remains usable after it later finishes.
Restoration validates the bounded header/fact, not an independently counted registry.

Keep the ChildRoot write order for comparable failure points: G -> selected member -> audit ->
outbox -> receipt. G UPDATE qualifies tenant/G and old revision/epoch. Member UPDATE qualifies
tenant/G/member/namespace, SCOPED_PATH, chain_id IS NULL, exact originalRun/owner/index,
state='ACTIVE' and finished_at IS NULL. Each must affect exactly one row; otherwise contradiction
and full rollback. Use the guarded database time for member finished_at, audit and receipt result.

INSERT the scoped receipt with an explicit column list: all common existing fields, member_kind
and scoped binding fields; either explicitly NULL the chain fields or omit them (NULL defaults).
Never populate chain_id/head_run_id/accepted_chain_revision with stand-ins. Preserve all three
group-wide uniqueness keys across kinds; no ON CONFLICT DO NOTHING/UPDATE. An actual unexpected
uniqueness failure is sanitized Storage and full rollback, not a manufactured replay.

Use AuditRecord::engine, CapacityMemberFinished, CapacityGroup/G UUID, the original tenant,
guarded time/trace and exact reason `ordinary leaf path capacity member finished`. Insert the
fixed noncollapsing audit directly as the certifier does. identity::insert_audit/store_error can
log raw driver text; do not route this boundary through them. Reuse the genuine raw guard helpers,
canonical reader, domain types and telemetry vocabulary; do not introduce empty snapshot mappers.

Append exactly one capacity.scoped_member.finished outbox row with aggregate_id=G and the canonical
closed CapacityScopedMemberFinished payload. Exact keys: tenantId, namespaceId, groupId, memberId,
originalRunId, rootOwnerNode, iterationIndex, resultingRevision, epoch, retired, optional traceId.
Identity is the validated retained binding; result fields are the newly computed receipt result.
Use a fresh stable UUID, persist it in the receipt and retain dispatcher Nats-Msg-Id behavior.
The original Run's trace is constrained by runs_trace_id_format. Additionally project these two
columns in the guarded Run query, without copying arbitrary malformed text into Rust:
`(trace_id IS NULL OR trace_id ~ '^[0-9a-f]{32}$') AS trace_valid` and
`CASE WHEN trace_id ~ '^[0-9a-f]{32}$' THEN trace_id END AS trace_id`.
After all first-application CAS/A1/counter checks and before any DML, require trace_valid=true and
fallibly decode the optional trace (non-NULL must be exactly32 lowercase hex); otherwise return
ContradictoryProjection. NULL becomes JSON null or omission, as the message permits. A malformed
non-NULL value is refused, never silently turned into absent trace. Matching receipt replay returns
before this creation-only check: it uses no current trace and produces no new audit/hint. This adds
no replay or authority precondition and no query. Do not truncate, normalize or substitute caller
text. No chain/head, payload, credentials, operation key,
holder ID or source body enters the hint. Actual produced rows must pass the canonical schema.

## Locks, bounds and diagnostics

There are at most eleven explicit read/guard statements before first-application writes: discovery,
own-chain, Run, G, binding reread, two scheduler statements, context EXISTS, certificate, receipt,
and optional different-holder fact. Each query has a fixed primary/unique/index-qualified cardinality;
no history/count/all-members/sibling/payload/definition hydration. Source and receipt are ordinary
SELECTs, relying on immutable retention and retained own Run/G guards, not locks they acquire.

NOWAIT covers selected-G tuple contention, not the earlier blocking chain/Run prefix or DDL/relation
locks. Inventory implicit waits too: member state participates in the ACTIVE unique reference key,
so ACTIVE->FINISHED takes UPDATE strength and triggers the reverse deferred holder FK; receipt
group/member/historical-holder FK checks remain inside guarded G. The member's original-Run FK
already references the held Run and is unchanged by Finish. The scoped receipt's ChildRoot chain
FK is NULL/inapplicable. Audit/outbox reference tenant identity; uniqueness/index/DDL waits remain.
No new certificate/source FK, advisory lock, parent/link/foreign-run lock or retained-usage delta.
No global deadlock-free or wait-free claim follows. Test the real deferred checks at COMMIT.

Classify all raw SQLx failures locally: expose only a five-character ASCII-alphanumeric SQLSTATE,
otherwise a fixed message. Never format raw driver detail, constraint message, SQL/binds, command,
record or scheduler owner. Treat row decoding failures as semantic contradictions as above.
Reuse the sanitized StoreError returned by the canonical reader without logging its source chain.
Keep safe application Display/Debug. No logging-based authority or extra telemetry reads.

Observations follow the reviewed vocabulary: exact retained span handle, identity fields only
(originalRun, no owner/chain/head), six exhaustive outcomes; applied only after acknowledged first
COMMIT, replayed only after confirmed replay COMMIT, Busy, refused, storage_error and unknown.
The fixed completion log carries finite outcome only. Attempt count/duration once per returned
attempt; unlabeled transition count only for acknowledged first application. Cancellation returns
nothing and is uncounted. A committed unknown followed by replay may leave this metric uncounted.

## Rollback, uncertainty and recovery

Every apply error consumes the owning transaction through rollback; no savepoint continuation or
retry with retained locks. A rollback failure returns sanitized Storage. Confirmed COMMIT aborts
are only server SQLSTATE class23, 40001 or40P01, matching existing ChildRoot semantics. Every other
COMMIT failure is CommitOutcomeUnknown{operation_key}; never assume abort from EOF/protocol/
cancellation/unknown-resolution errors. Return success only after acknowledged COMMIT. A dropped
future may have dispatched COMMIT and produces no reliable returned outcome.

Recovery retains the complete original command, key, identity and expectations; renew only scheduler
authority. Reenter a fresh transaction through the same prefix. For this flat consumer the unresolved
original operation blocks same-Run reconciliation at Run, before G; a distinct-Run member can reach
G and receive Busy. Timeout/cancellation/refusal leaves the prior unknown operation unresolved.
A supported-source matching receipt proves the original result even after later header/Run changes.
Absent receipt after resolved guards allows the same command to apply only if its original CAS still
holds. A stale refusal does not authorize changing the old key's fingerprint. A later independent
operation requires fresh discovery and its own new logical key after the prior outcome is resolved.

## Minimal sequential implementation and evidence units

1. **Prerequisites / fresh baseline.** Root completes and reruns application checks, canonical-reader
   extraction controls and scoped vocabulary checks. Preserve ChildRoot native and hint baselines.
   No artificial failing test for the behavior-preserving reader extraction. No adapter until these
   executable prerequisites are integrated.
2. **First native transaction.** Add focused integration tests first for actual public flat workflow
   -> native certifier -> owner-seeded membership -> new runtime-role Finish. Implement the one store
   and registration. Prove first apply/duplicate original-result replay, source absence, current fence,
   context refusal, ordered CAS/inactive checks, all A1 holder/vacancy/retirement cases and signed-counter
   exhaustion. Assert unchanged actual selected source/certificate, Run revision/journal/steps and
   nonselected members, plus exactly one linked receipt/audit/hint with the canonical payload. Include
   a Run still doing later work and replay after legitimate later failure.
3. **Closed decode and qualification.** Exercise exact tenant/namespace/originalRun/owner/index,
   same-tenant two namespaces in one G, cross-kind same-key FingerprintConflict and full receipt
   checks. Wrong actual pin/future certified revision must refuse before an existing receipt; missing
   source likewise. Use owner-inserted rows only for clearly labelled decoder fixtures. Schema-valid
   out-of-range receipt time needs a native refusal plus 9999/BC positive controls. For schema-invalid
   stored decoder controls, use only isolated disposable corruption fixtures or module-local SQL row
   decoder tests; preserve production guards and do not claim arbitrary-owner corruption resistance.
   Test malformed/unknown/null branches affected by the new decoder, not a duplicate of every schema
   constraint. Use role-denied source/history tables or SQL observation to prove the metadata-only
   read, with real native positive control. Prove raw diagnostic canaries never appear in Display,
   Debug, logs or spans.
4. **Real waits and confirmed failure.** Held-lock barriers observe own-chain-before-Run on an
   unsupported member, Run-first entry for actual flat members, Busy rollback releasing prefix,
   duplicate serialization, both distinct-member completion orders, and unchanged-row scheduler
   wait/expiry on first apply and replay followed by fresh-authority success. Refused context must
   not read certificate/receipt (a denied SELECT probe is a useful deterministic oracle). Test actual
   certifier/Finish and current later-source-writer/Finish ordering on the same Run. Inject failures
   at audit, outbox, receipt and deferred COMMIT; snapshot proves no orphan effects and same-command
   fresh success. Do not use sleeps as ordering evidence or invent a retained-quota wait.
5. **Unknown COMMIT and returned observations.** Reuse CommitAckLossProxy with a test-only deferred
   receipt trigger and held advisory gate: both actual committed and aborted server outcomes, lost
   reply while Run/G/shard guards remain held, same-command Run waiter/cancellation, distinct-Run G
   Busy, then fresh-pool/store reconciliation of the unchanged command. Keep cold-store recovery
   distinct from actual process restart; reuse the existing child-process pattern before claiming
   process-death coverage. Exercise all six real outcomes, filtered-span parent safety and abandoned
   calls. Finish with exact affected ChildRoot/source/schema/privilege/contract regressions and
   format/strict-Clippy/architecture checks on the root's frozen merged tree; record actual commands,
   outputs and failures. These tests close this local dormant unit only.

Tests remain test-first for new behavior, using real repository-owned disposable PostgreSQL and
explicit sagat_app pools even in default suite mode. Root alone runs the tests and records evidence.
Private external CARGO_TARGET_DIR, disk check and machine build limit remain required. No result is
claimed by this design draft and no canonical task may be checked from it.

## Test-only privileges and actual remaining decisions

In each isolated test database, the owner seeds only capacity identities/header and gives the native
certifier temporary INSERT on leaf_collection_success_certifications to create genuine evidence.
Revoke that source INSERT before invoking Finish; source SELECT is enough. The dormant scoped
Finish INSERT needs only the four extension columns member_kind, original_run_id, root_owner_node,
iteration_index in addition to the existing20-column ChildRoot receipt grant. Grant those four only
inside the labelled native fixture, not in a migration or startup code. Existing header/member UPDATE,
Run/chain/shard guard and audit/outbox privileges suffice; test actual effective privileges/current_user.
Keep production/default-role probes proving scoped receipt INSERT, certificate INSERT and group/member
creation denied. No superuser adapter, RLS/GUC claim, trigger disabling in the native positive path or
permission broadening outside disposable tests. Fault-injection DDL belongs only to the fixture owner.

No unsettled product decision was found that prevents this fixture-only native transaction after its
three executable prerequisites pass. Namespace opacity, exhaustive impossible-reader errors and safe
audit SQL above are explicit implementation choices derived from the reviewed contracts; review them
as part of this bounded unit rather than silently copying the older ChildRoot adapter.

Actual production gates remain unresolved and outside this driver: durable caller intent or exact
reconstruction, released source/scoped INSERT authority, C0 retention and old/mixed writers, general
chain/child/nested relationship closure, creation/join/handoff/Acquire/Vacate, admission/accounting,
discovery/fairness and full B1/B2/resource/release acceptance. Do not invent their answers, infer
chain-source acceptance from a prefix lock or use this flat gate as proof of descendant settlement.


## Bounded review and executable prerequisite checkpoint

At3f6b2973, one fresh independent Astra/xhigh read-only review found no issue in this
transaction contract. This is not a fourth whole nested-protocol review. The user already
declined an external second review; no external model-review CLI was invoked.

Root executed the prerequisites: application6/6 at2b53a658, canonical reader21passed/1private
probe ignored in direct enumeration atc0b38fbc, and hint3/3, AsyncAPI4/4, canonical CLI valid
plus existing native capacity Finish33/33 at3f6b2973. Exact evidence remains in those commits.
This checkpoint approves only the next dormant implementation unit, not its unrun native
tests, source authority, caller, runtime grants, C0 or any broader B1/B2/runtime task.


## First dormant executable unit — 2026-10-18

B1b.2b.13 implements PgScopedLeafFinishStore without a production caller or release grant.
Four real PostgreSQL tests pass using explicit sagat_app pools, genuine public-source native
certificates and labelled owner membership fixtures. Actual Finish on two genuinely certified
indices changes G; later legitimate Run failure still permits the first original receipt/result.
A1 holder/nonholder/vacant/last results preserve nonselected members; exact member/audit/receipt
time and canonical actual outbox metadata are checked. Missing source/current stale authority/
root-chain context, ordered CAS including inactive fixtures, and signed overflow refuse atomically.

Natural pre-export E0432 RED precedes implementation. Two independent implementation slices
found no issue. Test review found three actionable oracle issues, fixed before the first run;
its second fresh review found none. Native4/4, strict adapter/all-target and integration Clippy,
fmt and architecture pass, with1091 frozen Rust/SQL/Protobuf/Cargo/canonical-contract inputs
unchanged. Detailed qualification/corruption/metadata-only probes, complete context matrix, real
wait/failure barriers, unknown-COMMIT/process recovery and returned observations remain pending
units14..17. Instrumented code alone does not establish those guarantees or broader B1/B2/C0.
Evidence: evidence/scoped-finish-native-first-20261018/manifest.json.


## Canonical source ordering and denied-read metadata controls — 2026-10-18

B1b.2b.14a adds3 actual PostgreSQL tests. After genuine native source certification and first
Finish, owner-only schema-valid wrong pin/future revision and exact certificate removal refuse
before matching/conflicting receipts; current stale authority refuses before missing source.
Per-call snapshots precede repair, then exact restored-source replay/conflict controls pass.
Only the named immutable certificate guard is temporarily disabled by the fixture owner and
restored ALWAYS before native entry. No definition row/guard is modified.

With certificate SELECT/no certificate INSERT and audit INSERT/no audit SELECT, native apply
and replay succeed after actual42501 probes deny reads from11 source/history/definition/audit/
payload/item tables. This is concrete denied-read evidence combined with the reviewed bounded
queries, not a statement-level audit of every possible SECURITY DEFINER dependency. Three final
tests pass, Clippy/fmt/architecture pass and1092 frozen inputs are unchanged. The earlier E0521
helper compilation error and initial nine-table pass are retained separately. ReviewC1 added
the omitted payload/item tables; freshC2 found no issue. Broader14b..d/15..17 gates remain open.
Evidence: evidence/scoped-finish-source-controls-20261018/manifest.json.


## Binding and schema-valid receipt controls — 2026-10-18

B1b.2b.14b/.14c add3 qualification and2 receipt tests. The integrated scoped family passes12/12
(4first-native,3source,3qualification,2receipt). Hidden tenant/namespace/group probes deliberately
disagree with the stored binding so unfenced discovery cannot be masked by later Missing. An
actual other-namespace holder-member probe returns opaque Missing; native selected Finish
preserves that holder, its uncertified alternate Run and epoch. Differing-fingerprint ChildRoot
receipt reuse returns FingerprintConflict before unrelated decoding; a fresh key applies/replays.
No statement-level foreign-Run SELECT instrumentation is claimed.

Labelled receipt-owner decoder fixtures keep all CHECK/FK constraints validated and active,
disable only the named append-only guard and restore ALWAYS before native entry. Exact matching
fingerprint plus year10000 refuses, differing fingerprint takes precedence, and9999/BC controls
replay exact stored time. Schema-valid counter-pair/accepted-Run/coherent-binding discrepancies
refuse atomically; restored native fields replay the original result. No owner-forgery resistance.
Three bounded qualification reviews fixed hidden-discovery oracles and ended None; receipt
reviewNone. ActualE0282 helper compilation failure and fix are retained. Clippy/fmt/architecture
pass, with1094 frozen inputs unchanged. Residual14d remains for an exact-fingerprint ChildRoot
variant, distinct from the already tested differing-fingerprint conflict;15..17 remain open.
Evidence: evidence/scoped-finish-binding-receipts-20261018/manifest.json.


## Current-schema decoder completion — 2026-10-18

Local B1b.2b.14/14d close after a fresh bounded inventory and matching-fingerprint cross-kind
control. A schema-valid owner-only ChildRoot receipt with exactly the scoped fingerprint
reaches decoding and returns ContradictoryProjection, preserving exact source/capacity
snapshots. A fresh key with the same scoped fingerprint applies once and its genuine scoped
receipt replays exactly. All CHECK/FK/append-only guards remain intact; this fixture does not
claim a hash collision or native ChildRoot finality.

Root's complete current scoped family passes13/13 in86.09s wall (21.12s harness), with all1095
frozen executable/contract inputs unchanged. Strict integration Clippy16.17s, fmt4.22s and
architecture0.43s/24crates pass. Fresh Astra review found no substantive issue. Inventory
separates current-schema guarantees from defensive changed-schema branches. Actual wait/fault,
commit recovery, diagnostics/OTLP units15..17 and production/C0/nesting gates remain pending.

Evidence: tasks/project-completion/evidence/scoped-finish-decoder-residual-20261018/manifest.json.


## Actual native entry barriers — 2026-10-18

Local B1b.2b.15a adds3 tests witnessing actual Run-first entry and selected-G Busy rollback
(released Run while G still held), exact duplicate commands serialized into one apply/one
original-result replay, and applicable-chain-before-Run followed by context refusal before
actually denied source/receipt reads. Chain membership is an owner-labelled context fixture,
not evidence of non-flat source closure. Source/capacity snapshots and linked fact counts
are preserved; ordering uses actual pg_stat_activity/blocking graphs and NOWAIT probes.

Root's full scoped family16/16 passes in61.22s wall (25.97s harness), strict integration
Clippy9.43s, fmt4.19s and architecture0.29s/24crates. All1096 frozen executable/contract
inputs match. Fresh bounded Astra review found none. Production adapter unchanged;15b..e,
16/17, production/C0/nesting and canonical runtime/release tasks remain open.
Evidence: tasks/project-completion/evidence/scoped-finish-entry-barriers-20261018/manifest.json.


## Post-shard-wait current authority — 2026-10-18

Local B1b.2b.15b. Two tests use real renewal to shorten the current lease, hold its committed tuple unchanged, observe the native shard-row wait while still live, and prove original Run/G guards retained with NOWAIT probes. Database-clock expiry occurs before releasing that unchanged row. Both first application and exact receipt replay return SchedulerFenceLost with exact source/capacity snapshots and no extra effects. Actual fresh owner/generation changes only renewable command authority; the unchanged logical fingerprint applies once or returns the exact original receipt.

Actual focused unchanged-row shard expiry controls 2/2 in44.33s wall (8.99s harness); strict integration Clippy9.31s, fmt4.2s, architecture0.28s/24crates. All1097 frozen executable/contract inputs match.
Fresh bounded Astra review found no remaining substantive issue.
No production implementation changed. Previous complete scoped16/16 result remains separately scoped to2426083b; this run is focused2/2, not an aggregate18-test pass. Remaining15c..e,16/17 and production/C0/nesting/canonical runtime/release gates stay open. No sleep ordering, forced expiry UPDATE during wait, or non-flat source claim.
Evidence: tasks/project-completion/evidence/scoped-finish-shard-expiry-20261018/manifest.json.
