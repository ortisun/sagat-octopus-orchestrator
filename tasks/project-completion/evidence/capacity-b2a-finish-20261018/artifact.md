# Dormant ChildRoot Finish storage — B2a

Status: reviewed scoped contract, unimplemented. Baseline49ee1997; consumes A1, checked restoration, B1a and
B1b.1. One native method, existing groups populated only in explicit owner fixtures, no scheduler
call site, public endpoint, production seed, admission API, usage projection or nested activation.

## Tables and authority boundary

The group has tenant identity and no group-wide namespace. Every predicate includes exact tenant
and applicable namespace/member binding. Follow the existing core no-RLS design: no tenant GUC or
RLS protection is claimed. Composite FKs and explicit native runtime-role cross-tenant tests guard
this trusted-server database boundary; sandbox workers receive no database credentials.

`capacity_groups`: PK(tenant_id,group_id), revision, epoch, active_members, possession, holder ID.
Revisions/epochs are bigint1..i64::MAX; active count0..u32::MAX. Require epoch<=revision and
active_members<=revision; HELD requires positive count/non-null holder, VACANT positive/null,
RETIRED zero/null. Identity is immutable; RETIRED is absorbing. No DELETE/TRUNCATE, including
tombstones. Domain u64 values/result increments outside SQL range produce typed CounterExhausted
before DML, never wrap/reset/truncate.

`capacity_group_members`: PK(tenant_id,group_id,member_id), immutable namespace_id, kind CHILD_ROOT,
chain_id, state ACTIVE|FINISHED and optional finished_at. The empty child root path is intrinsic;
no structured path is accepted. Unique(tenant_id,group_id,namespace_id,chain_id) prevents aliases
inside one group. Composite FK to that tenant/group and (tenant_id,namespace_id,chain_id) to
execution_chains. ACTIVE has no finished_at; FINISHED requires it, is irreversible and immutable.
Rows persist as non-reusable tombstones; no DELETE/TRUNCATE or identity rewrite. Owner run is
resolved from the locked chain head, not a mutable registry projection. The port never counts or
hydrates the registry: read selected member and at most one distinct holder fact.

The current header holder must reference an ACTIVE member in the same tenant/group. Enforce this
with a deferred composite FK to member identity/state and a generated ACTIVE holder-state marker
(null without holder). Document the reverse member-state FK check and its UPDATE-strength lock:
state participates in an eligible unique reference key. These edges stay within the already guarded
G; member→chain identity insertion is fixture-only here. This does not prove the aggregate count
against every registry row: atomic A1 writers maintain that projection, checked by independent test
reconciliation. Malicious owner SQL or arbitrary disk corruption is not a new guarantee.

`capacity_group_finish_receipts`: append-only operation receipt and Finish journal entry. Unique
(tenant_id,group_id,operation_key), (tenant_id,group_id,resulting_revision), and
(tenant_id,group_id,member_id): a member finishes once even across keys. Internal operation_key is
UUID; fingerprint is32bytes SHA256 over a versioned canonical command: tenant/namespace/group/
member/chain/head identities plus expected group/chain/run revisions/epoch. Exclude renewable
scheduler credentials and trace metadata. Persist fixed fields for the exact resulting header,
completed head, accepted chain/run revisions, expected group revision/epoch, timestamp and audit/
outbox IDs, not arbitrary JSON values. Historical result-holder references identity only, never
ACTIVE state: that other member may later finish. Composite group/member/chain identity FKs cannot
address another tenant/group. Audit/outbox IDs do not retain delivered outbox rows through an FK.

A replay returns the original result even after later header changes; it grants no current execution
authority. This journal records accepted Finish transitions from the explicit fixture baseline, not
production Create/Join history or quota admission. Classify bounded control metadata in the canonical
storage-classification contract. Already-terminal runs/chain segments/chain outcome/run_events/
run_version/steps/telemetry stay unchanged; this is not another run.finished.

Grant sagat_app SELECT and only necessary header/member UPDATE columns, plus receipt INSERT/SELECT;
no header/member INSERT, DELETE or TRUNCATE. Actual migrations/harness privileges must be inspected
and tested, not inferred. Immutable/absorbing/append-only triggers protect the declared metadata.
Guarded downgrade refuses any capacity row, including tombstones/receipts. No production purge or
backfill is implemented; later retention/rollout needs a compatible contract.

## Exact native operation

CurrentChildRootFinishStore::finish_current_child_root(command), implemented by a dedicated
PgCapacityFinishStore owning/consuming its entire TX. No reusable TX handle, generic create or
unchecked mutation interface. Commands carry exact identities/expected revisions/epoch and current
SchedulerAuthority, never settled/sourceTerminal/eligibility flags or caller-selected values.

1. begin_safely, exact read-only candidate binding discovery; not authority.
2. Applicable own chain → exact own run guards, with bounded scalar chain/head/segment/run reads.
   lock_member_chain preserves entry order only; it does not authorize a head or hydrate history.
   Reject a discovery/head mismatch, never forward to a successor.
3. One exact header FOR UPDATE NOWAIT. Only SQLSTATE55P03 from this statement is typed Busy;
   explicitly consume/roll back the whole TX, no savepoint/retry/remaining handle. Absent group
   never means vacancy, quota room or legacy-flat fallback.
4. Recheck exact immutable tenant/namespace/group/member/chain association under G.
5. Retain own-shard guard and use locked_scheduler_time's actual postwait DB clock, including on
   terminal runs and receipt replay. Validity is at that sample, not continuous through later waits
   or until commit.
