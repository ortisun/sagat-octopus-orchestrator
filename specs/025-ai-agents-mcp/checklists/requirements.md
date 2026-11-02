# Specification Quality Checklist: AI agents and MCP

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

Filled by the specification author on 2026-10-26 (one validation pass plus one revision that aligned
the spec with the parallel 021 and 026 drafts), revised on 2026-10-27 after the independent
architecture and security reviews and the coordinator's decisions. Not a reviewer sign-off; owner
review is pending.

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs) — **not met, deliberately**: in this
  repository specifications name endpoints, permissions, reserved activities and executors (house
  style of features 014, 016 and 017) because the guarantees are about those boundaries.
- [x] Focused on user value and business needs — each story states who gains what and why now.
- [ ] Written for non-technical stakeholders — **not met**: the audience is engineers and the owner;
  the stories are plain, the requirements are technical.
- [x] All mandatory sections completed — user stories, edge cases, requirements, key entities,
  success criteria, assumptions, out of scope.

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain — three owner questions (O1280–O1282) carry recommended
  defaults in research.md instead.
- [x] Requirements are testable and unambiguous — every FR names its observable outcome or error code
  and is traced to a task and a success criterion (tasks.md, Traceability).
- [x] Success criteria are measurable — each names the automated proof and the rounds or counts.
- [ ] Success criteria are technology-agnostic — **not met, deliberately**: criteria name PostgreSQL,
  the agent runner and real browsers because evidence must come from the real stack (constitution VIII).
- [x] All acceptance scenarios are defined — happy and failure scenarios per story.
- [x] Edge cases are identified — parallel completion, races, rate limits, model substitution, drift,
  lost MCP responses, deletion under a pinned run.
- [x] Scope is clearly bounded — Out of scope section; root-only agents; text modality.
- [x] Dependencies and assumptions identified — 001–017 features, 021, 022, 026 drafts, protocol minor.

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria — via the stories and SC table.
- [x] User scenarios cover primary flows — provider, agent, tools, approval, budgets, data protection,
  MCP server, MCP client, observation, assistant.
- [ ] Feature meets measurable outcomes defined in Success Criteria — **not applicable yet**: nothing
  is implemented; every SC is future evidence.
- [ ] No implementation details leak into specification — **not met, deliberately** (see the first item).

## House rules (constitution and AGENTS.md)

- [x] Transactional boundaries stated (plan.md transaction table) and terminal runs never reopen.
- [x] No exactly-once claim; idempotency or reconciliation for every effect; model calls re-billed and
  said so.
- [x] Untrusted work and third-party credentials outside control plane and scheduler (agent runner).
- [x] Tenant boundary, typed errors without new `Error.code`, OTel with fixed labels, audit with the
  decision.
- [x] Finite defaults and hard ceilings for every new resource (plan.md); zero is not unlimited.
- [x] Five-SDK conformance scenarios and an independent MCP client conformance plan.
- [x] Failure tests planned: crash at each commit point, duplicates, lost broker hint, races, failed
  compensation, restart, quota exhaustion, tenant isolation.
- [x] Unproven items marked pending: protocol minor (G1280), token bound (G1284), provider retention
  (G1285), prompt injection (G1286), cost estimates (G1287), unbilled error responses (G1294),
  registries owned by other drafts (G1635).
- [x] Model-written values never classified below the transcript level; destination ceilings for
  providers and MCP servers; `write` tools need approval or an explicit acknowledgement.

## Notes

- The unchecked items reflect the repository's established specification style or the absence of an
  implementation; they are not hidden defects.
- `spec.md` is about 56 KB after the review fixes, above the 15–35 KB target of the brief, because the feature combines four
  surfaces (agent step, MCP server, MCP client, assistant); splitting it would separate rules that
  share one execution and disclosure model.
