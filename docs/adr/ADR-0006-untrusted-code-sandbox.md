# ADR-0006 — Scripts as untrusted execution

Status: **Accepted for initial design** | Date: 2026-10-07

## Context
Sagat Octopus Orchestrator requires an evolvable, durable and operationally explainable architecture.

## Decision
Compile in build workers and run in isolated containers/pools, with quota and egress policy.

## Why
Risk containment and exemption from app deploy.

## Trade-offs
Cold start and build overhead; use verified caches.

## Review criteria
Open a new ADR with benchmark data, failure tests, operational cost and compatibility with existing runs. Do not change the semantics silently.
