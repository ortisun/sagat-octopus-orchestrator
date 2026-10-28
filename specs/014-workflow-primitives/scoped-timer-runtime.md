# Scoped collection timers — staged 0.3 increment

Status: unreleased 0.3 implementation. The working public collection body accepts timers;
focused native and five-SDK evidence does not close complete release acceptance. Refs: T014-014–017/040, FR-014-004/005/014/015/016/017.

## First boundary: recorded lifecycle

Share TimerStarted/TimerFired step lifecycle across full and exact-path replay, preserving
the qualified identity. Starting requires Pending; firing requires WaitingOn::Timer. A
cancelled or already fired timer rejects another fire without mutation. Pending timers
leave the ordinary scoped planner idle because that planner has no clock argument. A
dedicated clocked scheduler command starts them in the following runtime increment.

## Runtime contract and acceptance boundaries

- Clocked start and due fire commands validate the exact pinned Timer and complete path,
  current state/fences and explicit database time. Reuse duration overflow and existing
  root timer semantics; do not consult a wall clock inside the domain.
- Full and bounded writers persist waiting_kind/due_at atomically with journal, routing,
  head, quota, audit/telemetry and any outgoing obligations. Native reads/writes reject
  malformed timer lifecycle and orphan worker/event/child/approval/control metadata.
- Activity→Timer and scheduler-owned Timer entry consume their actual instances exactly
  once. No worker lease or task.ready is fabricated. Return-only entry remains separate.
- Timer discovery retains root behavior while selecting/deduplicating qualified instances;
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

## Clocked command and resume contract

Timer start requires full forward eligibility, an exact pinned Timer, a pristine Pending
state and explicit time. Fire requires WaitingOn::Timer, now>=due_at, a nonterminal unstopped
Run and active/open ancestors. As at the root, an accepted timer may record firing while
the Run is paused; routing stays deferred. It must not release forward work during pause.

To resume that durable completion, nominate ADMITTED members with succeeded/unrouted steps
in collection discovery, then run the ordinary bounded/full local settlement under current
revision and shard authority. This settlement evaluates no expression and invents no event:
the existing planner emits its qualified StepRouted/StepReleased events, or remains idle.
Pending Timer/Condition/Switch/Return stays owned by each specialized scheduler action.

Start and fire share one prepared-event validation across both state models. Persisted timer
state allows no worker, retry, choice, event, child, approval, output or secret metadata;
Waiting owns exactly waiting_kind=TIMER plus due_at, and every settled state clears both.
Only pristine Pending, unrouted Waiting, Succeeded and routed Cancelled are legal; Timer has
no Failed command. Native UPDATEs compare original status, routed and deadline in addition
to all constant lifecycle/physical invariants, refusing out-of-band changes after preparation.
The same physical predicates guard reads and native UPDATEs, including metadata added after
a prepared snapshot without a revision change. Existing root timer behavior is preserved.

Required focused evidence includes fire while paused→cold resume→one route, wrong clock
and path/kind/fences with no mutation, duplicate fire, cancelled fire, duration overflow,
and deadline/full-path replay properties. Native barriers must cover both cancellation/fire
commit orders and expiry after waiting for authority. The pure domain pass alone is not release acceptance.
