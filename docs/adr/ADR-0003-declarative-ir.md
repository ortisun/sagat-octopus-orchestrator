# ADR-0003 — Declarative IR as visual and execution contract

Status: **Accepted for initial design** | Date: 2026-10-07

## Context
Sagat Octopus Orchestrator requires an evolvable, durable and operationally explainable architecture.

## Decision
Graph-native as V1. SDK compilers to IR and a procedural worker with determinism rules later.

## Why
Avoids promising round-trip of arbitrary imperative code to the canvas.

## Trade-offs
Code-first DSL will have explicit limits in V1.

## Review criteria
Open a new ADR with benchmark data, failure tests, operational cost and compatibility with existing runs. Do not change the semantics silently.

## Note (2026-11-24, constitution 5.0.0)

The procedural worker this record deferred ("later") enters release 0.3.0 (Owner decision 2026-11-24-3), and
constitution principle IV (version 5.0.0) now states its obligations: the workflow function runs in an SDK worker, is
replayed against the journal, proposes commands accepted under a lease and a fencing token, and ships in an SDK only
after a determinism study and the shared replay conformance pass. The graph model of this record stands beside it.
Design 044 proposes the records that refine this one (ADR-0103 to ADR-0109, starting with ADR-0103: a code workflow
is a decider whose commands are IR nodes committed in the run transaction), with the evidence the review criteria
above ask for.
