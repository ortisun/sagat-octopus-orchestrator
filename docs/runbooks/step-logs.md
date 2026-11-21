# Runbook: step logs (feature 041)

Operational notes for the step logs of design 041 (`docs/designs/041-step-logs.md`): the lines a step's code writes to
its logger, stored per attempt beside the journal with ceilings and a retention period. The architecture decisions are
ADR-0084, ADR-0085, ADR-0095 and ADR-0097. This runbook is completed by the design's tasks T041-10 (the purge, the
retention rule and the deployment switch) and T041-25 (the alerts and their entries); until then it holds the sizing
facts the owner decided on.

## 1. What capture costs

Capturing step logs adds work to every report that carries records: the report's transaction stores the attempt's last
lines (ADR-0095) and charges them to its writer's step-log quota lease (ADR-0097). Task T041-35 measured this cost on the
hosted runners (4 cores, saturated by the benchmark itself; run 44834024933, recorded in
`specs/041-step-logs/validation-report.md`, decision D041-122), with every step writing ten `info` lines:

| Fleet | Rate with capture on, against capture off | Completion p99 against the allowance | Rate when the output stores nothing new | Log tickets used (of half the pool) |
|---|---|---|---|---|
| 1 API replica, 16 workers | 82.0% | +8.6 ms over it | 87.1% | 7 |
| 8 API replicas, 128 workers | 93.1% | 17.0 ms under it | 91.5% | 5 |

The allowance is the capture-off p99 plus the larger of 10% of it and 2 ms (OD-2026-11-20-1, OD-2026-11-20-5).

The owner accepted the measured cost ("Accept the measured cost", OD-2026-11-20-13). What it means for you:

- **A small fleet on a saturated host pays about 18% of its step rate for capture.** When the database host and the
  replicas share few cores and run near their limit, expect roughly one step in five fewer per second, and a completion
  p99 about 9 ms longer, with capture on. A larger fleet (8 replicas, 128 workers) stays within 10% of its rate.
- **If the rate matters more than the lines**, lower the cost per namespace: the attempt ceilings and the capture switch
  are namespace settings (OD-2026-11-16-9), and a namespace with capture off pays nothing. Capture is on by default at
  `info` (OD-2026-11-13-14).
- **Sizing:** give the database host headroom above the benchmark's saturation point, or scale out replicas, before
  turning capture off.

## 2. Lines not stored because the log service was busy

A carry whose lease must be refilled and cannot get the tenant's usage row within 200 ms drops its records; the report
always lands. The reader sees "not stored: the log service was busy (not your quota)" on the stream and in Studio, and
the drop is counted with the reason `tenant_quota_busy`. The owner accepted this visible loss as built ("Accept the
visible loss", OD-2026-11-20-14; gap G041-026). Task T041-35's fourth run measured 1 such carry of 864,000, far from the
limit with direct connections. It is more frequent near the tenant's limit and behind a transaction pooler (gap
G041-027). If it exceeds 0.1% of carries in a regime that task T041-9 measures, the question goes back to the owner.
