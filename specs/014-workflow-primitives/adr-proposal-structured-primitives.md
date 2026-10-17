# Structured durable instances and explicit continuation chains

**Status**: Architecture accepted on 2026-10-14; implementation and release evidence pending.

The proposal is accepted as two independently scoped permanent decisions:

- [ADR-0030 — Structured instances and continuation chains](../../docs/adr/ADR-0030-structured-instances-and-continuation-chains.md)
  accepts acyclic bodies, typed scoped identity, one reducer/store, dependency-based compensation,
  approval role grants, worker 1.5, bounded evaluator/instance budgets and parent-child chain locking.
- [ADR-0031 — Declared operation result disclosure](../../docs/adr/ADR-0031-declared-operation-result-disclosure.md)
  separately refines ADR-0028 for explicit named Query/Update business results, with closed bounded
  schemas, current namespace operation authorization, successful audit and authorized receipt replay.
  General structural payload inspection and ordinary payload-free projections keep their policy.

The accepted product defaults and repository evidence are in [research.md](./research.md). The
[data model](./data-model.md) and [contract design](./contracts/README.md) are normative implementation
inputs; machine contracts and code must implement them and pass their named tests before publication
is enabled. This acceptance closes the architecture task, not an engine, SDK or runtime-test task.

Required release evidence remains migration/old-root replay, every accepted worker minor, process
and transaction failures, stale fences, duplicates/lost delivery, nested quotas, compensation and
chain races, tenant/secret/authorization tests, reviewed all-five conformance and measured workloads.
No benchmark, durability guarantee or security result is inferred from either ADR's acceptance.

[ADR-0032 — Additive workflow IR](../../docs/adr/ADR-0032-additive-workflow-ir.md) supersedes the
original separate IR revision: all capabilities extend `sagat-octopus-orchestrator.io/v1` in the
existing canonical schema. Old forms/runs remain supported, and publication gates actual capability
support independently of schema shape and worker negotiation.
