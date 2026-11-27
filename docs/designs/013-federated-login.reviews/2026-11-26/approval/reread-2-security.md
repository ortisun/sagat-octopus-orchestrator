# Targeted security re-read of design 013 for approval, round 2 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: fd2aad8c
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the fixes of round 1 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer judged round 1's findings 1, 3 to 9 closed and finding 2 closed for a lowering and a re-enable.
Counts: Blocker 0, Major 1, Nit 6

1. **Major. Requirement 013-010, Technical Plans 3, 4 and 6, Task 013-20.** One floor per connection, replaced by every raise, and "raise" undefined for a set of accepted values: requirement {a, b}, session with `a`; raise to {b} (refused), lower to {a, b} (still refused), change to {a} (a raise: the new floor {a} accepts the session again), and a loop selecting by the current floor never marks it. Fix: a raise is any change that removes a value; the loop selects by the floor and clears it after marking; while a floor is set another raise is refused (retryable); a test.
2. **Nit. Requirement 013-019, Technical Plan 13 (suspicion).** A cut-off time taken at transaction start can precede the time of a session committed by a racing redemption that held the row `FOR SHARE`; a re-enable before the marking would revive it. Fix: times after the lock, or better an epoch on the connection copied onto the session under the lock; extend the barrier test with a re-enable.
3. **Nit. Requirement 013-019, Technical Plan 4 "Renew", Technical Plan 13, Architecture decision 0140 decision 6.** Renewal and refresh re-check only the state, the current context and the membership, so a cut-off session renews and rotates after a lowering or a re-enable; the stream and upload re-checks are unnamed. Fix: all use the per-request check's predicate; a test.
4. **Nit. Technical Plan 4 step 3.** The poll shows the verified identity to the attempt's subject from any browser, so a start address forwarded to a colleague discloses the colleague's provider identity. Fix: require the binding cookie.
5. **Nit. Technical Plan 9, Task 013-3, Architecture decision 0140 decision 8.** The `claims` override lets anyone reaching the provider mint a token for any subject, with no size bound. Fix: only the names the table breaks, `sub` only empty, 2 KiB, behind a test-only setting.
6. **Nit. Requirement 013-007, Tasks 013-9, 013-20.** The table misses the named wiring case (a pending revision's client identifier as `aud` and the reverse) and puts the milestone-D `acr` row in a milestone-A task. Fix: two rows; move `acr`.
7. **Nit. Technical Plans 3 and 6, Constitution Check VII.** The 10-minute rule needs a suspension time nothing stores; row VII omits the exception. Fix: `memberships.status_changed_at`; mention it in row VII.
