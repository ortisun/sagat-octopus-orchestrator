# Specification Quality Checklist: AI agents and MCP

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

Filled by the specification author on 2026-10-26 (one validation pass plus one revision that aligned
the spec with the parallel 021 and 026 drafts), revised on 2026-10-27 after the independent
architecture and security reviews and the coordinator's decisions, and revised again on 2026-11-26
after an independent audit against `develop` (3 Blockers, 19 Majors, 14 Nits, each corrected). Every
item that was unticked is now disposed of under the owner's delegation (Owner decision 2026-11-24-4):
ticked where the corrected set meets it, or left unticked with "Accepted as it stands, delegate (Owner
decision 2026-11-26-125)" and its reason. No security item is accepted this way; every security item
below is ticked on the corrected text.

Revised on 2026-11-27 after the security review of that date (`reviews/2026-11-27-security.md`, 9
Majors and 11 Nits): every finding is fixed in the set, none accepted (Decision 025-016); the items
below that the fixes touch were re-checked against the revised text and keep their ticks, with the new
evidence named in them. One owner question was open (Owner question 025-001); it blocked nothing in
scope, because the modes it concerns are refused until Gap 025-001 is closed.

Revised again on 2026-11-27 (round 3): Owner question 025-001 is decided (no, Owner decision
2026-11-27-400; the Entra and AWS modes reach the runner only as access tokens from design 047's
broker, Decision 025-017); the owner accepted Gap 025-004 as a documented known limit (Owner decision
2026-11-27-2); MCP tool annotations are hints under a classification the administrator pins (Decision
025-018); and a tool-call approval shows a redacted view and escalates when nobody can decide it with
context (Decision 025-019). The items these touch were re-checked and keep their ticks with the new
evidence named in them.

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs) — **not met, deliberately**: in this
  repository specifications name endpoints, permissions, reserved activities and executors (house
  style of features 014, 016 and 017) because the guarantees are about those boundaries. Accepted as
  it stands, delegate (Owner decision 2026-11-26-125): the guarantees of this feature (tenant boundary,
  credential delivery, egress ceilings) are stated at the boundaries that enforce them; removing the
  names would make them untestable.
- [x] Focused on user value and business needs — each story states who gains what and why now.
- [ ] Written for non-technical stakeholders — **not met**: the audience is engineers and the owner;
  the stories are plain, the requirements are technical. Accepted as it stands, delegate (Owner
  decision 2026-11-26-125): the user stories and the context section are the plain-language part; the
  requirements are a build contract for engineers.
- [x] All mandatory sections completed — user stories, edge cases, requirements, key entities,
  success criteria, assumptions, out of scope.

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain — the three owner questions (Owner questions 1280 to
  1282) are decided (Owner decisions 2026-11-26-120 to 2026-11-26-122); the one open dependency, OAuth
  MCP servers and the provider modes that need a client secret or a signing key, is a recorded gap (Gap
  025-001) with its parts marked as waiting and refused, and the owner question it raised (Owner
  question 025-001) is decided (Owner decision 2026-11-27-400).
- [x] Requirements are testable and unambiguous — every requirement names its observable outcome or
  error code and is traced to a task and a success criterion (tasks.md, Traceability).
- [x] Success criteria are measurable — each names the automated proof and the rounds or counts.
- [ ] Success criteria are technology-agnostic — **not met, deliberately**: criteria name PostgreSQL,
  the agent runner and real browsers because evidence must come from the real stack (constitution VIII).
  Accepted as it stands, delegate (Owner decision 2026-11-26-125): Principle VIII requires evidence on
  the real stack, so the criteria name it.
- [x] All acceptance scenarios are defined — happy and failure scenarios per story.
- [x] Edge cases are identified — parallel completion, races, rate limits, model substitution, drift,
  lost MCP responses, deletion under a pinned run, unattended writes in child workflows, undeclared
  tool names as a covert channel, batches, a ceiling lowered after publication, a redirected probe, an
  MCP server that falsely marks a tool read-only, an approval nobody can decide with context.
- [x] Scope is clearly bounded — Out of scope section; root-only, graph-model-only agents; text
  modality; OAuth MCP servers waiting for Gap 025-001.
- [x] Dependencies and assumptions identified — features 001–018, 035, 038, 041 and 043 (merged or
  built), drafts 020, 021, 022, 026, 028, 029, 031, 032, designs 040, 044 and 047, the protocol-minor
  register.

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria — via the stories and SC table.
- [x] User scenarios cover primary flows — provider, agent, tools, approval, budgets, data protection,
  MCP server, MCP client, observation, assistant.
