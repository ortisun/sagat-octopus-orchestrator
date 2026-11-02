# Specification Quality Checklist: Multi-region replication and active-passive failover

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

**Review Ownership**: filled by the specification author on 2026-10-26; a reviewer must confirm. `[x]` means
the requirement-quality criterion is met in the document; it says nothing about implementation, of which
there is none.

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs) — **partly not met, on purpose.** The
  feature is about PostgreSQL physical replication, its synchronous-commit behaviour and the platform's
  existing contracts; requirements name `pg_last_wal_replay_lsn()`, `GraphChanges.reset`, error reasons
  and endpoints because the guarantees depend on them (FR-032-007, 009, 018). Crate placement is kept
  in the plan.
- [x] Focused on user value and business needs (operator, tenant administrator, client and integrator
  journeys US01–US07; what is lost or kept after a failover).
- [ ] Written for non-technical stakeholders — **not met.** The audience is platform operators and
  integrators; the owner questions are written for the owner.
- [x] All mandatory sections completed (stories, edge cases, FR, NFR, key entities, SC, assumptions,
  out of scope, owner questions).

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain (three genuine owner choices are O1560–O1562 with
  recommended defaults; the spec is written against the defaults).
- [x] Requirements are testable and unambiguous (each FR names its refusal, transaction or observable;
  each is traced to tasks in `tasks.md`).
- [x] Success criteria are measurable (counts of missing acknowledgments, refusals, duplicates, resets;
  latency percentiles as benchmark criteria).
- [ ] Success criteria are technology-agnostic — **partly not met**: SC-032-002 and SC-032-011 refer to
  replication modes and commit latency, which are the subject of the feature.
- [x] All acceptance scenarios are defined (Given/When/Then for every story, including failure paths).
- [x] Edge cases are identified (partition with live former primary, `ANY 1` quorum, cancelled wait,
  restart while waiting, promotion without activation, double activation, wrong cell, clocks, sequence
  reuse, object lag, webhook senders, regional brokers, OpenBao, fan-out interruption, mixed versions).
- [x] Scope is clearly bounded (active-active, automatic election, stale reads, logical replication,
  relocation, cross-cell tenants, broker mirroring, OpenBao replication, merging divergent writes are out).
- [x] Dependencies and assumptions identified (features 001–017 reused; 019/020/021/026/030 interactions;
  operator-provided replication, buckets, DNS/GSLB, NTP).

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria (FR → SC mapping in spec; FR → task
  mapping in tasks).
- [x] User scenarios cover primary flows (init/observe, planned switchover, unplanned failover, work after
  failover, placement, drill, reconciliation).
- [ ] Feature meets measurable outcomes defined in Success Criteria — **not applicable yet**: nothing is
  implemented; SC-032-001..013 are future evidence.
- [ ] No implementation details leak into specification — see the first item.

## Constitution and house rules

- [x] Constitution conflict identified and routed: the single-region Delivery Constraint requires
  ADR-0067 and an amendment (O1560); active-active stays out of scope.
- [x] Tenant boundary, typed errors, OTel, audit and finite ceilings are explicit (FR-032-022..028, 032;
  plan ceilings table).
- [x] Rule-9 failure tests planned: crash at each critical point (activation, commit), duplicates,
  timeout/signal race, lost broker delivery, failed compensation, restart, tenant isolation,
  quota/ceiling exhaustion, region loss mid-commit (tasks T032-020, 023, 030–032, 053, 055, 056).
- [x] No invented numbers: RPO/RTO and latency are benchmark criteria; defaults in the ceilings table are
  engineering defaults, not measured capacity.

## Notes

- The unchecked items are deliberate and explained; they do not block planning, but a reviewer should
  confirm that the technical vocabulary in the spec is acceptable for this infrastructure feature.
- D1566 (cancelled synchronous wait) is a safety assumption to be proven by T032-023/024 before the design
  is called safe; it is recorded as G1578.
- Review round 2026-10-27 (independent architecture review and coordinator decisions C2, C11, C12, C13)
  applied: acknowledgments through read paths (FR-032-009, D1590), in-region RPO inside a multi-region
  cell (FR-032-010, D1591), governance gate (O1560, D1592, T032-063), database-enforced epochs
  (FR-032-006, D1593), epoch in downstream keys and tombstones (FR-032-018, 034, D1594), key inventory
  (FR-032-020, D1595), CDC under async (FR-032-015, D1596), time classes (FR-032-017, D1597), peer probe on
  the admin listener (FR-032-011, D1598), connector leases at activation (FR-032-007, D1599). The items
  above were re-checked against the revised text; their marks are unchanged.
