# Feature Specification: Elastic scaling driven by telemetry

**Feature Branch**: `feature/036-elastic-scaling`

**Created**: 2026-10-28

**Status**: Draft — specification proposed; not planned, not implemented; pending owner review

**Input**: User description: "Specify horizontal and vertical scaling capabilities. Today a consumer of a queue or topic cannot be scaled by message lag, event reception and the other background roles have no scaling signal, and nothing can be sized from telemetry. The system must scale from telemetry that is configurable both by configuration and programmatically."

## Context and delivery boundary

Feature 010 made the **scheduler** scale: its work is divided in 64 shards owned by lease with a fence, a
second replica adds capacity, and worker pools scale on the length of their own queue. A review of the code
on 2026-10-28 found what it did not cover, and the owner's request is to close those gaps:

1. **No lag exists for inbound connectors.** A Kafka, NATS or SQS connector reports how many messages it
   consumed and how many failed. It does not report how far behind the broker it is, so nothing can scale a
   consumer by lag, and an operator cannot tell "idle" from "falling behind".
2. **Kafka partitions are not balanced.** Replicas compete for a lease on each partition and the holder
   renews it for as long as it lives. A replica that joins later receives none, and one replica can hold all
   of them: adding a replica does not add read capacity until a lease happens to expire.
3. **Ingestion cannot scale on its own.** Every engine replica runs a reader for every active connector,
   in the same process as the scheduler, the dispatchers and the worker-facing interface. The parallelism of
   a NATS or SQS reader inside a replica is fixed (it takes one message at a time). A connector's rate limit
   already holds for all its replicas together (a shared token bucket in the database, with the namespace's
   events-per-second bucket on top), so **adding readers cannot raise the throughput of a connector that is
   at its rate limit**, and nothing tells an operator that this is why the lag grows (corrected at planning,
   see Clarifications).
4. **Scaling signals exist for two things only**: the scheduler's lateness and the queue of a worker pool.
   Undispatched events, outbound deliveries, outbox age, the saturation of the worker-facing interface and
   of the REST ingress have none; the ingress, when autoscaling is enabled at all, scales on CPU alone.
5. **Some background work is not divided by owner.** The event dispatcher and the delivery worker coordinate
   by claiming rows (feature 010, G243), the worker claim path is unchanged (G246), and the shards are shared
   by count, not by load (G239).
6. **Nothing scales vertically.** Concurrency, batch sizes, connection budgets and per-connector in-flight
   limits are fixed at start; no telemetry can resize them, and resource requests are static.
7. **A failed measurement looks like a zero.** The platform deliberately reports zero when a sample fails, so
   that a frozen value does not scale anything (ADR-0020). A zero is also what a scaler reads as "no load".

