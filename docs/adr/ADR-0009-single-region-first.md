# ADR-0009 — Single region with HA before active-active

Status: **Accepted for initial design**; **superseded in part by [ADR-0067](ADR-0067-a-cell-has-one-writable-region-and-fails-over-by-a-fenced-epoch.md)** (2026-11-24): a cell may declare standby regions and fail over to one by a fenced epoch, with one writable region at a time. In-region high availability (now specified by [ADR-0130](ADR-0130-postgresql-high-availability-is-a-reference-topology-with-synchronous-quorum-and-one-moving-address.md)) and the deferral of active-active stand. | Date: 2026-10-07

## Context
Sagat Octopus Orchestrator requires an evolvable, durable and operationally explainable architecture.

## Decision
V1 HA with PostgreSQL HA, stateless replicas and coordinator leases; multi-region active-active deferred.

## Why
Avoids complex consensus/global-order prematurely.

## Trade-offs
RTO/RPO depend on the configured backup/replication; test it.

## Review criteria
Open a new ADR with benchmark data, failure tests, operational cost and compatibility with existing runs. Do not change the semantics silently.

## Note (2026-11-24, constitution 5.0.0)

The constitution (version 5.0.0, Delivery Constraints, Owner decision 2026-11-24-2) now admits one writable region per
cell with standby regions and fenced active-passive failover (feature 032). This record stays accepted until
[ADR-0067](ADR-0067-a-cell-has-one-writable-region-and-fails-over-by-a-fenced-epoch.md) is accepted on its evidence
(the region drill report and the commit-latency benchmark), which supersedes it in part; in-region high availability
and the deferral of active-active inside a cell stand.
