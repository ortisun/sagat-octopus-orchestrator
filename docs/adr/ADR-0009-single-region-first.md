# ADR-0009 — Single region with HA before active-active

Status: **Accepted for initial design** | Date: 2026-10-07

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