This feature turns telemetry into capacity decisions that an operator can **declare by configuration** and a
program can **define, read, evaluate and override through the management interface**, for horizontal scaling
(how many replicas of each role) and vertical scaling (how large each replica's work and resources are).

**What the platform decides and what it applies.** The platform collects signals (its own, and those it
pulls from external sources that the operator declares), evaluates policies, records decisions, and hands both
the signals and the decisions to the autoscaling provider. Parameters inside a running replica (concurrency, batch sizes, in-flight limits, partition
assignment) it applies itself. The number of replicas and their CPU and memory are applied by the environment
that runs them: by default the platform publishes the desired capacity of every role in a form that the
packaged autoscaling integration and external controllers can follow. For operators who want a closed loop
without an external autoscaler, an **optional applier** (a separate component, off by default, holding only the
permissions it needs) carries out the same published decisions in the supported orchestrator (owner decision of
2026-10-28, O1706). Neither replaces the environment's autoscaler (see Out of scope).

**Terms.** *Operator* in this specification always means the platform operator: the person or system that runs
the deployment and holds an operator token. It is not the role named Operator that exists inside a tenant
(which can read connectors and runs there); that role gains no power from this feature. A *tenant administrator*
is a principal of a namespace holding the permissions named in FR-036-023.

**Delivery order (owner decision of 2026-10-28, O1709).** The first delivery is US01, US02, US03, US04, US05,
US06, US10 and US13: lag of consumers, balanced Kafka partitions, scaling on lag, policies by configuration and
through the management interface, explanation of decisions, safety, and pulled sources, with the signals and the
desired capacity exported to the autoscaling provider (SC-036-019). Later deliveries, in an order set at
planning: the other roles (US07), the dedicated ingestion role (US09), vertical scaling (US08), the optional
applier (US12) and the benchmark on separate machines (US11; SC-036-010 and SC-036-014). The lag control loop
of SC-036-004 is measured in the first delivery on the environment available and reported as such. A
requirement that serves several stories is delivered with the first of them; the mapping is made at planning,
and the first delivery must be shippable on its own.

**Dependencies.** 001 (foundation), 002 (execution, claim path), 004 (events, inbox and connectors),
008 (observability), 009 (tenancy, quotas, audit), 010 (shards, pools, packaging and the benchmark harness).
Feature 019 (proposed, not approved) routes work to worker deployments and excludes autoscaling of its push functions and warm pools;
this feature does not depend on it, defines pool signals on the pool concept that exists today, and leaves the
signals of 019's deployments to be added through FR-036-005 when 019 is approved. Feature 033 (Redis Streams)
is outbound and internal only and has no inbound consumer to scale; any transport added later falls under
FR-036-005. Feature 032 (multi-region) is out of scope here.

**Numbering.** Checked against `origin/*` and the sibling worktrees on 2026-10-28 (highest taken: D1671,
G1669, O1599, ADR-0071). This feature takes D1700–D1799, G1700–G1799, O1700–O1799 and ADR-0072 onward; the
registers are opened in `research.md` at planning.

## Clarifications

### Session 2026-10-28

- Q: When a policy is in *enforce* mode, who changes the number of replicas and the CPU and memory of a role? →
  A: Option B — by default the environment's autoscaler or an external controller follows the published desired
  capacity; in addition an optional applier (separate, off by default, minimal permissions) carries out
  enforced decisions in the supported orchestrator, for operators who want a closed loop (O1706).
- Q: May an operator point a scaling signal at an external metrics system or queue by configuration, or may
  outside data enter only as a custom signal that a program publishes? →
  A: Option B — by configuration. The platform queries the declared external sources itself (metrics such as
  queued messages, CPU and memory), evaluates, and hands the signals and the decisions to the autoscaling
  provider (KEDA or another), which scales out and in. Operator-only, bounded and fail-closed (O1707).
- Q: Must programmatic access to signals and policies exist in the six official SDKs, or only in the documented
  REST API and the operator command line? →
  A: Option A — the REST API and the operator command line only. The SDKs do not change in this feature
  (O1708).
- Q: What may a tenant do about scaling: only read the lag and decisions of its own connectors, also adjust their
  consumption by hand within its quota, or also define automatic policies for that? →
  A: Option B — read, and adjust the in-flight parallelism and rate limit of its own connectors by hand within its
  quota. Policies are operator-only. A tenant never controls replicas or resources (O1703). Consequence: a custom
  signal published by a tenant has no automatic effect until an operator admits it to a policy (O1704).
- Q: Should the feature be delivered in a smaller first cut that closes the loop from lag to scaling, or all at
  once? →
  A: Option B — a first delivery of US01, US02, US03, US04, US05, US06, US10 and US13 (consumer lag, balanced
  Kafka partitions, scaling on lag, policies by configuration and through the management interface,
  explanation, safety, pulled sources, signals and desired capacity exported to the provider). The other
  roles, vertical scaling, the dedicated ingestion role, the optional applier and the benchmark on separate
  machines follow (O1709).
- Correction (found while planning, 2026-10-28): the first version of this specification, and
  `docs/runbooks/events.md`, said that a connector's `ratePerSecond` is a limit per replica. The code enforces
  it for all replicas together through a shared bucket in the database. Consequences: FR-036-013 now asks for
  visibility and documentation instead of a new global limit; a *rate-limited* state joins FR-036-003 so that
  scaling out is not decided for lag that more readers cannot drain; O1705 needs no decision.
- Correction (security checklist review, 2026-10-28): reading the requirements against
  `checklists/security.md` showed gaps and two conflicts, and this version closes them. Added: a definition of
  *operator* that cannot be read as the tenant role named Operator (Terms); the permission-to-operation mapping,
  the quota for parallelism and the tenant error rule (FR-036-023); what "missing resource" means
  (FR-036-002, SC-036-013); the allowlist matching, credential, TLS and query rules (FR-036-057 to
  FR-036-059); and FR-036-062 to FR-036-068 for operator authentication and token lifetime, the interface being
  off by default, configuration-only limits re-validated by the evaluator, audit of refusals and of actors,
  credential rotation, and rate limits. Conflicts settled: the specification followed the owner's "within
  quota" and now has a parallelism quota (the plan had a cap only; the plan follows), and tenant-published
  custom signals are stated as a later addition. Nothing in the checklist was ticked by this change.

## User Scenarios & Testing *(mandatory)*

### User Story 1 (US01) - See how far behind every consumer is (Priority: P1)

An operator, or an administrator of the namespace that owns a connector, opens a Kafka, NATS or SQS connector
and sees how many messages wait on the broker side and how old the oldest one is, together with the time of
the measurement, and whether the connector is behind, idle, or blocked on purpose (for example because the
tenant's backlog of undispatched events is full) or failing to read.

**Why this priority**: Without lag there is nothing to scale a consumer on, and nothing to alert on. Every
other story in the consumer path depends on it, and it is useful alone.

**Independent Test**: Against a real broker of each kind, stop the consumption, publish a known number of
messages and observe the backlog equal that number and the age grow; resume and observe both return to zero.
Make the broker unreachable and observe "unknown", never zero.

**Acceptance Scenarios**:

1. **Given** a connector that is reading and 500 messages published after it was paused, **When** its lag is
   read, **Then** the backlog is 500 within the lag tolerance (see Assumptions), the age is at least the time since the
   first of them was published (where the broker can say), and the sample time is present.
2. **Given** a connector whose tenant has reached its limit of undispatched events and therefore stops
   taking messages, **When** its lag is read, **Then** the lag grows and the connector is reported as
   **blocked by backpressure**, not as slow.
2a. **Given** a connector that is held back by its own rate limit or by the namespace's events-per-second
   limit, **When** its lag is read, **Then** the lag grows and the connector is reported as **rate-limited**,
   with the time after which the platform will try again, not as slow and not as blocked by backpressure.
3. **Given** the broker is unreachable or refuses the credentials, **When** the lag is read, **Then** it is
   **unavailable** with the reason class, and is never reported as zero.
4. **Given** a connector of a namespace the caller does not belong to, **When** its lag is read, **Then** the
   answer is indistinguishable from a connector that does not exist, and the attempt is audited.

---

### User Story 2 (US02) - Share a topic's partitions evenly across replicas (Priority: P1)

When replicas that read a Kafka topic come and go, the topic's partitions are redistributed so that every
replica reads about the same number, without an operator moving anything, and a partition is never read by
two replicas at once.

**Why this priority**: Without it a second replica adds no read capacity, and lag scaling (US03) would add
replicas that stay idle.

**Independent Test**: A topic with 12 partitions and a fixed message set: start one replica, then add two;
observe each hold four within the rebalance time; stop one cleanly and one abruptly and observe the others
absorb its partitions within the stated bounds; compare the inbox with the messages sent.

**Acceptance Scenarios**:

1. **Given** one replica holding all 12 partitions, **When** two more join, **Then** within the
   rebalance time (see Assumptions) every replica holds exactly ⌈12 ÷ 3⌉ = 4.
2. **Given** three replicas, **When** one stops cleanly, **Then** its partitions are taken by the others at
   their next round, and **When** one crashes, **Then** they are taken within the lease time plus one round.
3. **Given** more replicas than partitions, **When** the assignment is read, **Then** the surplus replicas
   hold none and the connector reports that it is **partition-bound** (more replicas cannot read faster).
4. **Given** a handover in the middle of a burst, **When** the inbox is compared with the topic, **Then**
   every message is present exactly once and the position moved only forward.
5. **Given** the topic gains partitions while running, **When** the next listing happens, **Then** the new
   partitions are assigned like the rest, without restarting the connector.

---

### User Story 3 (US03) - Scale consumers on their lag (Priority: P1)

An operator declares that the ingestion capacity should follow the lag of the connectors: a target backlog
per replica (or a target age), a minimum and a maximum number of replicas, and how fast to scale in and out.
When a producer outpaces the consumers, capacity rises until the lag is under target; when it recovers,
capacity returns slowly.

**Why this priority**: This is the owner's explicit need ("scale consumers by message lag"), and the first
end-to-end demonstration of the whole model.

**Independent Test**: With a policy in force, publish above the capacity of one replica; observe the desired
capacity rise in steps within bounds, the lag fall under target, and the capacity fall again after the
stabilization window; repeat for Kafka, NATS and SQS.

**Acceptance Scenarios**:

1. **Given** a policy with a target of 1,000 messages of lag per replica, a minimum of 2 and a maximum of 8,
   **When** the total lag is 5,200 messages, **Then** the desired capacity is 6 and the record explains it.
2. **Given** a Kafka connector with 4 partitions, **When** the policy would ask for 8 replicas, **Then** the
   desired capacity of the useful readers is clamped to 4 and the record says it was partition-bound.
3. **Given** a connector that is blocked by backpressure or held back by its rate limit, **When** its lag is
   high, **Then** no scale-out is decided for it, and the record says why (more readers cannot drain it).
4. **Given** the lag has been under target for the whole scale-in window, **When** the policy is evaluated,
   **Then** capacity falls by at most the step limit, never below the minimum.

---

### User Story 4 (US04) - Declare signals and scaling policies by configuration (Priority: P1)

An operator writes, in the deployment's configuration, which telemetry drives which capacity: signals, targets,
bounds, stabilization, cooldowns and a cost ceiling. The platform validates the declaration before it takes
effect, keeps every version, and does nothing unexpected when the declaration is wrong or absent.

**Why this priority**: Configuration is how operators run the platform today, and the safe default (no
declaration, no change) is what lets everything else ship without risk.

**Independent Test**: Apply a valid policy and watch decisions follow it; apply an invalid one and observe it
refused with the problem named and the previous version still in force; remove all policies and observe that
nothing changes automatically.

**Acceptance Scenarios**:

1. **Given** a deployment with no policy, **When** it runs under load, **Then** no role changes its capacity
   or its tunable parameters automatically, exactly as before this feature.
2. **Given** a declaration that names an unknown signal, a minimum above a maximum, or two policies for one
   target, **When** it is applied, **Then** it is refused with a typed error naming the cause, nothing
   changes, and the last valid version stays in force.
3. **Given** a policy changed by configuration, **When** the history is listed, **Then** the new version
   appears with author, time and reason, and the previous one can be restored.
4. **Given** the configuration source cannot be read, **When** the platform starts or reloads, **Then** it
   keeps the last valid policies, or none if there were none, and reports the condition.

---

### User Story 5 (US05) - Manage signals and policies programmatically (Priority: P1)

A program (an operator's controller, a deployment pipeline, a tenant's automation) uses the management
interface to define, read, update and delete policies and signals, to ask what a policy would decide for given
values without applying it, to override a decision for a limited time, and to publish its own signals.

**Why this priority**: The owner requires programmatic control as a peer of configuration, not an afterthought.

**Independent Test**: Create the same policy as in US04 through the interface and verify that identical samples
give identical decisions; run a dry evaluation; apply a time-boxed override and see it expire by itself;
publish a custom signal and see a policy use it.

**Acceptance Scenarios**:

1. **Given** a policy created through the interface and the same policy created by configuration, **When**
   the same samples are evaluated, **Then** the decisions are identical.
2. **Given** a policy, **When** a caller asks for a dry evaluation with supplied sample values, **Then** the
   answer is the full decision record and nothing is applied or audited as a change.
3. **Given** a policy owned by configuration, **When** a caller tries to edit it through the interface,
   **Then** it is refused and pointed to the override, and **When** a caller creates a time-boxed override,
   **Then** it applies until it expires and is audited.
4. **Given** a principal without the permission to manage policies, **When** it tries, **Then** it is refused
   without revealing whether the policy exists.
5. **Given** a published custom signal, **When** its time to live passes without a new sample, **Then** it is
   stale and no decision uses it.
6. **Given** a tenant administrator and a connector of the tenant's namespace, **When** they raise its in-flight
   parallelism or rate limit within their quota, **Then** it applies to the running replicas without recreating
   the connector, and **When** they ask for more than the quota allows, **Then** it is refused with a typed
   error; both are audited.
7. **Given** a tenant principal, **When** they try to create or change a scaling policy, or to control replicas
   or resources, **Then** it is refused, while reading the lag of their own connectors still works.
8. Once tenant publication exists (not in the first delivery): **Given** a custom signal published by a
   tenant, **When** no operator has admitted it to a policy, **Then** it changes no capacity and no parameter,
   and the publisher can read it.

---

### User Story 6 (US06) - Understand why capacity changed (Priority: P1)

For any change, or any deliberate decision not to change, an operator can read what the platform saw and why
it decided as it did: the signals with their values and freshness, the target, the capacity the signals asked
for, every limit that reduced it, and the final capacity.

**Why this priority**: An autoscaler that cannot explain itself is switched off at the first incident.

**Independent Test**: Provoke a scale-out clamped by the partition count and by the database connection
budget; read the record; re-evaluate the recorded inputs through the dry evaluation and obtain the same result.

**Acceptance Scenarios**:

1. **Given** any decision, **When** its record is read, **Then** it lists each signal's value, age and
   whether it was used, the raw desired value, each clamp applied in order, and the applied value.
2. **Given** a record, **When** its inputs are submitted to the dry evaluation of the same policy version,
   **Then** the same decision results.
3. **Given** the scaling metrics of the platform, **When** they are scraped, **Then** no series carries a
   tenant, namespace, connector, run or activity identifier.

---

### User Story 7 (US07) - Scale the other background roles on their own backlog (Priority: P2)

The event dispatcher, the outbound delivery worker, the scheduler, the worker pools, the worker-facing
interface and the REST ingress each have signals of their own and can be scaled on them. The event dispatcher
and the delivery worker can be divided among replicas so that a new replica adds capacity.

**Why this priority**: The same mechanism has to cover the whole platform, or only consumers scale and
the next bottleneck is the dispatcher.

**Independent Test**: Create a backlog of undispatched events and of pending deliveries; observe the signals
rise, the policy ask for more capacity, and throughput grow with replicas up to the limit the benchmark
states; saturate the worker-facing interface and the ingress and observe their signals.

**Acceptance Scenarios**:

1. **Given** 100,000 undispatched events across tenants (each tenant is held to its own backlog limit) and one dispatcher replica, **When** a second joins, **Then** the
   backlog drains faster by the measured factor reported for the environment, with no event dispatched
   twice into the same effect and no tenant starved.
2. **Given** the worker-facing interface shedding calls, **When** the signal is read, **Then** it shows
   saturation and the number shed, and a policy on it asks for capacity.
3. **Given** a role with a hard ceiling (64 scheduler replicas), **When** a policy asks for more, **Then** the
   decision is clamped to the ceiling and says so.

---

### User Story 8 (US08) - Size replicas from telemetry (vertical scaling) (Priority: P2)

An operator declares that a replica's concurrency, batch sizes, in-flight limits and connection budget, and
its CPU and memory, follow telemetry within bounds. In-process parameters change on a running replica without
a restart; resources change by a declared mode (rolling replacement by default, in place only when enabled).

**Why this priority**: Adding replicas is not always the cheapest answer, and static sizing leaves capacity
unused or starves it; but it is only safe after the signals and the decision record exist.

**Independent Test**: Raise load on a role with a concurrency policy; observe the parameter rise on a running
replica without a restart and without losing a lease; drive the database connection budget to its limit and
observe both vertical and horizontal increases refused.

**Acceptance Scenarios**:

1. **Given** a policy on a role's concurrency between 4 and 16, **When** its signal asks for 12, **Then**
   the running replica uses 12 within the time for a tunable change (see Assumptions), without restarting and without losing a lease.
2. **Given** replicas whose connection budgets sum to the database's declared capacity less the reserve,
   **When** any policy asks for more replicas or a larger budget, **Then** the increase is refused or reduced,
   and the record names the budget as the limit.
3. **Given** a memory policy, **When** observed peak usage over the window is near the limit, **Then** the
   limit is not lowered.
4. **Given** a resource change with the mode `rolling`, **When** it is applied, **Then** replicas are
   replaced one at a time and the role never falls below its minimum available replicas.

---

### User Story 9 (US09) - Run ingestion as its own role (Priority: P2)

An operator can run the connector readers in a deployment role of their own, scaled independently of the
engine's other roles, while the default stays "everything together" as today. Replicas with different roles
can run side by side.

**Why this priority**: Lag scaling is worth little if the only way to add readers is to add schedulers
and workers' interface; it is P2 because US03 can be shown with the combined role.

**Independent Test**: Run an ingestion-only deployment and an engine without readers, consume a topic with
loss and duplicates checked; then mix combined and separated replicas and repeat.

**Acceptance Scenarios**:

1. **Given** a deployment where connectors are read only by ingestion replicas, **When** the engine replicas
   are scaled, **Then** no connector reads move and none are read twice.
2. **Given** combined replicas and ingestion-only replicas at once, **When** a topic is consumed, **Then** the
   inbox holds each message exactly once and the partitions are balanced across all readers.
3. **Given** a deployment with no role selection, **When** it runs, **Then** it behaves as before.

---

### User Story 10 (US10) - Stay safe when telemetry fails or is abused (Priority: P1)

When a signal is missing, stale, absurd or raised on purpose by a tenant, the platform holds its capacity,
never exceeds its ceilings or the database's capacity, can be frozen at once, and keeps running if the
scaling machinery itself stops.

**Why this priority**: A scaler that amplifies a monitoring outage or a tenant's burst turns an incident into
an outage and a bill.

**Independent Test**: Cut the telemetry source; feed random, extreme and adversarial samples to the decision
function; kill the evaluator in the middle of a decision; flood a shared signal from one tenant.

**Acceptance Scenarios**:

1. **Given** every signal of a policy is stale or unavailable, **When** it is evaluated, **Then** the role
   holds its current capacity, a condition is raised, and no scale-in is decided on an absent signal.
2. **Given** one tenant produces a burst that dominates a shared signal, **When** the policy decides, **Then**
   the desired capacity does not pass the cost ceiling or the step limit, and the operator-only record shows
   that one scope dominated the signal.
3. **Given** the operator freezes scaling, **When** the next evaluation runs, **Then** no automatic change is
   made, and the freeze is audited with its author.
4. **Given** the scaling evaluator stops, **When** the roles keep running, **Then** they keep their last
   applied capacity and parameters and no work is lost.

---

### User Story 11 (US11) - Know the limits from evidence on separate machines (Priority: P2)

The capacity claims of this feature, and the limits that remain, come from a benchmark in which the database,
the platform and the workers run on different machines, with the environment, the failures and the limiting
resource stated.

**Why this priority**: Feature 010's report was taken on one machine and did not reach 5,000 transitions per
second; the constitution forbids a number without its environment.

**Independent Test**: Run the harness on the stated topology; the report contains the environment, the lag
control loop, the throughput with replicas of each role, and the outcome against each load level, reached or not.

**Acceptance Scenarios**:

1. **Given** the benchmark topology, **When** the report is produced, **Then** it states hardware, network
   between services, database configuration, workload and the limiting resource of every figure.
2. **Given** a load level that is not reached, **When** it is reported, **Then** it says "not reached" and
   what limited it.

---

### User Story 12 (US12) - Close the loop with the optional applier (Priority: P3)

An operator who does not run an external autoscaler enables the optional applier for a role. From then on the
decisions of enforced policies on that role are carried out in the orchestrator, replica count and resources,
within the ceilings and the change mode, and every action is audited.

**Why this priority**: The owner chose to offer a closed loop (O1706). It is P3 because every other story works
with the environment's own autoscaler following the published decisions.

**Independent Test**: With the applier absent, drive load and verify that nothing in the orchestrator changes;
enable it for one role and verify that the replicas follow the authoritative decision; change the replica count
by hand and verify that the applier reports a conflict and holds.

**Acceptance Scenarios**:

1. **Given** the applier is not installed, **When** policies run in *enforce* mode, **Then** the platform
   changes nothing in the orchestrator and publishes the decisions.
2. **Given** the applier is enabled for a role and an enforced policy asks for 6 replicas, **When** the
   decision is authoritative, **Then** the role reaches 6 replicas without passing the disruption limit, and the
   audit record names the decision that caused each step.
3. **Given** a role declared as controlled by an external autoscaler, **When** an operator enables the applier
   for it, **Then** the configuration is refused and names the conflict.
4. **Given** someone changes the replicas of an applier-controlled role by hand, **When** the applier next
   observes it, **Then** it raises a conflict condition and holds instead of fighting the change.
5. **Given** the orchestrator is unreachable or refuses a change, **When** a decision is to be applied,
   **Then** the applier holds and reports, and the role keeps running at its current capacity.
6. **Given** the applier's own credentials, **When** they are tried against secrets, tenant data and workloads
   that are not the platform's own, **Then** every attempt is denied.

---

### User Story 13 (US13) - Scale on metrics the platform pulls from outside (Priority: P1)

An operator declares an external source (a metrics query, or the depth of a queue) and a policy uses its
signal. The platform queries the source by itself at the configured interval, evaluates the policy, and hands
the signals and the decision to the autoscaling provider (KEDA or another), which scales out and in.

**Why this priority**: The owner asked for it explicitly (O1707): queued messages, CPU and memory, and similar
measures live outside the platform as often as inside it, and a signal the platform cannot reach cannot drive a
policy.

**Independent Test**: Declare a source against a test metrics system that returns a CPU figure, and a source
against a test queue; drive their values; verify the signals, the policy decisions and the exported series;
make each source fail in turn and verify that the signal becomes unavailable and the role holds.

**Acceptance Scenarios**:

1. **Given** a source for the CPU use of a role from an external metrics system, **When** the platform queries
   it each sampling interval, **Then** the signal is available with value, unit and sample time, and a policy
   that uses it produces a decision.
2. **Given** a source for the depth of a queue that no connector reads, **When** messages arrive, **Then** the
   depth is available and a policy on it asks for more capacity.
3. **Given** a signal marked exported, **When** the autoscaling provider reads the exported series, **Then** it
   finds name, unit, value, state and sample time and no tenant identity, and the desired capacity is available
   next to it.
4. **Given** a source that is unreachable, slow, or answers with something that is not a finite number or is
   outside its declared range, **When** it is queried, **Then** the signal is unavailable, never zero, the role
   holds, and the next attempts back off.
5. **Given** a tenant principal, a destination that is not on the allowlist, a source with credentials over an
   unencrypted connection, or a query that is not read-only, **When** a source is declared, **Then** it is
   refused with a typed error.
6. **Given** a declared source, **When** the operator tests it, **Then** the answer gives value, unit, latency
   and failure class and no policy or decision is affected.

---

### Edge Cases

- **More replicas than partitions or shards**: the surplus holds nothing; the ceiling is visible and the
  policy is clamped to it.
- **Partitions added or removed while running**; a topic that no longer exists; an offset the broker no longer
  holds (retention): lag must not go negative, and the existing reset behaviour is counted, not hidden.
- **The broker, not the consumer, is the problem**: unreachable broker, refused credentials, throttling.
  Lag is unavailable, and scaling out readers is not the answer.
- **Backpressure by design**: a connector that stops because the tenant's undispatched events are at the limit
  must not cause scale-out; more readers cannot drain a full backlog.
- **Approximate or inflated counts**: SQS counts are approximate; a NATS count includes messages in flight
  and redeliveries. Tolerances are stated and the policy can use age instead of count.
- **Signal oscillating around the target** (flapping), and a policy whose scale-out raises the load it
  measures (more replicas, more database connections, more lag in the dispatcher).
- **Several roles asking for database connections at once**: the budget is shared and must be divided.
- **A signal that a tenant can raise** (timers due together, a flood of events) driving a shared role.
- **Two policies on one target, a policy on a signal that was removed or renamed, a threshold in the wrong
  unit**: refused at validation, or held and reported if it happens later.
- **Configuration and API both define the same policy**; configuration changes during an evaluation.
- **An external autoscaler and the optional applier both acting on the same role**: refused at configuration
  (FR-036-051). An external autoscaler that follows the platform's authoritative desired capacity is not a second
  writer: it is the role's declared controller.
- **A decision superseded before the applier carries it out**: it is dropped, not applied late (FR-036-054).
- **The monitoring system is down, slow, or answers with a very large or label-heavy result**: the signal is
  unavailable, backoff applies, the role holds, and the response bound refuses the excess.
- **A pulled value in another unit or scale than declared, or older than the freshness limit by the source's
  own timestamp**: refused or stale, never silently used.
- **A destination removed from the allowlist while a source uses it**: the next query is refused and the signal
  becomes unavailable.
- **Evaluator replicated**: two evaluators must not apply two different decisions for one policy.
- **Rolling upgrade**: a previous-version engine without roles or without the new signals keeps working next
  to a new one; the new one does not scale on signals the old one cannot provide.
- **Scale-in in the middle of a handover** (lease or partition), and **scale-in of a replica that holds
  unacknowledged messages**: nothing is acknowledged to a broker before it is in the inbox.
- **Clock skew** between the samples of different replicas; **samples out of order**.
- **A tenant deletes a connector, or a pool disappears, while a policy refers to it.**

## Requirements *(mandatory)*

### Functional Requirements

**Signals and lag**

- **FR-036-001**: For every active inbound connector of every supported kind (Kafka, NATS, SQS) the platform
  MUST report its **backlog** (messages on the broker not yet taken into the inbox) and the **age of the
  oldest** such message where the broker can state it, each with the time of the sample. A measure the broker
  cannot provide is reported as unknown, never as zero.
- **FR-036-002**: A connector's lag MUST be readable in detail by the principals of the namespace that owns it
  who already hold `event:read` for it and, without any tenant, namespace or connector identity, in aggregate by
  operators and by external autoscalers. These aggregates are operator data and are never exposed to tenants.
  Another tenant's connector is indistinguishable from a missing one (the same status, body and headers, the
  trace identifier excepted, and no difference in response time beyond the normal variation of a miss) and the
  attempt is audited.
- **FR-036-003**: A connector MUST report why it is not draining: *behind* (reading, slower than the arrival),
  *blocked by backpressure* (held back by the tenant's undispatched-event limit), *rate-limited* (held back by
  the connector's or the namespace's rate limit, with the retry time), *paused*, or *failing to read* (with the
  failure class). Policies MUST be able to ignore lag that more readers cannot drain, and by default MUST NOT
  scale out for lag in the *blocked* or *rate-limited* state.
- **FR-036-004**: The platform MUST provide signals for each other role: undispatched events and the age of the
  oldest; outbound deliveries pending and the age of the oldest; outbox hints and their age; the scheduler's
  lateness per pass (existing); the queue of each worker pool (existing); the saturation (in flight against
  limit, and calls shed) of the worker-facing interface and of the REST ingress; the database connections in
  use against the budget; and CPU and memory per role.
- **FR-036-005**: Every signal MUST have a stable name, a unit, a scope (operator-only or namespace-visible),
  the freshness after which a sample is stale, and a documented meaning, in a catalogue that can be listed. A new
  inbound connector kind or transport MUST NOT ship without its lag signals; a conformance check enforces it.
- **FR-036-006**: Every sample MUST carry one of the states *available*, *stale* or *unavailable*. For the
  purpose of scaling, a failed measurement MUST NOT be represented as zero. The existing behaviour of the
  present metrics and their alerts is unchanged.

**Distribution of consumption**

- **FR-036-007**: The partitions of a Kafka topic read by a connector MUST be divided among the replicas that
  read it so that, when there are at least as many partitions as replicas, no replica holds more than
  ⌈partitions ÷ replicas⌉ and none fewer than ⌊partitions ÷ replicas⌋. The division MUST adjust when replicas
  join or leave, without operator action.
- **FR-036-008**: A partition MUST be read by at most one replica at a time. The position of a partition and
  the admission of the events read from it MUST remain one atomic commit in the platform's database; the
  platform does not hand partition ownership to the broker (see O1701). A handover resumes from the stored
  position with no loss and with no event admitted twice beyond what the inbox already deduplicates.
- **FR-036-009**: A replica that stops cleanly MUST give its partitions back so that others take them at their
  next round; a crashed replica's partitions MUST be taken within the lease time plus one round. Handover time
  is reported.
- **FR-036-010**: When replicas outnumber partitions the surplus MUST hold none, and the connector MUST report
  that it is partition-bound, with the number of partitions as the ceiling of useful readers.
- **FR-036-011**: Partitions added to a topic MUST be assigned like the others at the next listing, without
  restarting the connector; removed partitions release their assignment.
- **FR-036-012**: Each connector MUST have a configurable in-flight parallelism per replica (per partition for
  Kafka) with a platform-wide cap, changeable without recreating the connector. NATS and SQS messages MUST
  spread across all replicas that read the connector.
- **FR-036-013**: The rate limit of a connector holds for all its replicas together, as it already does (a
  shared bucket with a burst of two seconds of the rate, under the namespace's events-per-second limit). The
  platform MUST NOT change that meaning. It MUST make the limit visible as a cause of lag (FR-036-003), and the
  documentation that calls the limit "per replica" MUST be corrected. Raising the limit stays a tenant action
  within the namespace's quota (FR-036-023) and an operator's within the ceilings (O1705, resolved by fact).

**Signals, policies and their definition**

- **FR-036-014**: A *scaling signal* is a named measurement with a unit and a freshness. Its sources are the
  built-in signals of FR-036-001 to FR-036-004, any platform metric of the catalogue marked eligible (only series without
  tenant or other high-cardinality identity), a custom signal published through the management interface,
  the resource usage of a role, and an **external source** declared by an operator and queried by the platform
  (FR-036-056 to FR-036-061).
- **FR-036-015**: A *scaling policy* binds one or more signals to a **target** (the replicas of a role, a tunable
  parameter, or a resource of a role) and declares, for each signal, a target value; the minimum and maximum
  of the target; the largest step per interval in each direction; separate stabilization windows for scale-out
  and scale-in; a cooldown; the rule that combines several signals (the largest demand wins by default); a
  cost ceiling; and a mode.
- **FR-036-016**: A policy MUST support the two usual interpretations of a signal, each with a documented,
  deterministic formula: *demand* (the target is the amount one unit of capacity is expected to keep up with)
  and *utilisation* (the target is the level at which the current capacity should run).
- **FR-036-017**: A policy MUST have a mode: *observe* (evaluate and record, apply nothing), *recommend*
  (also publish the desired capacity as **advisory**: visible and available, but not marked to be followed) or
  *enforce* (apply what the platform applies itself, and publish the desired capacity of replicas and resources
  as **authoritative**: the value that the environment's autoscaler, an external controller or the optional
  applier follows). A new policy starts in *observe*.
- **FR-036-018**: Signals and policies MUST be definable in the deployment's configuration. A declaration MUST be
  validated before it takes effect; an invalid one is refused with a typed error that names the cause, and the
  last valid version stays in force. Every change creates a version with author, time and reason; versions can
  be listed and a previous one restored.
- **FR-036-019**: The same model, validation and versioning MUST be available through the management
  interface: create, read, update and delete policies and custom signals. Everything that can be declared by
  configuration can be declared programmatically by an operator, and the reverse; what a tenant can do
  programmatically about scaling is limited to FR-036-023. The **management interface** is the
  platform's documented, versioned REST contract, checked by contract tests like the other public contracts,
  together with the operator command line; the official SDKs do not expose it in this feature (O1708).
- **FR-036-020**: A policy MUST have exactly one owner: configuration or the management interface. The interface
  MUST NOT silently edit a configuration-owned policy; it MAY add a **time-boxed override** (hold a value, set
  a temporary minimum or maximum, or freeze), which expires by itself and is audited.
- **FR-036-021**: With no policy declared the platform MUST behave as it does without this feature. Example
  policies for each role ship in *observe* mode. If the policy store cannot be read the platform keeps the last
  valid policies, or none.
- **FR-036-022**: The management interface MUST also provide: reading the current state of every signal and
  policy; a **dry evaluation** (what would be decided for the current or supplied sample values, without
  applying anything); reading the history of decisions; and publishing samples of a custom signal.
- **FR-036-023**: Permissions MUST be separate and scoped by tenant and namespace. For operators: `scaling:read`
  (read signals, sources, policies, versions, decisions and the desired capacity, and run a dry evaluation),
  `scaling:manage` (define, replace and remove policies, sources and custom signals, restore a version, test a
  source), `scaling:override` (overrides, a freeze, the global switch) and `scaling:publish` (publish samples of
  a custom signal); every operation of the operator interface requires exactly one of them. A principal never
  sees or affects another tenant's signals, policies or connectors; operator-only signals are invisible to
  tenants. Shared roles and every scaling policy are managed by operators only. A tenant administrator MAY read
  the lag, the state and the limits that bound the tenant's own connectors (every principal that already holds
  `event:read` for the connector may), and MAY adjust their in-flight parallelism and rate limit by hand within
  the namespace's quotas: the rate by the existing `events_per_second` quota, the parallelism by a
  `connector_parallelism` quota whose default is the platform's cap (a change above a quota is refused with a
  typed error; every change is audited in the tenant's trail). An error returned to a tenant MUST reveal nothing
  operator-only (ceilings, source names, other tenants' limits); it names the class of limit that bounds the
  connector (rate-limited, partition-bound), not the operator's values. A tenant MUST NOT define a scaling policy
  or control replicas or resources (O1703).
- **FR-036-024**: A custom signal MUST be authenticated, rate-limited, size-bounded, declared with a range and a
  time to live, and stale once the time to live passes. In the first delivery only operators publish custom
  signals, and they feed operator policies; publication by tenants is a later addition (it needs a permission and
  a quota, G1700). When it exists, a custom signal published by a tenant MUST have no automatic effect until an
  operator admits it to a policy, in which case its influence is capped; it stays readable by its publisher
  (O1704, following from O1703).
- **FR-036-025**: Every definition change, override, freeze, admission of a custom signal and every decision
  applied in *enforce* mode MUST be audited with actor, reason and before and after. Changes of platform scope
  are recorded in the platform audit trail and changes of tenant scope (a connector's consumption) in the
  tenant's audit trail; every change class belongs to exactly one of them (FR-036-066).

**Decisions and their explanation**

- **FR-036-026**: Evaluation MUST be a deterministic function of the policy version, the samples and the
  current state. Every evaluation that changes capacity, or deliberately declines to, MUST produce a decision
  record with: time; policy version; each signal with value, age and whether it was used; the capacity the
  signals asked for; every limit applied, in order (bounds, step, stabilization, cooldown, connection budget,
  shard or partition ceiling, cost ceiling, freeze or override); the final value; and applied, recommended or
  not applied with the reason.
- **FR-036-027**: A recorded decision MUST be reproducible: submitting its recorded inputs to the dry
  evaluation of the same policy version yields the same decision.
- **FR-036-028**: Decision records MUST be retained for a configurable period (14 days initially) and
  be readable through the management interface and the operator command line. A decision record is operator-only
  data; the part about a tenant's own connector is readable by that tenant. Decision records and the platform
  audit trail MUST contain no payload, secret or credential and no tenant or connector identity (signal values
  are aggregates; a tenant's own view carries only its connector's facts). The platform audit trail is
  append-only for the runtime role and is retained at least as long as the tenant audit trail.
- **FR-036-029**: The platform's scaling metrics (desired and current capacity, decisions by reason, limits hit,
  stale signals) MUST carry no tenant, namespace, connector, run or activity identifier; that detail lives in
  the management interface, the logs and the traces.
- **FR-036-030**: The desired capacity of every role MUST be published in a form that the packaged autoscaling
  integration and external controllers can follow without knowing the policies, as a metric and through the
  management interface, each value marked advisory or authoritative according to the policy's mode.
- **FR-036-031**: With several evaluators running, exactly one decision per policy MUST be in force at a time;
  a second evaluator MUST NOT apply a different decision for the same policy.

**Horizontal scaling by role**

- **FR-036-032**: Each role (ingestion, event dispatch, outbound delivery, scheduler, worker pools, worker-facing
  interface, REST ingress) MUST have a documented **unit of division** (what is shared among replicas), a hard
  ceiling (64 for the scheduler; the partitions of a topic for a Kafka connector; the shards or owners of
  any other role), its signals and its handover cost. The decision clamps to the ceiling.
- **FR-036-033**: Event dispatch and outbound delivery work MUST be divisible among replicas so that adding a
  replica adds capacity until another resource limits it, without an effect happening twice beyond the
  at-least-once guarantee and without starving a tenant (the fairness of feature 010 is kept). The benchmark
  reports the capacity a replica adds; the way the work is divided is decided at planning on that evidence.
- **FR-036-034**: The benchmark MUST establish whether the worker claim path is the limiting step as workers and
  API replicas grow (feature 010, G246). If it is, the claim MUST be divided without losing more of oldest-first
  order than a stated bound; if it is not, the finding is recorded and the path is left alone.
- **FR-036-035**: The allocation of scheduler shards SHOULD take load into account when one replica's shards are
  materially later than the others' for a sustained window (feature 010, G239). If this is not built, the gap
  stays open and the decision record states that allocation is by count.
- **FR-036-036**: The connector readers MUST be runnable as a separate deployment role, selected by
  configuration per deployment, scaled independently of the engine's other roles. The default is all roles
  together. Replicas with different role selections MUST be safe side by side: no connector read twice in
  conflict, none left unread.
- **FR-036-037**: A replica that leaves (scale-in, upgrade) MUST finish or hand back what it holds within a
  bounded time, and nothing is acknowledged to a broker before it is in the inbox. Roles with lease-based
  ownership MUST NOT change membership faster than their handover period permits.

**Vertical scaling**

- **FR-036-038**: Per-replica tunable parameters (concurrency of the passes, batch sizes, in-flight limits of
  connectors, connection budget) MUST each declare a minimum, a maximum and a step, and MUST be possible
  targets of a policy. A change MUST apply to a running replica without a restart, at a point where no
  lease is lost, and the current value and its source (default, configuration, policy, override) MUST be visible.
- **FR-036-039**: The sum of the connection budgets of all replicas of all roles MUST NEVER exceed the
  database's declared capacity less a reserve for administration and migrations. Increases of replicas and of
  per-replica budgets are limited jointly by this invariant, and the budget is divided among roles by a
  declared rule.
- **FR-036-040**: CPU and memory requests and limits of a role MAY be targets of a policy. The platform computes
  and publishes the desired resources and the **change mode**: *rolling* (replace replicas one at a time within
  the disruption limit; the default) or *in place* (only when the operator enabled it for the role and the
  environment confirms support). A change never takes a role below its minimum available replicas (O1702). The change mode is honoured by
  whoever applies the change: the optional applier or an external controller.
- **FR-036-041**: A vertical decision MUST NOT reduce a limit below the observed peak over the policy's window
  plus a stated margin.
- **FR-036-042**: Horizontal and vertical policies on one role MUST NOT oppose each other: at most one kind of
  change per interval and per role, in a documented order of preference.

**Safety**

- **FR-036-043**: A stale or unavailable signal MUST NOT be used. When all signals of a policy are stale or
  unavailable the role MUST hold its current capacity, decide no scale-in, and raise a condition.
- **FR-036-044**: Scale-in MUST be slower than scale-out by default (stabilization and step limits), and the
  floor of a role MUST protect availability (at least two replicas for the control plane and the engine unless
  the operator declares otherwise).
- **FR-036-045**: The platform MUST limit the number of capacity changes per role per window, with hysteresis
  around the target.
- **FR-036-046**: A signal that a tenant can raise MUST be capped in its effect by the cost ceiling and the step
  limit, and the operator-only decision record MUST indicate when one scope dominated the signal. The same
  holds for values pulled from external sources, which are untrusted input: they are bounded by the declared
  range, the step limit and the cost ceiling like any signal.
- **FR-036-047**: Automatic scaling MUST be disableable at once by a global switch and per policy by a freeze,
  both by configuration and through the management interface, effective by the next evaluation and audited.
- **FR-036-048**: The failure of the scaling machinery MUST NOT affect the roles: they keep their last applied
  capacity and parameters and lose no work.

**Evidence**

- **FR-036-049**: A benchmark on separate machines (database, platform, workers) MUST report, for the lag
  control loop and for each role scaled, the environment, the workload, p50/p95/p99, the failures, the limiting
  resource, and whether each load level was reached, in the manner required by feature 010's FR-010-006.
- **FR-036-050**: The failure tests required by the constitution MUST cover: a replica crash and a clean stop
  during a partition handover; a duplicate delivery from the broker; a lost broker acknowledgement; a restart
  of the evaluator in the middle of a decision; two evaluators at once; a telemetry outage; a rolling upgrade
  with mixed versions; and a tenant flooding a shared signal.

**Applying decisions** *(added by the clarification of 2026-10-28)*

- **FR-036-051**: The configuration MUST declare, for each role, who controls its replicas and resources:
  *external* (the environment's autoscaler or an outside controller; the default, and the behaviour before this
  feature) or the *applier*. A role has exactly one controller. The platform MUST refuse to enable the applier for
  a role declared external. When the replicas or resources of an applier-controlled role are changed by another
  writer, the applier MUST raise a conflict condition and stop applying to that role until an operator resolves
  it.
- **FR-036-052**: The applier MUST be an optional component, separate from the control plane and the engine,
  off by default and absent from a deployment that does not enable it. Without it the platform changes nothing in
  the orchestrator.
- **FR-036-053**: The applier MUST hold only the permissions to change the replica count and the resources of the
  platform's own workloads in its own installation. It MUST NOT be able to read secrets or tenant data, or to
  change any other workload, and its credentials MUST NOT be shared with another role.
- **FR-036-054**: The applier MUST act only on the latest authoritative decision of a policy in *enforce* mode
  (a superseded decision is dropped), within the ceilings, the database connection budget, the change mode and
  the disruption limit. Each decision MUST be applied at most once, so that a retry is harmless. The applier MUST
  compare the observed result with the desired one and report both. When the orchestrator cannot be reached or
  refuses a change, it MUST hold, report the condition and decide nothing on its own.
- **FR-036-055**: Every action of the applier MUST be audited with the decision that caused it, the state
  before and after, and the outcome. The global switch and a freeze (FR-036-047) MUST stop it before its next
  change.

**Pulled signal sources** *(added by the clarification of 2026-10-28)*

- **FR-036-056**: Operators MUST be able to declare, by configuration and through the management interface,
  **external signal sources** that the platform queries by itself. The first kinds are: a query to an external
  metrics system, and the depth (and, where the broker can state it, the oldest age) of a declared broker queue,
  topic or stream, whether or not a connector reads it. A source has a name, a kind, a destination, an optional
  credentials reference, the query or the queue, a unit, a sampling interval, a freshness limit and a declared
  valid range. The kinds are an open catalogue: a new kind is added without changing the policy format.
  Declaring, changing and removing a source is audited.
- **FR-036-057**: Only operators MUST be able to declare an external source; a tenant publishes a custom signal
  instead. The destination of a source MUST match an entry of an operator-declared allowlist: an exact host and
  port, with no wildcard. A destination that is an address literal is accepted only if the literal itself is
  listed; for a name, every address it resolves to is checked and one forbidden address refuses the connection,
  so that a name cannot be rebound to an address that was not intended. Redirects are never followed and
  proxies from the environment are ignored. Where the deployment restricts its network the destination MUST
  also be allowed there explicitly. A source whose destination is not allowed is refused at declaration, and the
  check is repeated at every query.
- **FR-036-058**: The credentials of a source MUST be references to secrets, resolved at use, and MUST NOT
  appear in a source declaration, a policy, a decision record, a log, a metric or a response. A destination with
  embedded user information, or a destination query string or query text with a parameter named like a
  credential (`token`, `password`, `secret`, `authorization`, `api_key` and the like), is refused at
  declaration; the platform does not promise to recognise a credential in free text (G1718). A source with
  credentials MUST use an encrypted connection (TLS 1.2 or later, the certificate chain and the host name
  validated against the system trust roots and any roots the operator names in configuration) and is refused
  otherwise; a certificate error is a refusal, never a warning.
- **FR-036-059**: Querying a source MUST be bounded. It is read-only: the platform builds the request itself and
  uses only a read operation of the source's protocol (the operator supplies only the query text or the queue,
  topic or stream name), so no query can change the source. The defaults are a timeout of 5 s, a response of at
  most 64 KiB and a minimum interval of 5 s, configurable up to 30 s, 1 MiB and one hour; a query that is
  expensive for the source is bounded by those limits and by the minimum interval, and a source that rejects
  the query text is a failure of that source. The result MUST reduce to one number per signal. A result that is
  not a finite number, or is outside the declared valid range, MUST make the signal *unavailable* and MUST NOT
  be clamped or read as zero. At most one query per source per interval MUST be made however many replicas run,
  and a failing source is retried with backoff.
- **FR-036-060**: An operator MUST be able to test a source before relying on it, through the management
  interface and the operator command line: the test returns the value, the unit, the latency and the failure
  class, and has no effect on any policy or decision. The health of every source (last success, last failure
  class, state) MUST be readable.
- **FR-036-061**: Signals of every origin (built-in, catalogued, pulled or custom) that an operator marks
  *exported* MUST be available to autoscaling providers as metric series (name, unit, value, state, sample time)
  without tenant, namespace or connector identity, in addition to the desired capacity of FR-036-030, so that an
  autoscaling provider such as KEDA can scale on the signals directly or follow the platform's decision. An
  exported signal is operator data, offered to autoscaling providers and operators only and never to tenants; a
  source that reflects a single tenant's data is declared on the operator's responsibility.

**Operator authority, audit and limits** *(added by the review of 2026-10-28, checklist `security.md`)*

- **FR-036-062**: Every call of the operator interface MUST carry a bearer token whose audience is reserved to
  platform operators and which identifies the person or system acting. A token issued for tenants MUST be
  refused by the operator interface, and a token issued for operators MUST be refused by the tenant interface.
  Tokens accepted by the operator interface are short-lived (a lifetime above one hour is refused); access is
  revoked by expiry and by rotating the verification keys of the operator audience; and every operator action
  records the token's subject and identifier, so that the actions of a leaked token can be listed.
- **FR-036-063**: The operator interface MUST be absent unless enabled by configuration. When enabled it MUST be
  reachable only from peers named explicitly in the deployment; an empty or unrestricted peer list is refused
  when the deployment is rendered or started.
- **FR-036-064**: The hard ceilings of each unit, the database connection budget, the destination allowlist and
  the names of worker pools MUST be changeable only by configuration and never through the management
  interface. The evaluator MUST validate every policy, source and signal version against them at each
  evaluation, and MUST refuse, and raise a condition for, one that violates them, so that rows written outside
  the management interface cannot take a unit past its ceilings or a source to a destination that is not allowed.
- **FR-036-065**: Refused attempts on the operator interface, on a source declaration and on a cross-tenant read
  MUST be audited, without the content of the request, and counted by reason on a metric that carries no tenant
  identity.
- **FR-036-066**: The actor of an audit record MUST be: the operator identity of the token, for a change through
  the interface; `configuration` with the digest of the document, for a change that comes from the
  configuration file (each applied version is recorded with a reason, the document's declared change note or its
  digest); and the evaluator's identity, for a decision applied automatically in *enforce* mode. Every record
  carries the state before and after and a reason.
- **FR-036-067**: A credential is rotated by replacing its file; the new value is used from the next query, with
  no restart. A source whose credential file is missing or unreadable becomes *unavailable* with the reason
  `credential_unavailable`, is reported in its health and raises a condition, and never falls back to an
  unauthenticated query. Adding or removing a credential on a source is a new version of the source.
- **FR-036-068**: The operator interface MUST be rate limited per token (20 requests per second initially,
  configurable) with a typed refusal. The tenant endpoints for lag and consumption fall under the namespace's
  existing request-rate quota.

### Non-functional requirements

- **NFR-036-SEC**: Tenant boundary, least privilege and audit on every scaling operation; no secret, payload
  or credential in a signal, a decision record, a log or a metric; custom-signal abuse is bounded (FR-036-024).
- **NFR-036-DUR**: A scaling action, including the loss of a replica in the middle of one, MUST NOT lose or
  duplicate a transition, an event or a delivery beyond at-least-once; leases and fences are unchanged.
- **NFR-036-OBS**: Traces for evaluations and handovers; metrics without high-cardinality identifiers
  (ADR-0017); structured logs without sensitive data; typed errors with code, retryable and trace id.
- **NFR-036-COMP**: The change is additive. A deployment with no new configuration behaves as before; existing
  metrics, alerts and settings keep their meaning; versions mixed during a rolling upgrade are safe; wire
  contracts are versioned and compatible.
- **NFR-036-PERF**: The cost of measuring and evaluating (database reads, samples, decision records) is
  bounded and reported; targets are benchmark criteria on a stated environment, not promises.
- **NFR-036-OPS**: Runbooks and alerts exist for: signals stale, policy held at its ceiling, partition-bound,
  database-budget-bound, evaluator failing, and a scaling freeze left on.

### Key Entities

- **Scaling signal**: a named measurement with unit, scope, freshness and state (available, stale,
  unavailable); built-in, catalogue, resource or custom.
- **Signal sample**: one value of a signal at a time, with its state.
- **Scaling policy** and **policy version**: signals bound to a target with targets, bounds, steps,
  windows, cooldown, combination rule, cost ceiling and mode; owned by configuration or the interface.
- **Scaling target**: the replicas of a role, a tunable parameter of a role, or a resource of a role.
- **Role**: a unit of deployment with a unit of division, a ceiling, signals and a handover cost.
- **Connector lag**: backlog, oldest age, sample time, state of draining (behind, blocked, paused, failing).
- **Partition assignment**: which replica reads which partition of which connector, with the lease and the
  position; and the partition-bound ceiling.
- **Scaling decision record**: inputs, raw desire, ordered limits, final value, outcome.
- **Override and freeze**: time-boxed instructions with author, reason and expiry.
- **Capacity ceiling**: the most a role can usefully have (shards, partitions, database connection budget,
  cost ceiling).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-036-001**: For Kafka, NATS and SQS, the reported backlog equals the broker's own count, exactly on a
  quiescent test and within the lag tolerance under load, in 100% of the conformance scenarios; an
  unreachable broker is reported as unavailable in 100%, never as zero.
- **SC-036-002**: With 12 partitions and replicas going from 1 to 3 and back, the number held by any two
  replicas differs by at most one within the rebalance time, in 100% of the runs.
- **SC-036-003**: Across fault injection (replica crash, clean stop, scale-in, broker redelivery) every message
  sent appears in the inbox exactly once and no position moves backwards, in 100% of the runs.
- **SC-036-004**: With a policy on lag and a producer above one replica's capacity, the benchmark reports the
  time for the lag to fall under target for Kafka, NATS and SQS, with the environment, and records the replica
  count reached.
- **SC-036-005**: The same policy declared by configuration and through the management interface gives
  identical decisions for identical samples in 100% of the parity tests.
- **SC-036-006**: 100% of decisions have a record with every field of FR-036-026, and 100% of recorded
  decisions are reproduced by the dry evaluation of their inputs.
- **SC-036-007**: Over a randomized and adversarial corpus of samples (stale, missing, extreme, oscillating,
  tenant-raised) the decision function never takes a role above its ceiling, below its floor, over the database
  connection budget, or past its step limit, and never decides on a stale signal.
- **SC-036-008**: When the telemetry source is unavailable for longer than the freshness limit, 100% of roles
  hold their capacity, and a condition is raised within one evaluation interval.
- **SC-036-009**: A deployment with no new configuration passes the existing suites of features 004 and 010
  unchanged, and its metrics and alerts keep their names and meaning.
- **SC-036-010**: For the event dispatcher and the delivery worker, the report states the capacity added by each
  replica from 1 to 8 on separate machines, and whether the database was the limit.
- **SC-036-011**: A change of a per-replica tunable by policy takes effect on a running replica within the
  time for a tunable change, without a restart and with no lease lost, in 100% of the runs.
- **SC-036-012**: A mixed deployment (combined, ingestion-only and engine-only replicas) consumes a topic with
  no loss and no duplicate admitted, and balanced partitions, in 100% of the runs.
- **SC-036-013**: Every cross-tenant attempt on a signal, policy, connector lag or decision record fails as a
  missing resource and is audited, in 100% of the security tests. "As a missing resource" means the same status
  code, body and headers (the trace identifier excepted) as a resource that does not exist, and no difference
  in response time beyond the variation measured over a thousand requests.
- **SC-036-014**: The benchmark report on separate machines states, for every load level of feature 010
  (100, 1,000 and 5,000 transitions per second), reached or not reached, and the limiting resource.
- **SC-036-015**: With the applier absent, the platform changes nothing in the orchestrator in 100% of the runs.
  With it enabled for a role, the observed replicas and resources reach the authoritative desired values, within
  the evaluation interval plus the orchestrator's own rollout time (reported), without passing a ceiling, the
  database connection budget or the disruption limit, and with an audit record for every action, in 100% of the
  runs.
- **SC-036-016**: Every attempt to use the applier's credentials on a secret, on tenant data or on a workload
  that is not the platform's own is denied in 100% of the security tests (delivered with the applier, in the
  third delivery, not the first).
