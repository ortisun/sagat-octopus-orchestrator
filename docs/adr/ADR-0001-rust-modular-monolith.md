# ADR-0001 — Rust and modular monolith

Status: **Accepted for initial design** | Date: 2026-10-07

## Context
Sagat Octopus Orchestrator requires an evolvable, durable and operationally explainable architecture.

## Decision
Rust/Tokio with crates per context and processes that can be split out only after measurement.

## Why
Speeds up a unified domain, avoids premature microservices.

## Trade-offs
Higher learning-curve cost; mitigate with standards, lints and ADRs.

## Review criteria
Open a new ADR with benchmark data, failure tests, operational cost and compatibility with existing runs. Do not change the semantics silently.
