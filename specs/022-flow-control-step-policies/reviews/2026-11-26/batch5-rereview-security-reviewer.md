# Security re-read of the batch-5 fixes (2026-11-26)

Short re-read of the fixes alone (OD-2026-11-25-2 step 4), commits `ac107fcb..223ab80d` of `feature/022-throughput-control`,
against `reviews/2026-11-25/batch5-reviews-and-rulings.md`. Read by the security reviewer on the tree after the 2026-11-26
renumbering (D022-090 to D022-092 are now D022-121 to D022-123).

Verdict: six of the nine security rulings fixed as claimed, one partly, two not fixed; suspicion 3 partly fixed. Under
OD-2026-11-24-4 a security finding cannot be settled in writing, so the second nit, suspicion 3 and suspicion 4 still block
approval.

## Rulings

| Ruling | Verdict | Evidence |
|---|---|---|
| Security minor 1: a held signal is delivered without a fresh permission check, said in words | fixed | `spec.md` FR-022-060 amendment (about lines 761-767); test `run_creators::a_held_signal_is_delivered_without_reauthorization_and_keeps_its_sender` (`run_creators.rs:2712`). |
| Security minor 2: a dropped signal's audit names its sender | fixed (new finding 3) | `held_signals.rs:121-125`; only `DELIVERY_FAILED` is tested for the sender (`run_creators.rs:2622`). |
| Security minor 3: two doc comments | fixed | `routes/execution.rs:794`, `runs.rs:1034`. |
| Suspicion 1: a held signal is dropped however the run ends, if its start never ran | fixed | `infra-postgres/src/execution.rs:5597-5613`; `verify_run` (`execution.rs:783-795`); test `verify_run_refuses_a_held_signal_kept_for_a_run_whose_start_does_not_wait` (`run_creators.rs:2681`). |
| Suspicion 2: a delivery that keeps failing is dropped, with its audit | fixed | `passes.rs:594`, `:6284` (`DELIVERY_FAILED`), `:6166`; test `a_held_signal_whose_delivery_fails_is_dropped_and_the_start_is_granted`. |
| Suspicion 3: a replayed cursor, and what a page boundary reveals | partly | Replay fixed: fifteen-minute expiry (`routes/concurrency.rs:76-81`, `flow_control_keys.rs:457,472,568`), tenant and namespace binding unchanged; tests `flow_control_keys::tests::a_cursor_expires_fifteen_minutes_after_its_page`, `rest::an_expired_cursor_is_refused_with_its_own_message`. The page-boundary half is not closed (new finding 1). |
| Suspicion 4: a signal waited for only inside a body is held, then always dropped | not fixed | Depends on correctness minor 1, not built. `runs.rs:2116-2126` holds the signal when any node waits for the name (`def.nodes().any(...)`), not only a root step of the entry path, and does not check the output bound. A `waitEvent "x"` inside a loop body with a queued Signal-with-Start answers `held: true`, charges the payload to `tenant_content_bytes`, and the grant always drops it `NOT_WAITING_AT_GRANT`; with room on the key the same request is refused `SIGNAL_NOT_ACCEPTED_AT_START`. |
| Security nit: a counter for held, delivered and dropped signals | partly | The counter was not built (`dropped_not_waiting`, `dropped_output_bound` appear only in the rulings); the outcome is reported only through `scheduler_actions_total{kind="gate_grant",outcome}`. |
| Security nit: the alert detection run as `sagat_app` | not fixed | `tests/integration/flow-control-step-policies/alerts.rs` has no runtime-role switch, and CI's shard step (`.github/workflows/ci.yml:210-218`) does not set `SAGAT_TEST_AS_RUNTIME_ROLE`. |

Also checked, no finding: grants (`flow_control_held_signals` `SELECT, INSERT, DELETE`; `flow_control_counts` `SELECT` only;
`20270215*` change CHECK constraints only); `flow_control_counts_track()` (`search_path = pg_catalog, pg_temp`, qualified
names, `EXECUTE` revoked from `PUBLIC`); the held signal's name charged by ALWAYS triggers
(`20270214000000_flow_control_held_signals.up.sql:35-40`); no raw key or label in logs, spans, metrics or `spansPages`.

## New findings

1. **minor: the page boundary of the group view tells a caller without the reveal right how many keys a group has.**
   `crates/infra-postgres/src/flow_control.rs:1357-1424`: the 200-row budget is shared by every group on a page, and a group is
   emitted only if it finishes inside what is left. A developer (`definition:publish`, `run:start`, `ops:read`, no reveal
   grant, so `view: GROUPS`) publishes a group sorting just before the target G, starts m runs on m keys, and reads
   `?limit=2`: G appears on page 1 with `spansPages: false` exactly when G has at most 200 − m keys; a binary search over m
   (about 8 requests) gives G's key count, and reading again as one run of G ends tells which runs share a key, the linkage
   D022-061 and D022-086 forbid. Fix: each group its own 200-row budget, the page ending after the first group that does not
   finish; cap `limit` for the group view; record the remaining 200-step leak in D022-100 as accepted.
2. **nit: a cursor stamped in the future never expires** (`flow_control_keys.rs:472`): refuse one stamped more than about a
   minute ahead.
3. **nit: the sender in the drop record is cut to 128 of up to 255 characters** (`held_signals.rs:124`): keep all 255, which the
   `actor` CHECK bounds.
