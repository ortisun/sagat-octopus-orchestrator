# Scoped OrdinaryLeafV1 Finish application contract — next B1b.2b unit

Status: reviewed bounded interface decision only; no executable guarantee or nesting enabled.
Refs: T014-014..017/040; A1/B0/B1a; scoped-leaf-finish-binding.md and scoped-leaf-finish-schema.md.
This unit adds application types, structural checks and fingerprint tests only. No native adapter,
production caller, schema/privilege change, INSERT grant, group creation or release activation.
Dormant schema evidence is recorded separately at2deef834; this interface review does not
extend those results or close its dedicated downgrade unit.

## Exact additive API

Proposed module: application::scoped_leaf_finish. Existing types are imported, never copied:
LeafCollectionPathIdentity, SchedulerAuthority, CapacityGroupId, CapacityMemberId,
CapacityPossession, RunId, AuditId, StoreError, OffsetDateTime and Uuid.

```rust
pub struct ScopedLeafFinishExpectations {
    pub group_revision: u64,
    pub group_epoch: u64,
    pub run_revision: u64,
}
pub struct ScopedLeafFinishCommand {
    pub operation_key: Uuid,
    pub group_id: CapacityGroupId,
    pub member_id: CapacityMemberId,
    pub identity: LeafCollectionPathIdentity,
    pub expected: ScopedLeafFinishExpectations,
    pub scheduler: SchedulerAuthority,
}
impl ScopedLeafFinishCommand {
    pub fn validate(&self) -> Result<(), ScopedLeafFinishError>;
    pub fn fingerprint(&self) -> Result<[u8; 32], ScopedLeafFinishError>;
}
pub struct ScopedLeafFinishResult {
    pub group_id: CapacityGroupId,
    pub member_id: CapacityMemberId,
    pub original_run_id: RunId,
    pub revision: u64,
    pub epoch: u64,
    pub active_members: u32,
    pub possession: CapacityPossession,
    pub finished_at: OffsetDateTime,
    pub audit_id: AuditId,
    pub outbox_id: Uuid,
    pub replayed: bool,
}
pub enum ScopedLeafFinishError {
    UnsupportedPath, InvalidRevision, CounterExhausted,
    Busy, Missing, BindingMismatch, SchedulerFenceLost, UnsupportedRunContext,
    SourceEvidenceMissing, UnsupportedStoredProfile, ContradictorySource,
    FingerprintConflict, ContradictoryProjection,
    StaleGroupRevision { current: u64 }, StaleGroupEpoch { current: u64 },
    StaleRunRevision { current: u64 }, InactiveMember,
    Storage(StoreError), CommitOutcomeUnknown { operation_key: Uuid },
}
pub trait ScopedLeafFinishStore: Send + Sync {
    fn finish_scoped_leaf(&self, command: &ScopedLeafFinishCommand)
        -> impl Future<Output = Result<ScopedLeafFinishResult, ScopedLeafFinishError>> + Send;
}
```

The method is permanently fixed to OrdinaryLeafV1; no selectable profile or caller proof field.
Its frozen source profile excludes continuation policy and child/nested controllers.
Derive originalRun, tenant, namespace, root owner and index from the existing identity only.
No duplicated owner/index/chain/head/version/digest/sequence/settled flags enter the command.
Validate depth1, index0..9999, and owner through NodeId::parse(owner.as_str()); internal NodeId
can contain derived suffixes, so depth1 alone does not prove the schema's ordinary ASCII bound.
Unsupported owner/path is UnsupportedPath. All three expectations must be positive and fit i64;
zero or epoch>group_revision is InvalidRevision; values above i64::MAX are CounterExhausted.
These are structural checks, never proof of current scheduler ownership, source or CAS equality.

Use a separate scoped result because existing CapacityFinishResult.head_run_id has independently
meaningful ChildRoot semantics. Never populate it with originalRun, rename it or map through it.
The additive result reuses common field types; no empty wrapper, generic identity VO or new ID type.
The separate closed error set excludes ChildRoot StaleHead/StaleChainRevision/NotFinal and source
certification-creation errors. Safe Display/Debug must omit raw StoreError/command/owner text;
explicit diagnostic source inspection still requires redaction. No public wire mapping is added.

## Canonical operation fingerprint and receipts

fingerprint() validates first, then SHA-256 hashes this exact concatenation, with no formatting:
`b"sagat.capacity.finish.scoped-leaf.v1\0"`;
raw16-byte UUIDs in order tenant, namespace, group, member, originalRun;
owner ASCII byte length as unsigned BE16, then those exact owner bytes;
iteration index as unsigned BE32; group revision, group epoch, run revision as unsigned BE64 each.
Owner length is1..64; no separators, JSON, display UUIDs, normalization or platform-width integers.
The domain differs from ChildRoot V1 and fixes the source profile and depth1 interpretation.
Exclude operation_key, renewable scheduler credentials and trace metadata from the fingerprint.
Preserve group-wide receipt keys `(tenant,G,operationKey)`, `(tenant,G,resultingRevision)` and
`(tenant,G,member)` across kinds; never add kind, namespace or path to those uniqueness scopes.
The stored scoped receipt retains the exact original binding, accepted run revision, expected
G revision/epoch and immutable original result; it contains no invented chain/head/revision.

