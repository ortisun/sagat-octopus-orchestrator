# Performance and scalability report: feature 010 (scalability and high availability)

Measured by `tests/performance/scalability-ha/main.rs`, run by `tests/performance/scalability-ha/run.sh`
(`SAGAT_PERF_MAX_WAITS` = 1000000). **These are measurements of one machine on one day, not promises** (`NFR-010-PERF`):
each section says what it measured and on what. The machine was shared with other work, which the load
average of each section shows; absolute figures vary from run to run, and the claims that matter are the
structural ones (what grows with what).

The three things the numbers must not be confused with: the **capacity to wait** (how many suspended runs
the database holds, and what each costs), the **active throughput** (step transitions per second), and the
**latency under backlog** (how late a timer fires when many fall due together).
