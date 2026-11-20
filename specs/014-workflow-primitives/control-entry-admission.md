# Control entry admission — bounded 0.3 increment

Status: implementation under verification on the unreleased 0.3 branch; this document does
not advertise a released capability. Refs: T014-014–017/040, FR-014-004/005/014/015/016. This extends collection
entry only; loop broadening, nested capacity, waits and children remain separate.

## Command and ownership

Keep one `CollectionAdmission` command with a closed dispatch intent:
`Activity { activity, outbox_id }` or `Scheduler`. Existing activity constructors retain
their signatures and produce the former. A scheduler constructor carries no activity or
task-notification identity. Both share exact owner/path/index, immutable item digest/content
and the complete body reservation count. The native writer checks the pinned definition:
Activity must name its exact declared activity; Scheduler admits only Condition or Switch.
Return-only entry and every other node kind remain refused in this increment.

Add a general member-admission port. Its compatibility default delegates Activity to the
existing activity port and refuses Scheduler. PostgreSQL implements the general port; its
legacy activity entry point continues to reject Scheduler intent. The run-store facade
delegates both explicitly. No sentinel activity, fake task or empty model mapper is introduced.

## Transaction and discovery

Reuse the current admission transaction and lock order: own chain/run, shard authority,
tenant policy/usage, run usage and owning scope. Preserve revision CAS, exact item membership,
tenant slots, lifetime charge `1 + reserved`, replay/mismatch checks, audit and rollback.

Activity keeps its existing READY row, admission/release journal events and task.ready outbox.
Scheduler creates one pristine PENDING control row with its actual node_kind and empty physical
worker/wait/child/output metadata. It appends only collection.iteration_admitted, advances
Run.version once, and marks telemetry pending. The existing durable candidate query explicitly
discovers ADMITTED member paths with PENDING steps; the next ordinary scheduler pass resolves
the control. There is no outgoing worker notification obligation and no outbox record/identity
for this admission. Prove progress with a fresh cache/scheduler and no worker lease or hint.

The current public iteration-concurrency quota remains positive (1..1000000000). Zero is
rejected with 422 by the existing quota contract; the separate nested-capacity authority/expiry
zero-room gate is not a change to that policy.

An admitted control consumes its one logical instance; Return remains a reservation consumed
only at checkpoint. Unselected nodes retire without lifetime refund. A pure control/Return
body is allowed, with no worker ownership. Accepted decisions/checkpoints continue through a
policy reduction; new iterations still require current tenant admission capacity.

## Additive stored profile

Use a new immutable migration, allocated after checking other branches/worktrees. Existing
collection_iterations rows backfill a closed `entry_dispatch='ACTIVITY'`. ACTIVITY requires
non-null activity and outbox_id. SCHEDULER requires both NULL. Make the two old columns nullable
only with that CHECK; keep uniqueness for non-null activity notifications. Classify the new
bounded control-plane column in storage-classification.json and preserve runtime privileges.

Downgrade takes the appropriate table lock and refuses while any SCHEDULER membership is
retained, including terminal evidence. A scheduler admission records `entry_dispatch=SCHEDULER` in the
append-only journal too; downgrade checks that independent evidence even if a CHECK-valid
projection has been relabeled ACTIVITY. Full, exact-path and frontier readers validate dispatch
against the pinned entry kind and journal marker. Otherwise restore both NOT NULL constraints and remove
the additive dispatch column. Never rewrite applied migration history. Frozen OrdinaryLeafV1
certification still requires its activity-chain source; nullable activity must cause a typed
contradictory/unsupported-source error, never a panic or broader certificate acceptance.

Same-delivery updates must cover spec.md, plan/task traceability, contracts, quickstart and
actual validation evidence before the increment is accepted. The public parser may then accept a Condition/Switch entry with up to eight executable nodes
and one Return. Existing wire fields and worker1.5 stay unchanged. Released-profile upgrade, active-run and compatibility evidence is required before release
or advertising the capability, not inferred from focused tests. A 0.2 scheduler cannot execute
these new shapes. Deployment must fence/drain old scheduler authority before admitting new
0.3 definitions; mixed old/new scheduler execution of new definitions is unsupported. The
release gate must demonstrate that rollout boundary and continued execution of pinned 0.2 runs.

## Required evidence before activation

- Public publication RED, then a control-only collection finishes with zero worker leases,
  zero task.ready records and exact lifetime accounting.
- A switch entry selects one actual activity for its positive item and skips it for its
  negative item; correct ordered results, metadata and qualified replay/projection.
- Native Scheduler replay is idempotent and mismatches refuse; stale version/authority,
  denied quota, tenant mismatch and injected journal failure leave all projections unchanged.
- Pending control cancellation drains; accepted control progress survives capacity reduction;
  cold durable discovery does not depend on a broker hint or fabricated task.
- New schema backfill/checks/runtime grants, retained-control downgrade refusal and no-control
  rollback; common five-SDK authoring/runtime evidence and all affected regression gates.
- `public_control_entries_remain_outside_the_frozen_ordinary_leaf_profile`: both control
  entry kinds fail pure-domain OrdinaryLeafV1 shape checking and native certification with
  UnsupportedShape under owner/runtime roles. Rerun the existing successful activity-chain
  certification after nullability; public breadth must not broaden the frozen profile.

This document records a bounded contract, not completed runtime or release evidence.
