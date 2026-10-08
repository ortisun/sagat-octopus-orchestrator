# ADR-0008 — Runtime graph as a journal projection

Status: **Accepted for initial design** | Date: 2026-10-07

## Context
Sagat Octopus Orchestrator requires an evolvable, durable and operationally explainable architecture.

## Decision
Studio derives nodes/attempts/edges from the persisted history, not from ephemeral telemetry.

## Why
Exact debugging after failures and re-executions.

## Trade-offs
Eventually consistent reads and projections, with the lag status displayed.

## Review criteria
Open a new ADR with benchmark data, failure tests, operational cost and compatibility with existing runs. Do not change the semantics silently.
