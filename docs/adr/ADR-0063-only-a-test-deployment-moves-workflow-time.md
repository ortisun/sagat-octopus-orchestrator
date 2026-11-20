# ADR-0063: Workflow time and infrastructure time are separate classes, and only a test deployment may move workflow time

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26)
**Refines / supersedes:** refines ADR-0020 (the database clock decides; stable `now()` bounds candidate ranges) and
ADR-0019 (the first day runs on the real engine); supersedes none
**Enforced by:** planned checks of feature 028 — the time-classification catalog test
(`crates/infra-postgres/time-classification.json` against `pg_catalog`: columns, column defaults and `pg_proc`
bodies), the manifest-driven SQL lint and the extended ADR-0020 pass-file test, the domain newtypes (a mixed comparison
does not compile), an equality test of the two instants of one sample at offset zero, plan-equality tests of the
scheduler passes, the start-up refusal tests of the control plane and the engine and the engine monitor's re-check,
the production `404` test for `/v1/testing/*`, the architecture-check rule that only `apps/test-server` and `tests`
depend on `sagat-infra-testing`, and `scripts/check-production-image.py` (the Dockerfile builds and copies no test
server; no production binary's dependency tree contains `sagat-infra-testing`). None exists yet.

## Context

Feature 028 needs a time-skipping test environment: an SDK test harness over the real engine in which a 30-day
timer, a retry backoff or an approval expiry happens in seconds (Temporal's test server does this). ADR-0019 already
rejected a second engine, so the skipping must happen in the real one.

The platform decides time on the database clock (ADR-0020). The run transaction samples `clock_timestamp()` after the
run lock; candidate selection bounds index ranges with `now()`; worker sessions, shard leases, lease expiry, rate
buckets, inbox windows, cleanup leases and audit use the same clock. At the baseline, `crates/infra-postgres/src` has
172 lines that call `clock_timestamp()` (or `statement_timestamp()`/`transaction_timestamp()`) and about 40 more whose
SQL calls `now()`. The application `Clock` port (`crates/application/src/clock.rs`) covers only a few services. Today the
conformance suite moves time by back-dating rows (`tests/contract/sdk-conformance/control.rs`), which leaves the
journal on one time and the rows on another and is not available to users.

Two facts make the choice hard to reverse. Whatever is moved must be moved everywhere it is compared, or a predicate
compares instants from two clocks; and whatever is added to production to make it movable must not change production
behaviour or plans, because every in-flight run depends on it.

## Decision

1. Every instant the platform persists or compares MUST be classified as **workflow time** (timers, wait and approval
   deadlines, retry release, run/step/attempt and chain deadlines, loop delays, cron and schedule firing, and the
   recorded instants of a run's history) or **infrastructure time** (leases, worker sessions, shard and engine leases,
   rate buckets, deduplication and receipt windows, token and credential validity, cleanup, telemetry,
   synchronization jobs, audit). Two stored instants of different classes MUST NOT be compared, and a comparison or
   difference with "now" (a due check, a lag metric, a retention age) MUST use the now of the instant's class. The
   classification of every feature's instants, including those of the parity drafts, is one published catalog
   (feature 028 `contracts/time-classes.md`, coordinator decision C13).
2. Workflow time MUST be derived from one infrastructure sample through one SQL function,
   `sagat_workflow_from(ts timestamptz)` (`STABLE`, production body `SELECT $1`): selection bounds index ranges with
   `sagat_workflow_from(now())`, and the post-lock sample is one materialized `clock_timestamp()` from which both
   instants are taken. Two volatile clock calls MUST NOT be used for one decision (they differ even in production).
   Infrastructure time keeps the built-ins. In Rust the two classes MUST be distinct types, taken from the same sample,
   and equal in production.
3. Only a **test deployment** MAY replace the body of that function to add a non-negative, monotonic offset. A test
   deployment is a database marked by the test server's own migration and served by `apps/test-server`, which starts
   only with `SAGAT_ENV=dev`, on an empty or already-marked database, holds one tenant, and moves the offset only by
   audited transactions. Infrastructure time is never moved.
4. Production composition roots (the control plane and the engine) MUST refuse to start on a marked database or one
   whose function body is not canonical, and the engine's monitor MUST repeat the body check each round. The function
   is owned by the migration role, which the runtime role cannot replace. The profile is selected by the binary at
   compile time, never by configuration. No production image contains the test server, no production binary depends
   on the test-only crates, and no shared crate gains a testing cargo feature. The test server is a separate process
   because of principle V (isolation of test-only code), not for scale.
5. Moving time MUST use the production scheduler passes: in `step` mode the clock stops at each due instant in order
   and the passes run to quiescence before the next stop; in `jump` mode the clock moves once and due work is processed
   late under existing rules. The reducer, transactions, leases, fencing and "first commit wins" are unchanged.

## Considered options

1. **Two classes, workflow time virtual in a test deployment only (chosen).** Production derives the workflow instant
   from the same sample as today through an inlinable function; test deployments shift only what a workflow can observe, so leases, sessions and
   shard ownership keep working across an advance.
2. **Shift the whole database clock** (a global offset on every sample, or `libfaketime` under PostgreSQL). Simpler to
   wire, but each advance would expire every shard lease, worker session, inbox window and rate bucket at once — a
   fail-over at every stop, slower and less deterministic tests — and audit records would carry fictitious times.
3. **Back-date rows** (the conformance suite's current method). Needs no production change, but the journal and the rows
   disagree, replay no longer equals the projection for the touched fields, and every new time column needs its own
   back-dating code.
4. **A test-only cargo feature on the store and scheduler crates.** Production SQL would stay byte-identical, but the
   production Dockerfile builds all binaries in one cargo invocation, where features unify: one test package in that
   invocation would put virtual time into production binaries.
5. **Per-namespace virtual clocks.** Better isolation for parallel tests, but every sample would need a namespace
   argument, selection predicates would lose their index ranges in tests, and cross-namespace deadlines would compare
   two clocks. Deferred (feature 028 G1400).

## Consequences

- Every feature that stores an instant must classify it in the published catalog (018 schedule times and start delays,
  019 detached timeouts and token expiry, 020 retention ages, 022 queue deadlines and cache TTLs, 025 budget windows,
  026 link expiry and approval deadlines, 030 operation deadlines, 032 epochs). The catalog test makes forgetting a
  build failure.
- Lag metrics and retention ages over workflow-class columns run on workflow time; in a test deployment, advancing the
  clock far can trigger retention (feature 028 G1420), and journal-derived spans carry workflow-time stamps (G1418).
- The refactor touches the run transaction and the scheduler passes; plan-equality and zero-offset regression suites
  are release gates for it. Costs are benchmark criteria until measured (feature 028 NFR-028-PERF).
- In a test deployment, run and journal instants are in workflow time and audit records in real time, by design.
- Rollback: the functions remain with their production bodies; removing them is a code revert, not a data migration.
  In-flight runs are unaffected because production values never change.

## Evidence

None yet: this is a proposal. The design, its failure matrix and its tests are in
[feature 028's plan](../../specs/028-developer-tooling/plan.md) and
[research](../../specs/028-developer-tooling/research.md) (D1400–D1408, D1439); the baseline counts above were taken
on 2026-10-26 from `crates/infra-postgres/src` and its migrations.
