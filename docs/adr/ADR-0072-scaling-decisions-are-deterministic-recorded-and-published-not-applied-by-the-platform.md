# ADR-0072: Scaling decisions are a deterministic evaluation, recorded with their inputs and published; the platform applies only what lives inside a replica

**Status:** Proposed (2026-10-28)
**Decided by:** agent proposal for the owner's review; the owner's answers O1706 to O1709 of 2026-10-28 (`docs/owner-decisions.md`) fix the boundary it states
**Refines / supersedes:** refines ADR-0020 (lease with a fence, observed without a tenant on a metric) and ADR-0017; supersedes nothing
**Enforced by:** property tests and golden vectors of `crates/domain/src/scaling/` (`contracts/evaluation.md`), `scripts/check-architecture.py` (the domain stays pure), the two-evaluators fault test, `no_metric_is_labelled_with_something_that_grows_with_the_data`, and the chart contract test that every autoscaler query uses a metric the code emits

## Context

Feature 010 divided the scheduler by shard and let worker pools scale on their queue, with an autoscaler reading two series. Nothing scaled consumers, nothing explained a decision, and the platform could not be told what to scale on except by editing a chart. The specification of feature 036 asks for telemetry-driven horizontal and vertical scaling, definable by configuration and by program, with the signals and the decisions handed to the autoscaling provider.

Facts that bound the design. The platform has a lease-and-fence pattern that survives pauses and partitions (ADR-0020) and a monitor under a lease that exports gauges and **zeroes them when it loses the lease or a sample fails**. A zero is what an autoscaler reads as "no load, scale in". The owner chose that the provider (KEDA or another) applies replicas, with an optional applier component later, and that outside data is pulled by the platform (O1706, O1707).

## Decision

1. **One pure function decides.** `evaluate(policy_version, samples, state, context, now)` lives in the domain crate, reads no clock, database or random source, and returns a decision record. The loop and the dry evaluation of the management interface call the same function.
2. **One evaluator at a time.** The evaluator takes a lease in `engine_leases` and the transaction that records a decision MUST first read that lease row `FOR SHARE` and proceed only if it still names this holder and is unexpired on the database clock. A stale evaluator's decision is refused, never merged.
3. **A decision is recorded with its inputs** in the transaction that writes the desired capacity: policy version, each signal with value, age, state and whether it was used, the raw demand, every limit applied in order, the final value and the outcome. Records are append-only; identical consecutive decisions coalesce; retention is configurable (14 days initially).
4. **The order of limits is fixed and the safety ceilings are last:** usable signals, raw demand, stabilization, step, bounds with overrides, hold or freeze, then the role ceiling, the database connection budget and the cost ceiling. No override, freeze release or signal can pass a ceiling.
5. **Ceilings, the destination allowlist, the database budget and the pool names are configuration only.** The management interface cannot raise or widen them.
6. **A stale, missing or unavailable signal MUST NOT be used,** and a failed measurement MUST NOT be represented as zero. When no signal of a policy is usable the unit holds its capacity. Coverage below 1 makes a signal a lower bound that may raise demand and never lower capacity.
7. **The platform publishes; it applies only inside a replica.** The desired capacity of a unit is published with an authority, `advisory` (recommend) or `authoritative` (enforce). The platform process MUST NOT call the orchestrator to change replicas or resources. Parameters inside a running replica (slice 3) are applied by the platform itself. The optional applier of slice 3 is a separate component with its own permissions.
8. **Every replica of the engine and the control plane exports the desired capacity and the signal states from stored state,** not the evaluator alone. If the evaluator is down the stored value is still exported (hold). A signal that is not available is exported as `NaN`, never 0. The chart's autoscaler follows `authority="authoritative"` only.
9. **Metrics carry no tenant, namespace, connector, run or activity.** Labels are the fixed unit, signal, role, limit, outcome, authority, kind and state names, bounded by validation (64 signals, 32 policies).

## Considered options

1. **A pure function recorded with its inputs, one fenced evaluator, publication from stored state** — reproducible (FR-036-027), safe under pauses (the fence), holds when the evaluator dies, and gives the provider a value that is never a false zero.
2. **Let the autoscaling provider compute the decision from raw series** (its own expressions, one trigger per signal) — nothing to explain, no recorded inputs, the ceilings and the database budget cannot be applied consistently across units, and every operator re-derives the same formulas. Still supported: exported signals (FR-036-061) can feed it, but it is not the model.
3. **A separate controller deployment that both decides and applies** — another credential with power over the cluster in the first delivery, and a second authority beside the database for "who decides". Deferred as the optional applier, which applies decisions and does not make them.
4. **Export under the lease and zero on loss, as the monitor does** — simplest, and wrong here: a zero reads as scale in exactly when the evaluator is gone.
5. **Predictive or learned scaling** — out of scope of the specification.

## Consequences

- Good: an operator can read why a unit has the capacity it has, and reproduce it. The provider keeps working through an evaluator outage. The decision function is tested without a database.
- Bad: stored-state publication adds a read per replica per sampling interval and a table (`scaling_desired`); the exporter and the evaluator can disagree for one interval. The provider's own stabilization and cooldown add to the platform's (`G1711`); the chart sets the provider's neutral in the `platform` mode.
- Harder: what the provider does with an absent or `NaN` series is not known from documentation here; the kind and KEDA end-to-end test decides it (`G1703`). The runtime role writes the scaling tables, as it writes the lease tables (feature 010, G259); the configuration-only ceilings are what bounds a compromised control plane.
- Compatibility: additive tables; the existing `lag` and `cpu` autoscaling modes, metrics and alerts are unchanged; a deployment with no policy behaves as before. Rollback is the engines; the schema stays.

## Evidence

Specification `specs/036-elastic-scaling/spec.md` (FR-036-014 to FR-036-031, FR-036-043 to FR-036-048); `research.md` D1700 to D1706, D1719 to D1723, D1727; `contracts/evaluation.md`; feature 010's report for the baseline it improves on. To be added when built: the property-test run, the two-evaluators test, the kind and KEDA report.
