# Scoped collection timers — staged 0.3 increment

Status: implementation preparation. Public timer bodies stay closed until native and SDK
runtime evidence passes. Refs: T014-014–017/040, FR-014-004/005/014/015/016/017.

## First boundary: recorded lifecycle

Share TimerStarted/TimerFired step lifecycle across full and exact-path replay, preserving
the qualified identity. Starting requires Pending; firing requires WaitingOn::Timer. A
cancelled or already fired timer rejects another fire without mutation. Pending timers
leave the ordinary scoped planner idle because that planner has no clock argument. A
dedicated clocked scheduler command will start them; public parsing is not widened here.

## Required runtime boundaries, not yet implemented

- Clocked start and due fire commands validate the exact pinned Timer and complete path,
  current state/fences and explicit database time. Reuse duration overflow and existing
  root timer semantics; do not consult a wall clock inside the domain.
- Full and bounded writers persist waiting_kind/due_at atomically with journal, routing,
  head, quota, audit/telemetry and any outgoing obligations. Native reads/writes reject
  malformed timer lifecycle and orphan worker/event/child/approval/control metadata.
- Activity→Timer and scheduler-owned Timer entry consume their actual instances exactly
  once. No worker lease or task.ready is fabricated. Return-only entry remains separate.
- Existing timer discovery currently filters root paths and drops path identity. Extend
  it deliberately, retaining root behavior and selecting/deduplicating qualified instances;
  fire scoped timers through the existing bounded fenced apply path.
- Preserve pause/resume, cancellation/drain, policy reduction, cold recovery and due-time
  fences. A completion recorded while paused must remain durably discoverable/routable on
  resume; do not leave a succeeded unrouted timer stranded behind an active slot.
- Prove full/frontier parity, timer transaction rollback, no-worker waiting, duplicate and
  stale candidates, actual held-lock cancellation/fire order and source-profile exclusion.
  Use explicit clock inputs/aged test deadlines or barriers, never sleeping for races.
- Extend the common five-SDK case only after public runtime proof, inspect references and
  rerun without blessing. Keep the released-profile/rollout and complete release gates open.

The remaining event waits, child workflows, scoped approvals, broader loops and nested
capacity have separate transaction and source-finality prerequisites. This note closes none.
