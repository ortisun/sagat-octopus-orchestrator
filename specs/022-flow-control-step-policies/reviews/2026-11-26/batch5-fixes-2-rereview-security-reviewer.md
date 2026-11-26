# Security re-read of the batch-5 fix-up (2026-11-26, commit `c36e8336`)

Short re-read of the security fixes alone, by the security reviewer, against `batch5-rereview-security-reviewer.md`.

| Item | Verdict | Evidence |
|---|---|---|
| Suspicion 3 (cursor replay and page boundary) | partly fixed | Replay closed (expiry, lead refusal, tenant and namespace binding); the page boundary only partly (new finding 1). |
| New finding 1 (group view page boundary) | partly fixed | The leak across groups is closed: each group its own budget (`crates/infra-postgres/src/flow_control.rs:1359`, `:1411`, `:1427-1434`), `GROUP_PAGE_GROUPS = 5` (`crates/application/src/operations.rs:746-749`, `:824-827`); tests `rest::a_group_view_breaks_a_group_where_its_own_filter_does`, `rest::a_page_of_groups_holds_at_most_five_groups`. Paging inside one group is still a leak (below). |
| Suspicion 4 | fixed | `crates/application/src/runs.rs:2111-2140` (`entry_path().waits_for`, output bound); `WorkflowDefinition::entry_path` (`crates/domain/src/workflow.rs:2120-2182`) never enters a body (`workflow.rs:1079-1110`); tests `run_creators::a_signal_with_start_whose_wait_is_not_on_the_entry_path_is_refused_when_the_start_would_queue` (`:1644`), the output-bound twin (`:1693`), `the_entry_path_holds_what_the_release_of_a_start_reaches`. |
| Security nit: held-signal counter | fixed | `crates/telemetry/src/flow_control.rs` (six closed outcomes, one label), recorded only in `PgRunTx::commit` (`execution.rs:6262`); tests `flow_control_vocabulary::*`, `run_creators::the_held_signal_counter_counts_each_outcome_after_its_commit`. |
| Security nit: alert detection as `sagat_app` | fixed | `alerts.rs:62-75`, `:249` (`runtime_pool_with(4)`); hand mutation of D022-130 item 8. |
| Nit 2: cursor stamped ahead | fixed | `flow_control_keys.rs:477-484`; tests `a_cursor_stamped_more_than_a_minute_ahead_is_refused`, `rest::an_expired_cursor_is_refused_with_its_own_message` (`rest.rs:1720-1755`). |
| Nit 3: full 255-character sender | fixed | `held_signals.rs:141-142`; test `a_dropped_signal_keeps_the_whole_name_of_its_sender` (`:1804`). |
| New table `flow_control_grant_failures` | no finding | Tenant-led key, `(tenant_id, namespace_id, run_id)` foreign key with cascade, closed `failure` CHECK, no error text or raw key, `REVOKE ... FROM PUBLIC`, `sagat_app` `SELECT, INSERT, DELETE`, insert bound to the run's tenant and namespace (`maintenance.rs:262-296`), classified (`storage-classification.json:9632-9667`), down a bare `DROP TABLE`. |

## New finding 1: the residual is a leak (minor; must be fixed)

D022-100 proves only that the unfiltered view leaks no more than the `group` filter, not that the filter leaks nothing: both page
inside a group every 200 keys, so `spansPages` and the page count depend on the group's live-key count n. A developer with
`run:start` on a workflow keyed into group G and `ops:read`, without the reveal right, holds m runs on m fresh keys and reads
`?group=G`: `spansPages` is false exactly when n + m ≤ 200; a binary search over m (about 8 rounds) gives n; when a victim run of
G ends, n drops by one exactly when no other run used its key, which tells which runs share a key at any limit (the linkage
D022-061 and D022-086 item 5(b) forbid; batch 4 rated the plain key count security Major 1); a run on a chosen value X leaves n
unchanged exactly when X is live, a presence test. Nothing else this caller may use gives the key-sharing link above limit 1.

Smallest full fix: the GROUPS view never pages inside a group, filtered or not. Each group summed by one aggregate over its whole
key range (holders, waiters, oldest wait, the holders' oldest acquisition; rate windows the same); `KeyPosition::in_group` and the
partial sums leave the cursor; cursors fall only between groups; `spansPages` always false or removed; a page holds at most 5
groups, each bounded by the namespace's `concurrency_live_keys` ceiling (hard limit 100,000). Proof: the 100,000-key test on one
page within the 2-second timeout, and a test that groups of 200 and 201 keys, with and without the filter, give the same page shape
and flags, red if paging inside a group returns. If the aggregate cannot meet 2 seconds at the ceiling, page each group by a fixed
number of digest-range slices, the same for every group whatever its size. `rest::a_group_view_breaks_a_group_where_its_own_filter_does`
asserts the residual and must be replaced. Suspicion: a very large group may still time out with a 503; coarse, no single-key
resolution; measure it at the ceiling.

## Nits

1. `held_signals.rs:658-664`: for an unknown drop reason the fallback `_ => self.held_signal` keeps an earlier `Delivered`, so a
   drop could be counted as delivered; make the reason a closed enum.
2. No test uses the exact scenario of suspicion 4, a `waitEvent` inside a loop body; add one case to
   `the_entry_path_holds_what_the_release_of_a_start_reaches`.
