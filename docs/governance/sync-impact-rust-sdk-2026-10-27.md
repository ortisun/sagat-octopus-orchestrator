# Constitution Sync Impact Report — Rust SDK

Amendment carried by the feature034 delivery PR, following the owner's request on2026-10-27.
Version1.0.0 →1.1.0 (MINOR): compatibility guidance expands the official SDK language enumeration
from five to six by adding Rust. No durable execution, retry, lease, tenant, secret, broker or
wire-compatibility principle is removed or weakened.

Affected guidance: constitution principleIV, root and sdks AGENTS.md, normative sdks/SPEC.md,
feature006 dependency notes, ADR-0070, and feature034 spec/plan/tasks. Historical five-language
acceptance and released0.2 artifacts are preserved; new six-language acceptance requires actual
Rust conformance/fault/package/consumer evidence. Package/CI/matrix documentation remains T034-007.

Fitness: canonical protocol/catalog drift, common conformance runner and standalone package checks
must cover Rust. No backend crate or database/broker runtime becomes a dependency of the public SDK.
The PR is not release acceptance; all actual gates and the remaining0.3 dependencies still apply.