## Required owning-transaction order for a future adapter

1. Validate structurally, begin safely, and discover only the exact qualified member binding.
   Missing/unobservable tenant/namespace resources yield Missing; an observable wrong kind/path
   yields BindingMismatch. Discovery grants no authority and must not redirect the original run.
2. Retain applicable own chain -> exact qualified original Run -> selected G NOWAIT. Reread exact
   binding under G; retain the original-run scheduler guard and sample its actual postwait clock.
   G contention is Busy and consumes/rolls back the whole transaction. Never require current head.
   Current scheduler authority is mandatory on first application and replay, even for terminal Run.
   Before source or receipt lookup, require locked originalRun.parent_run_id IS NULL and no
   exact tenant/namespace/originalRun membership in chain_segments; otherwise UnsupportedRunContext.
   This applies to first application and replay after current authority, without a late chain lock.
   See scoped-leaf-finish-flat-context.md; this initial restriction does not prove child/chain closure.
3. Read the exact canonical persisted certificate by tenant/namespace/originalRun/owner/index in
   this same transaction. Decode the closed ORDINARY_LEAF_V1 profile and bounded metadata; reuse
   LeafCollectionSuccessRecord::validate_structure, require its actual immutable definition pin
   equals the locked Run pin and certified_run_revision<=locked run_revision. Strict decoding must
   reject malformed identities, digests, sequences and timestamps rather than panic or truncate.
   Absent certificate is SourceEvidenceMissing; wrong profile is UnsupportedStoredProfile;
   malformed/disagreeing certificate is ContradictorySource. No caller record, separate certifier
   transaction, pure classifier result, terminal status, checkpoint or absence-of-work substitutes.
   Do not recertify, load history/payloads/siblings, follow successors or infer topology closure.
4. Look up the operation receipt by the group-wide key. Different fingerprint yields
   FingerprintConflict, including cross-kind reuse. Matching fingerprint still requires exact
   SCOPED_PATH discriminator/binding/expected counters and structurally valid stored result;
   disagreement is ContradictoryProjection. Return the original result with replayed=true before
   current G/run CAS or active-member checks; later header changes do not rewrite that result.
   Preserve the existing receipt counters/result_possession CHECK semantics exactly, including
   revision=expected+1, epoch delta0..1 and HELD/RETIRED restrictions; decode result time safely.
5. On receipt miss compare group revision, group epoch, then original run revision, returning the
   corresponding stale error. Require ACTIVE membership; then validate selected/header/optional
   exact holder projection with existing CapacityGroup::restore/finish. Counter overflow refuses.
6. Future first application changes only selected member/header contribution and atomically appends
   receipt, truthful scoped audit and reference hint. Preserve another holder and descendants;
   do not change source/run journal/revision or claim execution eligibility. Retain guards to commit.

This freezes authority -> supported original-Run context -> validated canonical source -> receipt -> creation CAS/member ordering.
No receipt replay bypasses missing/corrupt retained source. Native query/lock/error-order tests
must prove this contract separately; the application API and schema shape cannot establish it.

## Recovery, dependencies and evidence still required

Success is returned only after confirmed COMMIT. Confirmed abort is Storage; unresolved COMMIT
is CommitOutcomeUnknown. Cancellation may return nothing and is never evidence of rollback.
Retain the complete original logical command durably before any production invocation, including
operation key, identity and all expectations; renew only scheduler authority on reconciliation.
Fresh same-command entry waits for the original owner transaction, then replays before stale CAS.
Absent receipt does not permit changing the old key's fingerprint; stale expectations require a
separate later discovery/new operation. Durable intent or deterministic reconstruction is pending.

Application evidence: RED-first compile/use checks reuse the exact existing identity; owner/path/
counter bounds; independent SHA-256 vector and every identity/expectation field perturbation;
owner-length ambiguity controls; operation-key/authority exclusion; ChildRoot domain separation;
explicit original_run_id result and safe error formatting. No mock proves transaction authority.
Native evidence remains separate: exact persisted certificate/pin corruption and absence; replay
precedence and group-wide cross-kind conflicts; current/postwait fence; bounded reads; A1 holder/
retirement results; rollback/audit/hint atomicity; real both-order barriers, unknown COMMIT and restart.
Keep schema/down/up/privilege evidence, source preservation and retention, truthful observation/
audit/hint contract, C0/old writers, B1b.2c, release permissions and production intent as explicit
pending dependencies. Chain locking is ordering only; chain-source acceptance remains unproved.
No task T014-014..017/040 or whole B1/B2 closes from this interface decision. No tests were run here.

One fresh independent Astra/xhigh read-only review found no actionable interface, ordering or
source-contract defect. No tests, native adapter, permission or production caller follow from
this documentary decision. External review was declined by the user; no external CLI invoked.
