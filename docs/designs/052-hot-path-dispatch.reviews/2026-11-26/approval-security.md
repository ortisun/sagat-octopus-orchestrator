# Approval confirmation of design 052: security (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 58b6fd20e8723d36fdac8be920b5cf5ca13dd3cb
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`security-full.md`, at d734e026, 2
Majors and 5 Nits), then the fixes re-read in `reread-1-security.md` to `reread-4-security.md`, every finding of each
fixed before the next. Not a full review. The reviewer read files only and ran nothing.

Re-read 4's Nit is resolved: Task 052-6 puts `the_control_plane_opens_no_broker_egress_without_the_nats_wake` and
`the_chart_refuses_the_wake_without_its_credentials_secret` in the chart's contract test, `contract_scalability_ha`
(which exists), and a rule of `scripts/select-test-targets.py` with its own test selects it for changes of the network
policy templates where Requirement 052-054 places the broker egress and the credentials check, and of the wake's values
(in practice of `values.yaml` as a whole, which runs the test more often but never misses a change).

Every finding of every security report of this approval run is fixed; none is accepted.

No Blocker, no Major, no Nit.

Counts: Blocker 0, Major 0, Nit 0
