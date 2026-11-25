# ADR-0109: Workflow tasks, worker queries and replay checks travel on the next free worker-protocol minor, as additive messages

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6, OD-2026-11-24-4) and the register rule of OD-2026-11-15-8
**Refines / supersedes:** extends ADR-0015 (minor-version negotiation) and ADR-0029 (task streams)
**Enforced by:** `contract_workflow_as_code::the_worker_protocol_change_is_additive_to_the_baseline` against `worker.baseline.json`; `a_session_below_the_minor_is_never_offered_a_workflow_task`; `a_workflow_task_lease_is_refused_by_every_activity_rpc`; the conformance scenario `protocol-negotiation` extended with a workflow worker (design 044, Tasks 044-2, 044-8)

## Context

The register of minors in `sdks/SPEC.md` gives 1.7 to feature 041; the branch of feature 006's delivery acknowledgement
reserves 1.8; drafts 019, 020, 021, 025, 026 and 031 each need minors and take the next free one when their contract
merges. A code workflow needs new task kinds, a history stream, new completions, and fields for the deferrable parts of
design 044 (eager dispatch, worker queries, behaviour flags) so that they can land later without reshaping the contract.

## Decision

1. The messages of design 044, Technical Plan 13, MUST take the next free minor of the register when their contract
   merges, recorded in the register in the same pull request.
2. They MUST be additive: new fields, enumeration values, messages and RPCs only; no existing field changes meaning. The
   fields of the deferrable parts MUST be in that same minor.
3. A session MUST be offered `WORKFLOW`, `QUERY` and `REPLAY_CHECK` tasks only at that minor or later and only for the
   `(workflow type, code digest)` pairs it registered (at most 64); every activity RPC MUST refuse those leases.
4. The history a worker receives MUST be the typed `HistoryEvent` projection, not the journal's internal JSON, and MUST
   NOT carry the fragments of past commands.

## Considered options

1. **The next free minor, additive, with the deferrable fields reserved** — the register's rule; old workers unaffected;
   later parts need no new minor.
2. **A separate gRPC service for workflow tasks** — lost: a second session, authentication and stream to keep in step,
   for no semantic gain.
3. **Raw journal rows on the wire** — lost: the journal's detail could no longer change without a protocol change, and
   the fragments would reach the worker.

## Consequences

- Good: activity-only workers keep their minor; workflow workers negotiate as every worker does.
- Bad: the exact minor number is known only at merge; documents say "the next free minor" until then (Gap 044-013).

## Evidence

Design 044, Technical Plan 13; the register in `sdks/SPEC.md`, section 9.
