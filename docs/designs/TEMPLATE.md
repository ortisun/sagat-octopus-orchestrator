# [Feature name]

*   **Author:** [Author name]
*   **Status:** Draft
*   **Date:** [YYYY-MM-DD]

--------------------------------------------------------------------------------

<!--
This template extends the EGM design template (egm-draft, references/design_doc.md) with what this
repository needs and EGM does not provide. See docs/designs/README.md for the naming, the status
block, the task grammar and the owner-decision rows.

An unnumbered (cross-cutting) design also declares its task prefix in the header, 2 to 8 capital
letters, for example:   *   **Task-prefix:** EGM
A numbered design (docs/designs/NNN-slug.md, NNN from scripts/next-feature-id.py) does not.
-->

## Overview

## The Problem

## Requirements

<!-- Functional requirements FR-NNN-xxx, success criteria SC-NNN-xxx, non-functional NFR-NNN-xxx and
user stories US0n, where NNN is the feature number. A cross-cutting design may omit this section. -->

## Goals

## Non-Goals

## Open Questions

## Owner decisions needed

<!-- One line per product, scope, budget, release or security-exception decision the owner must
make. Record the answer as a row of docs/owner-decisions.md and gate the task on it with an
`Owner-gate: OD-YYYY-MM-DD-N` line. -->

## Critical User Journeys (CUJs)

### CUJ 1: [Journey title]

*   **Given**:
*   **When**:
*   **Then**:

## Technical Plan (Architecture)

## Risks and Mitigations

## Constitution Check

<!-- One row per principle of docs/constitution.md and one for Delivery Constraints. Result:
Pass, Pass with conditions (justify in Complexity Tracking), Fail, or n/a. Rows start exactly as
shown, because scripts/check-design-docs.py looks for them. -->

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | | |
| II. Modular domain, ports and adapters | | |
| III. External effects are at-least-once; sagas compensate | | |
| IV. Compatibility is versioned and pinned | | |
| V. Code, tenant boundary and secrets are isolated | | |
| VI. Observability and audit are part of the feature | | |
| VII. Events are redeliverable; inbox and outbox are the contract | | |
| VIII. Failure is tested and contracts are verified, honestly | | |
| Delivery Constraints | | |

## Complexity Tracking

<!-- Every violation or added moving part, with the simpler alternative that was rejected. -->

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|

## Alternatives Considered

## Non-Functional Requirements

--------------------------------------------------------------------------------

## Detailed Implementation Plan

<!-- Tasks use the grammar of docs/designs/README.md. Tick a task only with an evidence block, and
only after the doc is Approved. -->

- [ ] T000-1 Describe the task in one line.
  Refs: FR-000-001
  Owner-gate: OD-YYYY-MM-DD-N

#### Follow-up tasks

None yet.

### Verification Plan
