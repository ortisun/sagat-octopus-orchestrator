# ADR-0005 — Native and auditable compensation

Status: **Accepted for initial design** | Date: 2026-10-07

## Context
Sagat Octopus Orchestrator requires an evolvable, durable and operationally explainable architecture.

## Decision
CompensationPlan persisted per Saga with reverse topo, retry and manual review.

## Why
Facilitates long transactions and domain orchestration.

## Trade-offs
It is not a real rollback of effects already settled.

## Review criteria
Open a new ADR with benchmark data, failure tests, operational cost and compatibility with existing runs. Do not change the semantics silently.

## Proposed refinement — 2026-11-01

[ADR-0076](ADR-0076-compensation-failure-and-uncertain-effects.md) records owner-approved direction for compensation review, truthful terminal
failure and qualified uncertain-effect recovery. It was accepted on 2026-11-01 for delivery 1 (qualification stays proposed);
this record is not fully superseded and its historical rationale remains unchanged.
