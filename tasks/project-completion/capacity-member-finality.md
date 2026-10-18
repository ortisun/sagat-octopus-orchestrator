# Capacity member finality — B1b.1 candidate

Status: reviewed scoped decision; dormant exact ChildRoot Finish port implemented, broader authority pending. One existing child-chain root member Finish only. Refs:T014-014..017/040,
ADR-0030, A0/A1, B0 and reviewed B1a. No scoped-path settlement or nested runtime authorization.

## Artifact

An immutable registry binding associates (tenant, namespace, group, member) with ChildRoot(chainId);
its current owner is that chain's current head with empty InstancePath, while a structured path
retains its own runId/nonempty InstancePath and does not become a successor's path. Read-only
discovery selects a candidate, but mutation retains applicable own-chain → exact own-run → G NOWAIT
guards and rechecks that exact binding, scope, current owner and observed revisions under G. Finish
requires current persisted facts showing this own run is the chain head, its own segment is FINAL
without a successor, run finished_at and segment completed_at are present, and its terminal run
outcome agrees with the chain's non-null final outcome; terminal
status alone, CONTINUED, historical segments and caller-supplied settlement flags fail closed.
These facts authorize only A1 Finish of the selected active member, preserving every other
membership and another holder's possession, with eventual member/header/receipt/audit/outbox
mutation atomic under B1a. This decides no quiescence, Acquire/Vacate eligibility, creation,
handoff integration, migration, fairness, root-wide cancellation sweep or nested-runtime authority.

## Preconditions

- Commands carry expected identities/revisions, never settled=true or sourceTerminal=true.
  Copied snapshots, possession, active counts, parent/root identity KS and inactive ancestor flags
  cannot establish settlement. Authority is derived in the owning transaction.
- Discovery/head/owner mismatch rejects the attempt, never forwards to a successor. Read-only
  discovery is not authority. Recheck exact member binding after G, before A1 state mutation.
- Logical ChildRoot identity spans continuation; its owner follows authoritative current head.
  If storage materializes owner-run, it must equal that locked head; maintenance in handoff remains
  a separate contract. Parent child-step path and child's empty root path are distinct identities.
- Use own chain, own run/segment and exact member binding only. No foreign business state or
  mutation is needed. Parent notification/link settlement remains separate; finishing B never
  settles A/C because a parent is terminal or detached.
- SQL schema/RLS/binding uniqueness/non-reuse/implicit locks, validated bounded restoration,
  durable same-key/fingerprint receipts and current scheduler authority must be specified before
  persistence. Existing applicable scheduler guard and postwait database-time check remain required,
  including on terminal runs. B1a contention consumes the TX; uncertain COMMIT requires reconciliation.
- The adapter reads exact scalar finality facts under retained guards. Existing full RunStore entry
  hydrates all own steps/scopes and is not evidence of bounded selected-member access.

## Existing source facts and limits

ChainRunTx::locked_chain / LockedChainHead expose current head, revision, scope and final outcome.
RunChainState exposes own segment completion kind and successor. RunState reduces CONTINUED to
Succeeded, so RunStatus::is_terminal alone would incorrectly finish a logical chain member.
PathExecutionState has no chain-head/finality proof; inactive ancestor is not settled membership.
Existing finalize_chain commits own run FINAL, segment FINAL and chain outcome together; existing
regressions do not prove capacity settlement or the future registry invariant.

## Required storage evidence

1. Current FINAL finishes once; CONTINUED and historical segments are inert before/after successor
   FINAL. Missing/contradictory finality facts fail closed.
2. Wrong tenant/namespace/group/member/chain/run/nonempty path cannot settle. Stale discovery,
   owner or member association cannot forward or mutate.
3. Actual handoff/discovery and duplicate Finish barriers; fresh retry rechecks current facts.
   Wrong/expired scheduler authority produces no effects.
4. Confirmed abort of a first attempt leaves zero new member/header/receipt/audit/outbox effects.
   A successful fresh retry produces one atomic Finish; duplicate replay keeps that result, while
   a conflicting fingerprint adds no Finish effects. Postdispatch uncertain outcomes reconcile to
   either committed or aborted state before retry; receipt rules still require a schema.
5. A→B→C with B FINAL preserves A/C. B-held creates vacancy; C-held preserves C's epoch. Parent
   terminality alone never settles C. CONTINUED does not settle B's logical membership.
6. Assert bounded selected-row access, no foreign-run business access and no group sweep. Root
   cancellation and later bounded per-member reconciliation remain pending.

Later decisions: selected-member schema/receipts and one real fenced SQL Finish; structured path
settlement, creation/join and handoff; current forward/drain eligibility and reduction; discovery,
fairness, cancellation/cleanup, rollout and resource gates. No production activation follows.

Two fresh bounded Astra/xhigh source review cycles completed. First corrected pre-COMMIT abort
evidence to zero effects; second found no remaining concrete identity/finality/locking/recovery
or source-claim issues in this unit. No tests or SQL/capacity runtime acceptance is inferred.
