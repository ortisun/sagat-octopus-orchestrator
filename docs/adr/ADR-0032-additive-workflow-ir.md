# ADR-0032 — Extend the existing workflow IR additively

Status: **Accepted architecture; implementation and release evidence pending** | Date: 2026-10-14 | Feature: 014 | Supersedes ADR-0030's separate IR v2 allocation and version-based dispatch only; refines ADR-0031's revision wording

## Context

The project is still establishing its first production-ready workflow contract. The user explicitly
directed this delivery to retain `sagat-octopus-orchestrator.io/v1` rather than introduce another
public workflow revision. ADR-0030 allocated a separate revision before structured execution had
been released. Its identity, transaction, compensation, authorization and bounded-evaluation
decisions remain necessary; a second IR document is not necessary to implement them.

The existing canonical schema and runtime support are distinct. In particular, the old schema
contains `foreach` and `humanApproval` shapes that the codec does not publish. Schema acceptance
alone cannot establish runnable semantics, and preserving a schema shape must not accidentally
enable it. Existing accepted definitions and stored runs must retain their meaning.

## Decision

1. Keep the only supported workflow `apiVersion` as `sagat-octopus-orchestrator.io/v1` and extend
   `specs/001-platform-foundation/contracts/workflow.schema.json` in place. Add named `bodies`,
   `limits`, declared `state`, `chainPolicy` and the explicit `forEach`, `while`, `repeatUntil`,
   `approval`, `switch`, `return` and `continueAsNew` shapes. No separate v2 schema, endpoint,
   SDK builder family or migration to a new workflow revision is introduced.
2. Preserve every previously accepted root document form, existing node branch, default, root
   identity, event meaning and published-version pin. Old schema branches are not tightened to
   the new closed-body rules. New declarations and node kinds use explicit bounded fields;
   a body has one local return and cannot escape, recurse or terminate the workflow. Previously
   unsupported `foreach` and `humanApproval` remain unsupported; they are not implicit aliases
   for the new kinds. Unknown fields or capabilities are never silently dropped by the codec.
3. Publication validates shape, definition semantics, expression support and the server's enabled
   runtime capabilities. A kind is unavailable until its codec, reducer, persistence, security,
   faults and five-SDK conformance are proven. Unsupported definitions fail before publication
   or run creation using the existing error envelope. A canonical schema describing a planned
   kind does not enable it. `WorkflowDefinition::new_structured` is an internal construction and
   validation API; its name does not allocate an IR revision or promise execution support.
4. Determine required capabilities from the immutable published definition, not from a new public
   revision or a caller-controlled compatibility flag. Decode persisted identity/chain information
   explicitly. Missing instance path means the existing root identity; existing runs without a
   chain retain legacy parent/child behavior. Never choose the new lock protocol solely because
   `apiVersion` is v1: both old and extended definitions use that value. Migration must preserve
   old-root reads and replay before writers emit scoped events; do not rewrite historic events
   or automatically convert an active legacy run to chain semantics.
5. Keep worker protocol minor **1.5** for explicit scoped task identity. This additive protocol
   negotiation is independent of the workflow `apiVersion` and SDK package versions. All accepted
   older worker minor forms remain supported for eligible root tasks; neither polling nor streaming
   may hand an unrepresentable scoped task to an older worker.
6. ADR-0030's scoped identities, lifetime admission reserves, compensation dependencies, chain
   quiescence and chain-before-own-current-run / child-link lock order remain in force for runs
   using those capabilities. ADR-0031's explicit declared-result disclosure, role grants, closed
   result schemas, current authorization on replay and audit requirements apply to additive v1
   Query/Update declarations. Neither policy depends on a second public IR revision.
7. Preserve the evaluator's effective 1024-byte source, 256-token, depth-16, 64-element literal,
   4096-element constructed-list and 64 KiB generated-string limits. Do not reduce the old domain
   wrapper or retroactively reject persisted definitions solely through a new schema restriction.
   Newly authored structured expressions must satisfy the existing evaluator; old publication
   validation retains its established checks. A 10,000-item input/blob collection still requires
   materialization and byte-budget evidence, not a higher expression limit.
8. Roll out one enabled capability at a time. Rollback disables new publication/start for that
   capability and retains compatible readers/workers until affected runs drain. No capability
   rollback may erase active scoped instances, chain links, receipts or retained values. An older
   server that cannot decode the extensions is not a safe reader for those in-flight runs merely
   because their `apiVersion` is unchanged.

## Alternatives and consequences

A separate public IR revision was the previous design. At this project stage it would introduce
another schema, builder branch and migration boundary without a released incompatible contract
that requires them. Silently accepting unknown nodes would avoid explicit capability checks but
could publish definitions the scheduler cannot execute, so it remains prohibited.

The additive design keeps one authoring contract and preserves existing documents. Its cost is
that `apiVersion` alone cannot describe the server's capability set. Publication and worker
negotiation must check actual support, and operational rollback must retain readers for the
features already present in stored definitions. New schema tests therefore cover both old forms
and new shapes; replay, runtime and compatibility evidence remain separate release gates.

## Review criteria and pending evidence

Verify the canonical schema still accepts the previous fixture corpus and leaves previous node
definitions/root property constraints intact; reject a second workflow `apiVersion`. Verify new
bounded/closed shapes, legacy unsupported aliases and enabled-capability rejection. Run the
existing domain/codec suites and contract tests. Before releasing a capability, also prove the
transaction, replay, fault, tenant, audit, negotiation and five-SDK scenarios required by
ADR-0030/0031. Acceptance of this decision supplies none of that runtime evidence.