- **SC-036-017**: For each supported source kind, a declared source delivers its value as an available signal
  within two sampling intervals, and a source that is unreachable, slow, malformed or out of range is reported
  unavailable and never as zero, in 100% of the runs.
- **SC-036-018**: In 100% of the security tests a source is refused when declared by a tenant, with a
  destination off the allowlist or not allowed by the network policy (including a name that resolves to an
  address that was not allowed, an address literal in IPv4, IPv6 or an alternative notation, and a redirect),
  with embedded user information or a credential-named parameter, with credentials over an unencrypted
  connection, with a response over the size limit or slower than the timeout, or with a result that does not
  reduce to one number; and no credential appears in any record, log, metric or response.
- **SC-036-019**: In an end-to-end test on a real cluster, a reference autoscaling provider scales a role out
  and in following an exported signal, and following the authoritative desired capacity, with the environment
  and the failures reported.
- **SC-036-020**: A token for tenants is refused by the operator interface, a token for operators is refused by
  the tenant interface, a token with a lifetime above one hour is refused, and every refused attempt is audited
  and counted by reason, in 100% of the security tests.
- **SC-036-021**: A policy, source or signal version written outside the management interface that violates a
  ceiling, the database budget or the allowlist is never used by the evaluator, and a condition is raised, in
  100% of the tests.

