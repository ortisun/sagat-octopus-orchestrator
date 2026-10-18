# ADR-0038: Production incidents follow execution lock entry

Status: **Accepted scoped correction** | Date: 2026-10-18 | Features:009/014.
Refines only ADR-0022's production serialization clause under ADR-0030§18.
Refs:T014-040/044, FR-009-004/005/008, FR-014-014.

## Context

ADR-0022 prescribes binding EXCLUSIVE→run UPDATE→step UPDATE→audit dedupe. Existing ordinary
worker reports and heartbeats open their own-run transaction first, then acquire binding SHARED
through ensure_bound/bound_patterns. An incident holding binding while waiting for that run and
a report holding the run while waiting for binding form a real wait cycle. A deterministic native
storage-prefix test plus the public incident use PostgreSQL's own deadlocks counter: unchanged
production changed0→1, returned Unavailable for the incident, and failed the test. Unavailable
alone is not the evidence of deadlock. A separate public published/built/leased continuation
script test showed attribution could bypass its held chain entirely.

## Decision

Production incident attribution acquires, in one transaction:

1. applicable tenant/namespace/run membership chain FOR UPDATE, using the existing execution helper;
2. exact own-run FOR UPDATE, then requested namespace validation;
3. the existing authenticated subject/namespace binding EXCLUSIVE advisory guard;
4. exact own-step FOR UPDATE, then unchanged fence, post-lock database expiry, script/artifact,
   worker-session and binding validation;
5. existing bounded audit identity deduplication guard and audit insert, before commit/ACK.

The operation never forwards a run address or gains head/finality authority. Initial membership
commits with public new-run creation; later identity is immutable. This is not a live-reparenting
or delayed-initialization protocol. Binding registration/replacement acquire no run or chain;
replacement's session/step writes follow binding, while attribution's own-step guard follows
binding. The source inventory includes implicit locks separately; this scoped decision does not
accept the complete B0 graph or future group order.

The incident's binding mode stays EXCLUSIVE. Namespace/stale-target refusal, audit-only semantics,
closed redacted categories, per-attempt deduplication, test-run attribution and commit-before-204
remain as specified in ADR-0022. No execution transition, task scheduling, outbox, migration,
public contract field or capability is added. No code runs inline in the control plane.

## Verification and limits

Two tests fail unchanged production (0/2,3.09s) and pass the correction (2/2,2.12s).
The controlled cycle uses a real native RunTx.bound_patterns acquisition, the exact storage edge
used by report/heartbeat, concurrently with public record_incident. Public completion and journal
verification follow it as separate positive controls; this is not a claim of controlled concurrent
public completion. The public script-chain barrier observes the actual app-role wait and proves
own-run NOWAIT remains free before release, then verifies audit/completion/journal consistency.
World's runtime option selects the actual runner/store pool, verified at the waiter.

Existing revoke/stale/expired/duplicate/tenant/rollback cases remain required regressions.
script_incidents_total with closed category labels and the destination-free committed log retain
their existing after-commit behavior; the private port tests do not assert telemetry counters.
Exact source/commands and broader regression limits live in
`tasks/project-completion/evidence/incident-lock-order-20261018/`. Nested eligibility, group
persistence, worker-session post-wait freshness and integrated release gates remain separate work.

## Revisit when

Revisit if binding/catalog writers acquire runs/chains, if live membership identity becomes
mutable, if attribution starts changing execution state, or if any ADR-0022 durability,
authorization, redaction or category boundary changes.
