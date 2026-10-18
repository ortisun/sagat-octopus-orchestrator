# Initial scoped Finish: flat original-Run context gate

Read-only bounded decision at `4f3b6ac0ead1162151e9c909d0de63331f63925f`.
Scope: future dormant fixed OrdinaryLeafV1 native consumer; no implementation or tests executed.

## Decision

Add an explicit consumer-only context refusal before canonical source and receipt lookup.
The reviewed contracts defer chain-source acceptance but contain no precise adequate gate.
The fixed definition excludes continuation policy and child/nested controllers; it does not encode
whether this Run is itself a child or a stored chain segment. The certifier currently checks neither
parent absence nor chain absence. This does not invalidate an existing certificate or extend its proof:
the consumer is narrower than the certifier's supported metadata, with preservation unproved outside
its current flat evidence. Keep certification, canonical-reader decoding and certificate profile unchanged.

Source correction: current production child creation sets ParentLink even for an ordinary definition;
PgRunTx::commit initializes a chain only when continuation_policy() is Some. No unconditional implicit
ordinary-child chain creation was found at this revision. The gate must nevertheless inspect actual
membership, not infer its absence from source definition or this creation path.

## Exact predicate

After the existing guards, require both actual persisted facts for the command's original identity:

```sql
locked_original_run.parent_run_id IS NULL
AND NOT EXISTS (
  SELECT 1 FROM chain_segments
  WHERE tenant_id = $tenant AND namespace_id = $namespace AND run_id = $original_run
)
```

Read parent_run_id (or its IS NULL boolean) from the exact qualified Run FOR UPDATE row. Use a fresh
same-transaction, scalar EXISTS for exact qualified membership after the guards; the existing
UNIQUE(tenant_id,run_id) bounds this probe. It reads no chain head, outcome, revision or foreign state.
A SCOPED_PATH member's NULL chain_id is its binding discriminator, not evidence of Run nonmembership.
Any parent or any chain membership refuses, including legacy children without chains, root-chain
members, historical segments, FINAL chains and settled/detached children. No terminal-state exception.
Do not infer flatness from depth, root_run_id, parent_settled, the caller, a certificate or receipt.
Existing root CHECKs imply root identity/depth from parent absence; no redundant ancestry scan is needed.
“Flat” here restricts original execution-Run context, not capacity-group topology or retained descendants.

## Required order and stability

1. Keep structural validation, safe begin and exact qualified member discovery and its existing errors.
2. Keep applicable own-chain UPDATE -> exact qualified original Run UPDATE -> selected G NOWAIT;
   reread exact binding under G, then retain current original-run scheduler guard/postwait clock.
   G contention remains Busy; missing resources/binding mismatch/authority errors retain precedence.
3. Evaluate the predicate above. On failure return UnsupportedRunContext and roll back the whole TX.
4. On success retain canonical metadata -> operation receipt -> creation CAS/active-member ordering.
   Context and current authority apply to first application and replay; a matching receipt cannot bypass
   either. In unsupported context, do not inspect canonical source or operation receipt at all.
5. Preserve all guards until rollback/commit. Add no parent/root/child/link/group-registry locks or scans.
   Reuse lock_member_chain_sql before Run. It currently returns (), so its success is not absence proof;
   the exact post-guard EXISTS supplies that fact without acquiring a late chain lock. If that probe
   observes membership missed by earlier discovery, refuse/rollback; never lock that chain after Run.

Current membership is created with its new Run and is identity-immutable afterward; parent identity
is a current-writer invariant. Together with retained own-Run/chain guards this bounds the decision
under current writers. No gap/advisory lock or global topology proof is introduced. Reparenting,
late initialization/backfill or another membership writer requires revisiting these preconditions;
foreign keys alone do not prove them. C0/old-writer compatibility remains separate. Arbitrary privileged
row surgery is outside this contract; absence here is not a database-owner corruption guarantee.

## Additive dependency before native implementation

Add unit variant `ScopedLeafFinishError::UnsupportedRunContext` to the reviewed application contract
and its eventual closed Rust enum, safe Display/Debug and exhaustive refusal handling before the adapter.
Meaning: an observable original Run has a parent or chain membership outside this consumer's accepted
context. Use fixed safe text and no raw owner, parent, chain, command or driver diagnostics.
Do not overload UnsupportedPath (depth/owner/index), UnsupportedStoredProfile (certificate profile),
SourceEvidenceMissing (absent certificate), Missing (unobservable resource), or ContradictorySource.
Storage/query errors retain sanitized Storage; do not convert lookup failure into an absent membership.
No command field, fingerprint domain, result field, certificate version or schema change is required.

## Focused future evidence and limits

- Actual flat native certificate: normal apply and matching replay continue through the existing gates.
- Parent-only ordinary child, root chain member and child-chain member: UnsupportedRunContext;
  include historical/FINAL and settled/detached controls so no status exception enters the predicate.
- Unsupported context with absent source and matching/different receipt fixtures: context refusal first;
  stale scheduler authority still refuses earlier. Label fixture certificates/receipts as fixtures.
- Exact tenant/namespace/originalRun selection and a neighboring chain member cannot redirect the probe.
- Held-lock barriers establish existing chain-before-Run entry, postwait authority and refusal rollback;
  assert no source/header/member/receipt/success-audit/hint mutation. No sleeps or chain-source proof claim.

Acceptance adds only this initial consumer restriction. It closes neither B1b.2c nor chain/child/nested
producer preservation, B1c/d/e, C0, production caller/INSERT activation or full scoped Finish authority.
The reader extraction remains independent; it must not inherit this consumer-only refusal.

## Exact source anchors

- `tasks/project-completion/leaf-current-producer-preservation.md:45` (chain/nested evidence excluded).
- `tasks/project-completion/scoped-leaf-finish-binding.md:45` (profile versus chain/topology limits).
- `tasks/project-completion/scoped-leaf-finish-application-contract.md:47,66,94,126,149` (enum/order/open gate).
- `tasks/project-completion/scoped-leaf-finish-schema.md:20,26,35` (binding shape/qualified Run FK only).
- `tasks/project-completion/canonical-leaf-reader-contract.md:23,63,71` (caller guards and reader limits).
- `crates/domain/src/workflow/collection_shape.rs:24,53,73,147` (definition-only frozen shape).
- `crates/infra-postgres/src/leaf_certification.rs:57,60,74,102,258` (chain/Run/canonical/proof split).
- `crates/scheduler/src/passes.rs:4581,4604,4620`; `crates/infra-postgres/src/execution.rs:990,4246` (creation).
- `crates/infra-postgres/src/execution.rs:427,442` (qualified chain lock, immutable-membership premise).
- `crates/infra-postgres/src/chain_progress.rs:899,931,947,958,1041,1736` (same-TX chain/link and ancestry).
- Migrations under `crates/infra-postgres/migrations/`: `20261009000100_execution.up.sql:40,60,64,72`;
  `20261102000000_execution_chains.up.sql:42,56,82,138` (Run ancestry and membership constraints).
- `docs/adr/ADR-0037-stable-parent-identity-guards-for-child-handoff.md:35,61` (writer ancestry invariant);
  `tasks/project-completion/capacity-lock-graph.md:17,30,34` (existing creation/lock boundary).

One fresh bounded Astra/xhigh read-only review found no issue. No tests executed for this
documentary decision; future native context evidence remains required. External review was
already declined by the user. No external CLI invoked.