## Assumptions

- The platform decides and publishes. The environment that runs the replicas applies replica counts and
  resources: by default its own autoscaler or an external controller following the published desired capacity
  (for the supported packaging the autoscaling integration is extended to follow it), or, where the operator
  enables it, the optional applier (O1706).
- CPU and memory of a role are collected by the replicas themselves (built-in signals), so a pulled source is
  needed only for what the platform cannot see by itself. The first query dialect for an external metrics
  system is chosen at planning; a Prometheus-compatible interface is expected, because the packaged
  autoscaling already reads from one.
- The SDK client today covers run operations only (start, get, list, cancel, signal, query, update, chains,
  publishEvent, waitForCompletion) and no platform administration, which is why the management interface does
  not enter the SDKs (O1708).
- Parameters inside a running replica and the assignment of partitions are applied by the platform itself.
- Kafka positions stay in the platform's database, in the same commit as the admission of the events
  (ADR-0002); the platform does not adopt broker-managed consumer groups (O1701, recommended default).
- Initial parameter values, which the criteria of this specification are written against. Each is
  configurable; planning confirms or changes it by measurement and records any change as a decision (D17xx):
  - sampling interval of a signal 10 s; freshness limit three sampling intervals (30 s); evaluation interval 15 s;
  - **rebalance time** after a replica joins or leaves cleanly: at most twice the partition lease (60 s at
    today's 30 s lease); after a crash: the lease plus one round (FR-036-009);
  - **lag tolerance**: exact when the connector is quiescent; under load, within the messages that arrive in
    two sampling intervals; for SQS, within the broker's own stated approximation;
  - **time for a tunable change** to reach a running replica: two evaluation intervals (30 s);
  - retention of decision records 14 days;
  - scale-in stabilization 5 minutes and at most one replica per 2 minutes, as the packaged engine
    autoscaling does today; scale-out has no window by default and is limited by its step.
- Operators own every scaling policy. Tenants see the lag and the decisions about their own connectors and
  can tune their own connectors' consumption parameters by hand within quota (O1703, answered). Lag is
  readable by every principal that already holds `event:read` for the connector (the roles that can already
  read its health); it shows counts and ages, never a payload.
- The identity providers, the development provider included, can issue tokens with a separate audience for
  operators and a short lifetime. The operator interface refuses to start with a provider that cannot; the
  support of each provider is validated by a task before the interface is enabled (O1710).
- A single region with high availability, as in feature 010.
- Targets in this document are benchmark criteria on a stated environment, not promises of capacity.

## Out of scope

- Multi-region and active-active deployment (feature 032).
- Replacing the environment's own autoscaler or managing cluster nodes: the platform exposes signals and
  decisions for them to consume. The optional applier is an alternative for operators who want a closed loop,
  limited to the replicas and resources of the platform's own workloads.
- External signal sources declared by tenants: a tenant publishes custom signals instead (FR-036-024).
- Operations on signals and policies in the six official SDKs (O1708). A client can be generated from the
  versioned contract; publishing custom signals from the SDKs is the natural follow-up if a tenant needs it.
- Predictive or learned scaling; scaling on cost or billing data.
- Exactly-once effects on external systems.
- Read replicas and partitioning or archival of the journal (feature 010, G241 and G242; feature 020 covers
  archival): named as future levers, and a limit of this feature's results.
- A Studio view of signals, policies and decisions: the management interface and the operator command line
  come first; the view is a follow-up.
- Autoscaling of push functions and warm pools of feature 019 until that feature is approved.
- Changing the delivery guarantees of any connector, or adopting broker-managed consumer groups.

## Owner questions

Recorded with the recommended default in `research.md` at planning. The owner decides; until then the default
below is what the specification assumes.

- **O1700 — Dedicated ingestion role.** Recommended: yes, as an optional deployment role; the default stays
  "everything together".
- **O1701 — Kafka consumption.** Recommended: keep positions in the platform's database with a balanced
  assignment of partitions, and do not adopt broker consumer groups, so that the position and the admission
  of the events stay one commit.
- **O1702 — Default way to apply a resource change.** Recommended: rolling replacement by default; in place
  only when the operator enables it for the role and the environment confirms support.
- **O1703 — Who may define what.** *Answered by the owner on 2026-10-28 (option B):* operators define every
  policy; tenants read the lag and decisions of their own connectors and adjust their in-flight parallelism and
  rate limit by hand within quota; no tenant controls replicas or resources. Recorded in
  `docs/owner-decisions.md`; specified by FR-036-023.
- **O1704 — Custom signals.** Recommended, and following from O1703: a custom signal published by a tenant has
  no automatic effect until an operator admits it to a policy, with a cap on its influence. Pending the
  owner's confirmation.
- **O1705 — Rate limit of a connector.** *Resolved by a finding at planning, no decision needed:* the limit
  already holds across replicas, so there is nothing to opt into. The question rested on a wrong statement in
  the runbook. Specified by FR-036-013 (visibility and documentation only).
- **O1706 — Who applies replicas and resources.** *Answered by the owner on 2026-10-28:* by default the
  environment's autoscaler or an external controller follows the published desired capacity; in addition an
  optional applier (separate, off by default, minimal permissions) carries out enforced decisions for operators
  who want a closed loop. Recorded in `docs/owner-decisions.md`; specified by FR-036-051 to FR-036-055 and US12.
- **O1707 — Where outside data comes from.** *Answered by the owner on 2026-10-28:* the platform queries
  external sources declared by configuration (queued messages, CPU, memory and similar measures) and hands the
  signals and the decisions to the autoscaling provider (KEDA or another). Declaring a source is an operator
  action. Recorded in `docs/owner-decisions.md`; specified by FR-036-056 to FR-036-061 and US13.
- **O1708 — Programmatic access and the SDKs.** *Answered by the owner on 2026-10-28:* the management
  interface is the documented REST contract and the operator command line; the six official SDKs do not change
  in this feature. Recorded in `docs/owner-decisions.md`; specified by FR-036-019.
- **O1709 — Delivery order.** *Answered by the owner on 2026-10-28 (option B):* the first delivery is US01,
  US02, US03, US04, US05, US06, US10 and US13; US07, US09, US08, US12 and US11 follow. Recorded in
  `docs/owner-decisions.md`; described in the Context.
- **O1710 — How an operator acts through the management interface.** *Raised by planning, not yet answered.*
  The REST API knows only tenant principals, and today's operator actions are owner-role commands. Options: a
  separate operator listener and principal (recommended, assumed by the plan and ADR-0074); an operator role on
  the tenant API; owner-role commands only with a read-only REST. See `research.md` O1710. If the owner chooses
  the role on the tenant API: FR-036-062 and FR-036-063 change (one listener), the identity pipeline of feature
  009 changes, and the management milestone grows. If the owner chooses owner-role commands only: FR-036-019
  and US05 narrow to the tenant endpoints and a read-only operator API, and FR-036-062, FR-036-063 and
  FR-036-068 apply to that read-only interface.

## Known limits and future levers

The database remains the ceiling of the platform (every transition is a commit); more replicas move work
between processes and do not make the server's commits cheaper. A single large run is one shard's work
(feature 010, G254). Read replicas, journal partitioning and archival are the next levers and are not part of
this feature.
