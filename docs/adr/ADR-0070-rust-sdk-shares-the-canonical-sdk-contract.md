# ADR-0070 — Rust shares the canonical SDK contract

Status: accepted for implementation after owner request2026-10-27; feature034, release0.3.
Extends ADR-0007/0015/0029 and current feature014/016/017 wire contracts.

Add Rust as the sixth official SDK, with async REST client, Tokio worker host and declarative
workflow builders. Generate/check the same canonical gRPC protocol and closed error catalog.
Run the same authoring/runtime/fault conformance, without changing server semantics or giving
native Rust workers privileged access to PostgreSQL, broker messages or secret authorities.

Ship a standalone MIT crate that a clean Rust project can install, with packaged wire artifacts,
safe TLS/redirect/token handling, bounded retries/concurrency and safe trace/metric hooks.
The server continues to own state transitions, retries, quotas and capability authorization.

Amend the constitution's language enumeration through the delivery PR with a Sync Impact Report.
Historical five-language acceptance remains historical; Rust/six-language parity requires new
named evidence. A Rust crate scaffold alone is not SDK completion or release acceptance.
