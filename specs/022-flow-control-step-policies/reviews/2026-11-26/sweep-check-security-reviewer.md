# Security check of the sweep fix (2026-11-26, commit 01c6f916c)

Recorded by the implementer from the coordinator's relay of the security reviewer's check.

Verdict on the findings of the alerts-fix check: finding 1 (`getRun`'s `rateLimit.position` without the label) is fixed;
finding 3 (defence in depth: the drop in the service, `AlertReader::list_alerts` out of the production trait) is fixed;
finding 2 (a flow-control alert's state reaching a caller without the reveal right) is fixed only in part.

Items to close:

1. (medium) The flow-control rows are dropped after the statement's `LIMIT`: `list_alerts_as`
   (`crates/infra-postgres/src/operations.rs:878-883`) limits with no filter on the kind, `OperationsService::alerts`
   (`crates/application/src/operations.rs:685-687`) drops afterwards, and the route (`routes/operations.rs:64-79`) asks
   for one more row and sets `nextCursor` from what is left. For a Viewer, `GET /alerts?state=FIRING&limit=1` answers
   `nextCursor: null` or a cursor depending on whether the hidden saturation alert fires: one bit a request, and
   cursors at visible ids locate a hidden row of any age. It also breaks the Studio's paging. Filter in the same
   statement, keep the service drop as a guard, and test through REST as a Viewer (the flow-control alert firing, then
   resolved: equal bodies, ids and `nextCursor`), red on the current code.
2. (low) `getRun` still runs the place read (`runs.rs:2454` to `flow_control.rs:943`, a 2 s timeout) for a caller who sees
   no label; its response time grows with the key's grants and waiters. Skip the place read when the label is hidden.
3. (low, test gap) `operations::tests::a_flow_control_alert_sends_no_event` restates the predicate only. Add an
   integration test: a subscription to `ops.alert.fired`, a saturation alert fired, no webhook delivery of that kind,
   while another kind delivers one; red with the filter removed.
4. The sweep table of D022-151: add alert paging (page size, `nextCursor`), `getRun`'s response time, the run's
   telemetry and attempts (timing class). Correct the row of the full-queue refusals and gap G022-040's text: with the
   position withheld, a caller with the start right can start its own runs on a chosen value and count how many queue
   before the refusal, so it learns other runs' waiters on that key; fold that and the reset's `failureReason` into
   G022-040 as part of the owner question, with corrected reasons (not "inherent"). Correct the description of the
   matrix: it varies the rate limit (1 and 2) with the waiter cap fixed at 2.
5. The run's own gate timing (the implementer's gap, renumbered G022-043): a limit of the guarantee, not a fixable
   finding. A "held back" state or times cut to the minute are defeated by polling `getRun` or by the run's later
   events; closing it means removing `run:read`. Reword FR-022-022, which overclaims ("no surface MUST tell which runs
   share a key"): no surface shows a key's label, count, rank, fullness or per-key condition; the order and times of
   runs' own progress, which any reader of the runs sees, can suggest key sharing. Close G022-043 as an inherent, stated
   limit, and state it in `docs/architecture/security-and-sandbox.md`.

Conclusion: the family closes after items 1 to 5.