- [ ] Feature meets measurable outcomes defined in Success Criteria — **not applicable yet**: nothing
  is implemented; every SC is future evidence. Accepted as it stands, delegate (Owner decision
  2026-11-26-125): this item can only be met by the build; Task 025-072 records the evidence for each
  success criterion and closes it.
- [ ] No implementation details leak into specification — **not met, deliberately** (see the first
  item). Accepted as it stands, delegate (Owner decision 2026-11-26-125): same reason as the first item.

## House rules (constitution and AGENTS.md)

- [x] Transactional boundaries stated (plan.md transaction table) and terminal runs never reopen.
- [x] No exactly-once claim; idempotency or reconciliation for every effect; model calls re-billed and
  said so.
- [x] Untrusted work and third-party credentials outside control plane and scheduler (agent runner), and
  the credential disclosure fits Principle V's first declared set (Decision 025-001) for static API keys
  and bearer tokens only; the materialized node of every AI kind has no `secrets` and `ResolveSecrets`
  refuses managed leases; no redirect is followed; OAuth MCP servers and the provider modes that need a
  client secret or a signing key are refused until design 047's broker issues access tokens under
  constitution 5.1.0's third declared set (adopted, pull request 286; Gap 025-001); once it does, the
  runner receives only the Entra access token, never the client secret, and `aws-access-key` stays
  refused until the owner answers Owner question 025-002 (Decision 025-020), so never the
  long-lived AWS key or a session signing key (Owner question 025-001 answered no, Decision 025-017,
  Tasks 025-100 and 025-101).
- [x] Tenant boundary, typed errors without new `Error.code`, managed-step codes in the SDK error
  catalog, OTel with fixed labels, audit with the decision and its OCSF mapping (Gap 025-003).
- [x] Finite defaults and hard ceilings for every new resource (plan.md); zero is not unlimited.
- [x] Six-SDK conformance scenarios (Rust included) and an independent MCP client conformance plan.
- [x] Failure tests planned: crash at each commit point, duplicates, lost broker hint, races, failed
  compensation, restart, quota exhaustion, tenant isolation.
- [x] Unproven items marked pending: protocol minor (G1280), token bound (G1284), provider retention
  (G1285), prompt injection (G1286), cost estimates (G1287), unbilled error responses (G1294),
  registries owned by other drafts (G1635), OAuth MCP servers and the client-secret and signing-key
  provider modes (G025-001), the procedural envelope (G025-002), the OCSF mapping (G025-003), the bound
  of the output secret scan (G025-004, accepted by the owner as a documented known limit, Owner
  decision 2026-11-27-2, published in the form of Architecture decision 0085 rule 12 by Task 025-069).
- [x] Model-written values never classified below the transcript level, in the agent's result and in
  every workflow or child output carrier they reach through an expression or a child output (inherited
  output levels, Task 025-093); an activity or script that returns them creates a new value under its
  own carrier, stated in the threat model (Decision 025-020);
  destination ceilings for providers and MCP servers (`EgressCeiling`) applied at publication and again
  at admission and every resolution against the current revision (Task 025-097); `write` tools need
  approval or an explicit acknowledgement, the unattended refusal is transitive over child workflows
  (Task 025-094), an `mcp` tool runs without approval only when the pinned MCP-server revision
  classifies it `read`, the server's annotations being hints whose disagreement is audited (Task
  025-102), and a tool-call approval is decided only by someone who can see every decision field on
  the redacted view, an approval nobody can decide being escalated and, failing that, ended with the
  recorded reason `NO_DECIDER_WITH_CONTEXT` (Tasks 025-096 and 025-103).
- [x] MCP server: receipts and task handles belong to the credential identity and the exposure;
  credential and rights are rechecked before a result leaves; batches are refused; remote callers are
  bounded per caller and per exposure (Tasks 025-048, 025-099).
- [x] Run-read surfaces carry no string a model or a provider wrote (Task 025-098), and the transcript
  and agent payloads are readable only through the audited transcript endpoint (Task 025-041).

## Notes

- The unchecked items reflect the repository's established specification style or the absence of an
  implementation; each carries its disposition above, and none is a security item.
- `spec.md` is about 67 KB after the review and audit fixes, above the 15–35 KB target of the brief,
  because the feature combines four surfaces (agent step, MCP server, MCP client, assistant); splitting
  it would separate rules that share one execution and disclosure model.
- Readiness (2026-11-27): the short security re-read of the fixes and of the round 3 decisions was
  run (`reviews/2026-11-27-security-reread.md`; 4 Majors and 7 Nits) and every finding is fixed, none
  accepted (Decision 025-020). Feature 025's readiness for the build is complete under Owner decision
  2026-11-25-2, except Task 025-101 (the AWS broker route), which waits for the owner's answer to Owner
  question 025-002; `aws-access-key` stays refused until then.