6. Prove current own head, own segment FINAL/no successor/completed_at, terminal run/finished_at
   and matching non-null chain final outcome. CONTINUED/history/parent terminality are insufficient.
7. Same-key receipt lookup precedes expected group revision/active-member checks already changed
   by a successful Finish. Same fingerprint returns its stored result; conflict adds no Finish.
   New writes require all expected revisions and epoch to match current facts.
8. Read selected and optional holder fact, checked CapacityGroup::restore then A1 finish. Validate
   every domain→SQL conversion before DML. All group/member writers follow G before their rows/FKs.
9. Atomically update one header/member, append receipt/journal, truthful engine audit and closed
   outbox hint. Retain own chain/run/G/shard through actual commit, including deferred checks.

Only exact own run is mutation-locked. No foreign business run, parent link, other group, usage
allocation/release or root sweep. A holder registry fact is not authority to inspect its run.
No FK introduces another existing run; new member→chain FK insertion stays fixture-only. Include
header↔member reverse checks, receipt uniqueness and commit constraints in the lock inventory.

## Idempotency and errors

Busy, missing/binding mismatch, stale head/revision/epoch, non-final/contradictory projection,
fingerprint conflict, counter exhaustion, confirmed storage failure and CommitOutcomeUnknown are
distinct internal outcomes. Public wire error mappings are outside this dormant port.

The caller retains the full original key/fingerprint command for retry/reconciliation; the first
production caller will need a durable-intent or deterministic reconstruction contract before use.
Do not invent a new independent key after an uncertain response. A successful acquisition of G
(and own run) establishes that any earlier mutation holding those guards resolved; an unresolved
holder may block the ordinary chain/run entry before G is reached. Only contention at the G
NOWAIT statement is Busy; do not describe the earlier prefix as nonblocking. A reconciliation
timeout/cancellation/Busy that cannot establish the prior outcome preserves the original unknown
operation state, not evidence of rollback. Never retry while retaining the failed attempt's TX.
Under the guards, matching receipt proves commitment. Confirmed absence plus resolved prior TX
permits a same-command fresh retry if current preconditions still hold; contradiction fails closed.

Confirmed abort before commit/confirmed server abort leaves zero new effects. A potentially
committed operation with lost acknowledgement/cancellation/transport failure after COMMIT dispatch
is CommitOutcomeUnknown, not Busy or guaranteed rollback. Reconcile to committed or aborted state
with fresh current authority. Return success only after confirmed commit/reconciled receipt.

## Truthful observations and hint vocabulary

Audit action capacity.member.finished, resource capacity_group; bounded metadata only, no values,
SQL, credentials or arbitrary reason. Canonical OpenAPI audit fields are open strings. Outbox hint
capacity.member.finished has a closed reference payload: tenantId, namespaceId, groupId, memberId,
chainId, headRunId, resultingRevision, epoch, retired, optional traceId. It is a durable discovery
hint, not foreign-run scheduling authority or a new execution lease. Canonical AsyncAPI and actual
hint validation change before the port; no broker delivery/consumer guarantee is inferred here.

Use a closed capacity operation span and finite outcomes; IDs belong only in spans/logs, never
metric labels. Attempt outcomes distinguish applied/replayed/busy/refused/storage_error/unknown;
committed transition metrics increment after confirmed commit, not on replay/attempt. Lost commit
acknowledgement may leave the process metric uncounted; receipt/journal remains durable evidence.
No exact metric delivery guarantee is claimed.

Audit/outbox/fixed control metadata follow protected_system accounting and create no retained-usage
delta. Do not fabricate B1a's retained-usage wait: that gate stays pending for a later applicable
command. First Finish uses a labelled test-only deferred constraint COMMIT barrier/failure, observes
actual SQL/guards and tests unknown outcomes; this is not a retained-quota proof.

## Ordered implementation and real evidence

- Vocabulary: truthful domain audit/resource tags, canonical AsyncAPI closed hint and contract test.
- Schema: new20261210000000 migration pair, storage classification, native runtime-role constraints/
  privileges/tombstone/downgrade tests. Positive valid fixture before invalid-constraint assertions.
- Native port: dedicated application/PG modules and exact current real child FINAL tests; no runtime
  caller. B-held vacancy/A/C survival, C-held epoch preservation and last-member retirement.
- Barriers/recovery: chain-before-run, external G Busy releases guards, distinct own-run peers both
  orders, postwait shard expiry/fresh positive, stale discovery/handoff, duplicate/conflicting keys.
  Confirmed first abort has zero effects; successful fresh retry one result; uncertain commit
  reconciles both committed and aborted outcomes. Exercise a same-command reconciliation blocked
  on the first attempt's chain/run while COMMIT is unresolved, including timeout preserving Unknown,
  separately from a distinct member reaching G and receiving Busy. Inject actual audit/outbox/
  journal failures.
- Independent snapshots prove run journals/versions/steps, other groups and usage remain unchanged;
  indexed selected-row access, including same-tenant different-namespace members in one G. No
  history hydration/root sweep. Validate actual outbox payload, not merely a synthetic example.

Do not mark full B2/B1 complete from this first Finish. Production creation/admission/accounting,
scoped membership, Acquire/Vacate eligibility, handoff, cleanup/discovery/fairness, mixed old writers,
rollout, SDK58,100k and release gates remain pending. Nested runtime remains closed.

Schema decision review completed1bounded cycle with no concrete issues. Operation/recovery
review completed2bounded cycles: same-command chain/run wait versus distinct-member G Busy
corrected; final review found no remaining concrete issues, including postlock timing wording.
No SQL implementation or runtime/test acceptance follows from this decision review.
