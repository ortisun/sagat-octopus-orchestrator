# ADR-0007 — Common SDK protocol

Status: **Accepted for initial design** | Date: 2026-10-07

## Context
Sagat Octopus Orchestrator requires an evolvable, durable and operationally explainable architecture.

## Decision
gRPC/Protobuf and OpenAPI with conformance across TypeScript, Go, Python, C# and Java.

## Why
Avoids semantic forks and coupling to the Rust runtime.

## Trade-offs
Requires a permanent compatibility suite.

## Review criteria
Open a new ADR with benchmark data, failure tests, operational cost and compatibility with existing runs. Do not change the semantics silently.
