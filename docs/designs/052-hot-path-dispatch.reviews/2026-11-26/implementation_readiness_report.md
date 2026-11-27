# egm-implementation-readiness on design 052, full round of 2026-11-26

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: d734e0266bc4333e3e2cf8944a37034d296ab278
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing; it did not repeat `comprehension_report.md`.

## Blocker

1. The wake has no path from a commit to the registry: both commit methods return nothing
   (`PathExecutionTx::commit`, `RunTx::commit`); steps are made ready by completions, run starts, scheduler passes, the
   event dispatcher and the loop and chain stores; no registry type, owner or publish trait is named (the application
   crate cannot depend on `infra-nats`); the stream loop discards its `SessionRecord`. Fix: a `WakeSink` trait and a
   `WakeRegistry` in `crates/application/src/worker/wake.rs`; each transaction collects its keys and counts where it
   writes `task.ready` and calls the sink after a successful commit inside `commit()`; one registry per process handed
   to every store and to `WorkerGrpc`; the stream keeps its `SessionRecord`.

## Major

1. Only the execution engine serves worker streams; run starts commit in the control plane, which reads no NATS
   settings, so CUJ 2's first wake cannot be published; with the Redis transport there is no NATS. Fix: a publish-only
   core connection in the control plane; the files and the Helm template.
2. `NatsPublisher` holds only a JetStream context, cannot subscribe, refuses subjects outside `sagat.v1.`. Fix: a
   `NatsWake` type with a core publish and one `sagat.wake.v1.>` subscription per process.
3. The bound and the ineligible sample: `claim()` gets one list; whether a poll ended by the bound still tries the
   sample is unsaid. Fix: pass `eligible_len`; on the bound, jump to the sample and walk it uncounted.
4. The two-claimer barrier test has no done-condition and never exercises the spread (each session polls once). Fix:
   keep the existing test; a new one asserting round 1 as today, then a second poll's bound.
5. `a_session_that_met_contention_starts_spread_by_its_hash` cannot fail reliably (one session in eight has `h` 0). Fix:
   a unit test with fixed bytes; the integration test picks a session with `h` not 0.
6. The wake tests have no checkable done-condition (`idle_retry` 500 ms passes a loose assertion), no hook between the
   empty claim and the park, a process-wide registry would bypass NATS, Requirement 052-012 untested, no suite named.
   Fix: `idle_retry` 60 s with a lease within 1 s; a park probe; a registry per instance; three named tests; the suite
   and a selector rule for `crates/infra-nats`.
7. Nothing builds the gate's harness (the scenario writes Markdown only; no script pairs, computes and exits); where sd
   lives; the job without a pull request base. Fix: a JSON output, `scripts/throughput-gate.py` with tests, the job on
   pull requests only.
8. Task 052-6's pilot needs a harness and a workflow no earlier task builds; Task 052-8 comes after the task it gates.
9. The gate's selection differs from the requirement (the budget target is also selected by the step-log rule); no
   selector output. Fix: a `throughput_gate` output from design 038's rule only, tested, in the `changes` job.

## Nit

1. The kept `a_poll_alone_keeps_the_order_of_the_scan` cannot catch a spread bug.
2. The counters' file (`worker/claim_counters.rs`) and test are missing; `claim_run` is in `worker/scoped_claims.rs`.
3. The three settings' types, parsing and wiring are unsaid.
4. No direct dependency provides `xxh3`; adding one changes a manifest. Fix: `xxhash-rust` named, or the identifier's
   own bytes.
5. Task 052-1 is partly done; `docs/architecture/execution-semantics.md` never describes the window order.
6. The sample test needs the ChildRoot capacity guard's fixture, off in production.
7. `a_lost_wake_is_bounded_by_idle_retry` passes today; name the mutation that makes it fail.
8. The gate uses unary polls and cannot see stream or wake regressions.

Counts: Blocker 1, Major 9, Nit 8
