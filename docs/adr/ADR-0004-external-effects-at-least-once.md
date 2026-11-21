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

## Proposed refinement — 2026-11-01

[ADR-0076](ADR-0076-compensation-failure-and-uncertain-effects.md) records owner-approved direction for compensation review, truthful terminal
failure and qualified uncertain-effect recovery. It was accepted on 2026-11-01 for delivery 1 (qualification stays proposed);
this record is not fully superseded and its historical rationale remains unchanged.
