# Narrow architecture re-read of the root claim's lock order (2026-11-27, commit `83d30cd75`)

Narrow re-read (OD-2026-11-25-2 step 4) by the architecture reviewer of the fix of Major 1 of
[design-fixes-5-rereview-architecture-reviewer.md](design-fixes-5-rereview-architecture-reviewer.md), and of the two ADR
edits that acceptance needed.

**Verdict.** The fix holds. The deadlock is closed in both orders, and the new order adds no other cycle. Nothing here is Blocker or Major. Two minors affect how T022-272 and T022-255 are built. I had no shell, so I could not run `git branch -a` or `git worktree list` to look for duplicate work.

### 1. Is the deadlock closed in every order?

Yes. With T022-272 the claim takes its locks in this order: run lock (`SKIP LOCKED`), drain key (shared), the claimer's binding key (shared), then the session row (`FOR UPDATE`).
- **Claim first.** The replacement waits on the key (`bindings.rs:88-92`) before it touches any session or step. The claim's lookup round then takes the same key again, which never waits. The replacement then closes the session and expires the leases the claim made (`:245-255`).
- **Replacement first.** The claim waits on the key while holding only the run, chain and drain locks. The replacement never takes those, and it locks no step the claim holds (`steps` is only read at `execution.rs:1016-1019`). The claim then finds the session gone and leases nothing (`execution.rs:1006-1007`).
- **Producer is another identity.** The claim locks none of the other identity's sessions or leased steps, so the replacement of that identity never waits on the claim.
- **The lake path is fixed too.** The old order also allowed a three-party cycle: two claims, the tenant's `database-concurrency` key (`database.rs:108`), and a replacement. The new order closes it, because a second claim now holds its key before it waits.

### 2. Any other cycle against ADR-0050's slots?

None. Three transactions take a binding key exclusively: the replacement (`bindings.rs:88`), registration (`workers.rs:239`) and the incident record (`incident.rs:128`).
- None of them takes the drain key, a quota lock, a capacity group, `database-concurrency` or a semaphore.
- The incident record takes its run before the key (`incident.rs:112-132`).
- Drain and undrain take only their own key, a row and an audit record (`operations.rs:967-987`).
- The claim's run and chain locks are `SKIP LOCKED`, and the capacity group is `SKIP LOCKED` (`worker_capacity.rs:132`).
- The order "key, then session row, then the namespace secrets lock and memberships" is already the order of `database_owner` (`database.rs:38-98`) and of the scoped claim.

### 3. Is `key-held` checkable as specified?

Mostly. The in-function form (database.rs rewritten to call `hold_worker_binding`) and the named-caller form both match the tree:
- the callers of `.worker_session_room(` are `worker.rs:1147` and `scoped_claims.rs:112`;
- `.bound_patterns(&session.subject)` comes before the call at `scoped_claims.rs:102`.

There are two holes (minor 2 below).

### 4. Do the two required ADR edits read correctly?

Yes.
- **ADR-0135** lines 64-71 list the four argument kinds and the claim's key-before-row order.
- **ADR-0136** lines 130-134 and 245-250 strike the Signal-with-Start wording, call such a start a recovery start, and say who publishes the event.

### Minor findings that affect buildability

**Minor 1. T022-272's evidence cannot be seen failing (Rule 18).**
- **Where:** tasks.md:1314.
- **Problem:** Until T022-210 adds the lookup round, the old order does not deadlock either. "Never `40P01`" and "finds its session closed" are true with or without the fix, and D022-333 says so itself. Also, the harness `Slot` enum (`tests/support/lock_order.rs:41-84`) has no slot for the binding key or the session row, and `against_at` refuses a reference that is not in slot order.
- **Fix:** Make the case `against_at` with the reference [binding key exclusive, then that session row `FOR UPDATE`, then `UPDATE steps`], and assign those locks a slot (or add variants), the same choice T022-254 needs. Today's claim never waits for the key, so the harness fails with "the path ended without waiting" (`lock_order.rs:334-338`). That is red now and green after the fix.

**Minor 2. The `key-held` check can pass while the rule is broken.**
- **Where:** D022-334 (research.md:3811-3818).
- **Problem:** The statement lives in the free function `worker_session_room` (`execution.rs:985`). A new adapter caller of that function would be checked by nothing, because only the application callers of the trait method are constrained. The check also trusts, without looking, that `PgRunTx::hold_worker_binding`, `PgRunTx::bound_patterns` and `PgPathExecutionTx::bound_patterns` (`path.rs:119-126`) actually take the key.
- **Fix:** Add both to T022-255:
  - the free function may be called only from `PgRunTx::worker_session_room` and `PgPathExecutionTx::worker_session_room`;
  - each of those three methods must contain a `crate::bindings::hold_worker_binding(` call.
  - Add a negative fixture for each.

**Leftover text (not a condition).** ADR-0056:84 still reads "claim: run → drain fence → worker session → …", which contradicts 2(a) at :133-134. `docs/architecture/execution-semantics.md:259-262` says the same. Insert "→ the claimer's binding key (shared)" after the drain fence in both. The ADR line can go into the acceptance change; the document update belongs in T022-272's pull request (Rule 10).

### Acceptance and build

- **Accept now:** yes. The 2026-11-26 and 2026-11-27 amendments to ADR-0050, ADR-0056, ADR-0135, ADR-0136 and ADR-0137 can be accepted now. Nothing in this fix touches ADR-0050's slot table or ADR-0137. I recommend putting the ADR-0056:84 line in the same change.
- **Build:** nothing blocks starting slice by slice.
  - T022-272 can start now (any time after slice 1), once its evidence is reworded as in minor 1.
  - Slice 7 (T022-210, T022-255, T022-271) opens once T022-272 has merged, with minor 2 folded into T022-255.
  - Slices 10 to 13 (T022-239) are no longer held by the Signal-with-Start finding.

Main files:
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/adr/ADR-0056-concurrency-keys-are-durable-permits-granted-by-the-waiters-shard-owner.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/architecture/execution-semantics.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/tests/support/lock_order.rs
