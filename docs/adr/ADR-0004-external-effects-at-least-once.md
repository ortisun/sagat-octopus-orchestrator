# ADR-0004 — At-least-once semantics for external effects

Status: **Accepted for initial design** | Date: 2026-10-07

## Context
Sagat Octopus Orchestrator requires an evolvable, durable and operationally explainable architecture.

## Decision
Physical attempts may repeat; idempotency and reconciliation per connector.

## Why
Realistic in the presence of failure between the external effect and the commit.

## Trade-offs
Providers without idempotency require manual review or confirmation protocols.

## Review criteria
Open a new ADR with benchmark data, failure tests, operational cost and compatibility with existing runs. Do not change the semantics silently.
