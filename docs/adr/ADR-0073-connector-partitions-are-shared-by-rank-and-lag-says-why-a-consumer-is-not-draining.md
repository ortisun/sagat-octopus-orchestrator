# ADR-0073: Connector partitions are shared by rank among the replicas that read them, and a connector's lag says why it is not draining

**Status:** Proposed (2026-10-28)
**Decided by:** agent proposal for the owner's review; the owner asked for lag-based consumer scaling and confirmed the Kafka direction as a recommended default (O1701, not yet confirmed as a decision)
**Refines / supersedes:** refines ADR-0012 (events pass through a durable inbox) and ADR-0002; extends the lease pattern of ADR-0020 with an exact rank-based share; supersedes nothing
**Enforced by:** `integration_elastic_scaling` assignment tests and `fault_elastic_scaling` handover tests on a real Kafka broker, the property test of `partition_targets`, a conformance test that every inbound connector kind implements `LagProbe`

## Context

A Kafka connector's positions live in PostgreSQL, in the same commit as the admission of the events read (ADR-0002, ADR-0012). Replicas share the partitions by leasing each one; the holder renews for as long as it lives. A replica that joins later receives none, and one replica can hold all. Adding a replica adds no read capacity until a lease expires.

No connector reports its lag: Kafka's fetch returns the high watermark and the code discards it; NATS and SQS have broker figures nobody reads. And a connector's `ratePerSecond` is not per replica, as the runbook says: it is a shared token bucket in the database (capacity twice the rate) under the namespace's events-per-second bucket, so a connector at its rate limit cannot be drained faster by more readers, and `Admission::RateLimited` is folded into `Backlogged` with its retry time dropped.

ADR-0020's share is `ceil(members ÷ …)`. For 12 partitions and 5 members it gives 3, 3, 3, 3, 0: the last joiner gets none. The specification (FR-036-007) requires every replica within one of the others.

## Decision

1. **Partitions are divided by rank.** Replicas that read a connector announce themselves in `connector_members`. For P partitions and M live members sorted by identity, the first `P mod M` members have the target `⌊P/M⌋ + 1` and the rest `⌊P/M⌋`. A holder MUST give back its highest partitions beyond its target and MAY take the lowest free ones up to it. More members than partitions leaves the surplus at zero, and the connector reports it is partition-bound.
2. **Positions stay in PostgreSQL, one commit with the admission.** The platform MUST NOT hand partition ownership or offsets to the broker (no consumer groups). The move of a position stays fenced by the stored value, so a handover cannot lose a message or admit one twice beyond what the inbox deduplicates. A reader is started for a partition only while it is assigned, and a clean stop releases its leases.
3. **Lag is measured where the data is,** one row per (connector, holder) with the partitions it covers: Kafka from the high watermark that the fetch returns and the timestamp of the first unread record; NATS from the consumer's pending and ack-pending counts; SQS from the queue attributes (no age). An unmeasurable figure is `null`, never 0. A signal built from rows that do not cover every partition has coverage below 1 and never lowers capacity.
4. **The connector says why it is not draining:** `CAUGHT_UP`, `BEHIND`, `BLOCKED` (the tenant's undispatched-event limit), `RATE_LIMITED` (with the retry time), `PAUSED`, `FAILING`. The rate-limited outcome MUST be kept distinct from backpressure and its retry time used. Lag in `BLOCKED` or `RATE_LIMITED` is excluded from the default ingestion signals, because more readers cannot drain it.
5. **The rate limit keeps its meaning:** it holds across replicas through the shared bucket. The documentation that says "per replica" MUST be corrected in the same delivery.
6. **Parallelism inside a reader changes by a channel, not a restart,** with a platform cap and a default of 1 (today's serial reading).

## Considered options

1. **Rank-based exact share, positions in PostgreSQL** — meets the specification's balance, keeps the atomic commit, needs only a membership table and a keeper round.
2. **Kafka consumer groups** — the broker rebalances for free, but the offset commit becomes a second authority, the atomicity with the inbox is lost (a crash between the commit and the admission loses or repeats), and positions would live in two places.
3. **ADR-0020's `ceil` share** — already built for shards, and cannot give the floor and ceil split (3, 3, 3, 3, 0 for 12 over 5).
4. **Keep first-come leases and only measure lag** — scaling on lag would add replicas that stay idle.
5. **Static assignment by configuration** — no rebalancing when a replica dies; the operator becomes the keeper.

## Consequences

- Good: a joined replica reads its share within the rebalance time; lag becomes a signal; the platform stops scaling out for lag that cannot be drained, and says why.
- Bad: a keeper round and a membership table per connector reader; a rolling upgrade is unbalanced until the old engines are gone (an old engine still takes free leases greedily; the fences hold). Last-writer drain state can hide one failing reader among healthy ones (`G1709`).
- Harder: SQS lag is approximate and needs a new permission on the queue (`GetQueueAttributes`); the useful readers for a mix of connector kinds are bounded approximately (`G1701`).
- Compatibility: additive tables and columns; the connector API and its guarantees are unchanged; a redelivery is still answered `DUPLICATE` by the inbox.

## Evidence

Specification FR-036-001 to FR-036-013; `research.md` D1707 to D1712, D1731 and the evidence table; `contracts/evaluation.md` (partition targets); `crates/infra-connectors/src/kafka.rs:162-183, 226, 276`, `crates/infra-postgres/src/content_usage.rs:291-345`. To be added when built: the 12-partition run, the crash-in-a-batch run, the real-broker lag figures.
