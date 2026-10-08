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
