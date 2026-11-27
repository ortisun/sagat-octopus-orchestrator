# Specification Quality Checklist: 021 Integration resources, managed operations, variables and triggers

**Purpose**: Validate specification completeness and quality before planning and implementation
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

Filled by the specification author (agent) on 2026-10-26. `[x]` means the author judged the
criterion met; an owner review is still required (T021-001). Notes record every judgement call.

## Content Quality

- [x] No implementation details beyond what the platform's contracts require (see note 1)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders (see note 2) — accepted as not applicable, delegate (OD-2026-11-24-4), note 5
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain (three owner questions carry recommended defaults instead)
- [x] Requirements are testable and unambiguous (each FR names its contract or observable outcome)
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (see note 3) — accepted with the protocol names kept, delegate (OD-2026-11-24-4), note 6
- [x] All acceptance scenarios are defined (nine stories, Given/When/Then, each with a failure case)
- [x] Edge cases are identified
- [x] Scope is clearly bounded (Out of Scope, ownership by 022/024/025/026/029/031)
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria (every FR is traced by an SC and a task)
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] No implementation details leak into the specification beyond note 1

## Project-specific gates

- [x] Feature 017 contracts and data unchanged; sharing, not migration, decided and justified (ADR-0055, D1120)
- [x] Revisions pinned at admission; credential versions per attempt; secrets only as OpenBao references
- [x] No operation in API, scheduler or engine; managed executor under lease with two-phase resolution
- [x] Egress guard for every connection including OAuth2 token requests
- [x] Every effect at-least-once with an explicit idempotency/reconciliation/manual-review rule; no exactly-once claim
- [x] Every inbound source admits through the inbox before acknowledging, or states that it cannot
- [x] Tenant boundary, typed errors (`details.reason` only), OTel, audit and finite ceilings specified
- [x] Classification: outputs are carriers (undeclared ⇒ `pii`); variables `public|internal`; event extensions disclosed like `data`
- [x] Five-SDK conformance and Studio tasks present
- [x] Rule-9 failure tests planned per story (crash points, duplicates, timeout vs completion, lost hint, failed compensation, restart, tenant isolation, quota exhaustion)
- [x] Owner answers to O1120–O1123 recorded (pending T021-001) — decided 2026-11-24, delegate (OD-2026-11-24-4), note 7
- [x] ADR-0055 accepted by the owner (pending T021-001) — accepted 2026-11-24 with its amendment, delegate (OD-2026-11-24-4), note 8
- [x] Re-read against `develop` at `2c94064f` (2026-11-24): every stale item fixed in the spec, plan, tasks, data model and contracts (research.md, Readiness review)

- [x] Architecture and security review findings (2026-10-27) and coordinator decisions C1, C3, C4, C6, C7, C10, C12, C16, C21 applied (research.md, Review resolution)
- [x] Placement of every new customer-credentialed call follows ADR-0055 decision 11 (managed executors; no engine socket for new kinds)

## Notes

1. The spec names wire-level elements (`resourceOperation`, `details.reason` values, HTTP status
   codes, protocol behaviours of MQTT, Pub/Sub, Service Bus and PostgreSQL replication). In this
   repository those are the externally observable contract that tests verify, so they stay in the
   spec; crate placement, tables and libraries are kept in the plan and data model.
2. The audience is platform engineers and operators; the vocabulary section helps, but the
   document is not written for a non-technical reader. Accepted as house style (compare 017).
3. Several criteria name protocols (Kafka key, Mailgun signatures, CDC flush) because the guarantee
   being measured is protocol-specific. Measurable, but not technology-agnostic.
4. Size: the spec is above the template's typical length because the scope covers nine
   sub-capabilities; per-kind detail was moved to `contracts/` to keep FRs short.

Decisions of 2026-11-24 on the four items that were open, each under the owner's delegation, "delegate
(OD-2026-11-24-4)", which lets the agents accept or reject unticked checklist items for release 0.3.0:

5. "Written for non-technical stakeholders": accepted as not applicable. Options weighed: rewrite the
   spec for a non-technical reader, which would move the observable contract (status codes,
   acknowledgement order, reasons) out of the spec into the plan and make the requirements untestable
   from the spec alone; or accept the platform-engineer audience, as every accepted feature of this
   repository does. The user stories, the "Why this priority" lines and the domain
   vocabulary already give a product reader what each capability is for; the requirements are for the
   people who build and verify it, the same judgement as feature 018's checklist ("not met,
   deliberately") and feature 035's note on non-technical readers. Accepted; the spec is not
   changed for this item.
6. "Success criteria are technology-agnostic": accepted with the protocol names kept. The criteria
   measure guarantees that exist only per protocol (an MQTT acknowledgement order, a CDC flush
   position, a Mailgun signature): a protocol-neutral wording would no longer say what is proven, and
   constitution Principle VIII asks that a guarantee be stated where it is tested. Each criterion is
   measurable and names its automated proof. Accepted; no change.
7. Owner questions 1120 to 1123: decided with their option analysis in
   [research.md](../research.md#owner-questions). Task 021-001 keeps only the coordinator's
   allocation of the protocol minor, migration timestamps and permission order.
8. Architecture decision 0055: accepted with the amendment of 2026-11-24; the reasoning is in its
   status block. Its "Enforced by" checks and evidence list remain to be built.
