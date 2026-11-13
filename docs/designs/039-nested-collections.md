# Nested collections and the capacity program

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-12

--------------------------------------------------------------------------------

## Overview

General nesting of the feature-014 primitives: a `forEach` inside a `forEach` body, a collection beside a loop or an
approval, and the capacity program that makes nested admission safe (holder discovery, the sweeper, the creator of
capacity groups and members, the runnable predicate, activation). This design holds the scope that feature 014 handed
over by OD-2026-11-12-6, so that nothing of it is lost when 014 closes on root collections. It is a placeholder to be
written out with `egm-draft`; it decides nothing yet.

## The Problem

Feature 014 ships root collections, loops, approval, switch, declared operations and a restricted continue-as-new.
Nested documents are refused by the codec (`crates/infra-ir/src/convert.rs`, `published_collection_definition`): a
nested scope admits work for its parent's children, and doing that safely needs a capacity group whose possession moves
between members under one lock order. Much of that machinery is built and dormant in feature 014 (holder discovery,
Acquire, Vacate, Finish, transfer receipts, the ChildRoot worker-lease guard behind `child_root_capacity_guard`, off).
What is missing is listed in feature 014's own documents, which this design takes over:

- `specs/014-workflow-primitives/tasks.md`, the "Current frontier (2026-11-01)" note and the nested parts of T014-014
  to T014-017, T014-040 and T014-042;
- `specs/014-workflow-primitives/capacity-*.md` (discovery, holder discovery, coordinator, eligibility, Acquire,
  Vacate/Acquire);
- ADR-0075 (accepted for the guard only, OD-2026-11-04-3) and its activation gate (a) to (l);
- ADR-0078 (proposed): lock order and possession protocol, and its "Open and not decided here" items;
- gaps G715, G716, G718, G719, G720, G721, G722, G724, G725 and G726 in `specs/014-workflow-primitives/research.md`.

## Requirements

To be written. They are the nested acceptance criteria that left feature 014:

- **FR-039-001** Nested collections and collections beside loops and approvals publish and run, with descendant
  admission at limit one, joins that take no fresh admission, lifetime reservations, and reduction to zero (was
  T014-014 and T014-015, nested part).
- **FR-039-002** The broader body and source families of collections, each with a bounded proof plan, in all six
  SDKs (was T014-016, nested and broader part).
- **SC-039-001** Crash, duplicate, stale fence, lost broker, cancellation and unknown or failed compensation are proven
  for nested scopes, including the six released-source profiles and the uncertain-COMMIT and expired-authority
  compositions (was T014-017 and the nested part of T014-040).
- **SC-039-002** The nested 100,000-instance workload, on a separate database host, release build, runtime role, two
  repetitions (was OD-2026-11-04-7 in T014-042).

## Goals

- Nesting runs in production behind a recorded activation, with every condition of ADR-0075's activation gate held.

## Non-Goals

- Anything feature 014 already ships.
- Changing the transactional boundary or the lock order outside what ADR-0078 decides.

## Open Questions

- Every open item of ADR-0078, and whether the first full-stack run may be the release event itself
  (`implementation-ledger.md`).

## Owner decisions needed

- Accept ADR-0078, and decide its open items: the creator (API, audit, insert grants, lock order against ADR-0030 §19
  and ADR-0037 §6), sweeper coverage of non-holder FINAL members and vacant groups, `SCOPED_PATH` (G716), the receipt
  ceiling (G720) and fairness (G718).
- Confirm or change the delegated rows OD-2026-11-04-3 to OD-2026-11-04-8 that concern nesting.
- Accept this design for implementation once it is written out.

## Critical User Journeys (CUJs)

### CUJ 1: A collection of collections

*   **Given**: a workflow whose `forEach` body is itself a `forEach`.
*   **When**: it is published and run.
*   **Then**: every inner iteration runs once, the tenant's concurrency limit holds across both levels, and a crash at
    any point resumes to the same end.

## Technical Plan (Architecture)

To be written. The order recorded by feature 014: shared runnable predicate (G715) and G724/G725; the sweeper on the
scheduler tick; the creator and member insertion (migration, unique index, grants, quota, backfill, retention); barrier
and unknown-COMMIT acceptance, the lock-order check and the activation flag; then the codec opening, the nested tests,
the faults, the performance run and the traceability.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A nested admission oversubscribes a tenant | ADR-0075's activation gate; nothing is enabled before every condition holds |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | n/a | Not yet designed |
| II. Modular domain, ports and adapters | n/a | Not yet designed |
| III. External effects are at-least-once; sagas compensate | n/a | Not yet designed |
| IV. Compatibility is versioned and pinned | n/a | Not yet designed |
| V. Code, tenant boundary and secrets are isolated | n/a | Not yet designed |
| VI. Observability and audit are part of the feature | n/a | Not yet designed |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | Not yet designed |
| VIII. Failure is tested and contracts are verified, honestly | n/a | Not yet designed |
| Delivery Constraints | n/a | Not yet designed |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|

## Alternatives Considered

- Keeping nesting in feature 014 with its original criteria: rejected by OD-2026-11-12-6.

## Non-Functional Requirements

- As SC-039-002.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T039-1 Write this design out with `egm-draft` from feature 014's capacity documents and ADR-0078.
  Refs: FR-039-001

#### Follow-up tasks

None yet.

### Verification Plan

To be written with the design.
