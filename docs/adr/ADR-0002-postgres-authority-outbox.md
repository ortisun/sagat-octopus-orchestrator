# ADR-0002 — PostgreSQL as source of truth

Status: **Accepted for initial design** | Date: 2026-10-07

## Context
Sagat Octopus Orchestrator requires an evolvable, durable and operationally explainable architecture.

## Decision
PostgreSQL atomically persists journal, logical queues and outbox; NATS only distributes references.

## Why
Reduces the window of loss between state and event emission.

## Trade-offs
The DB can become a bottleneck; partition and measure before migrating the journal.

## Review criteria
Open a new ADR with benchmark data, failure tests, operational cost and compatibility with existing runs. Do not change the semantics silently.
