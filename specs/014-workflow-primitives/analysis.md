# Spec Kit consistency analysis

Date: 2026-10-14. Two reviews: a read-only consistency review of the feature's own documents, then
a validation of those documents against the codebase, accepted ADRs and canonical contracts. This is
a design and delivery review, not evidence of future primitive runtime behavior.

The counts and statuses below are a snapshot of 2026-10-14. Since then ADR-0030/0031/0032 settled
the disclosure, iteration-compensation and role-grant decisions, and more tasks closed with
evidence; `tasks.md` is the current status.

## Result

| Measure | Result |
|---|---|
| Functional requirements | 17 |
| Non-functional requirements | 4 |
| Success criteria | 6 |
| Planned requirement/criterion coverage | 27/27 (100%) |
| Unique task IDs | 45 (T014-045 added by the validation, placed in Phase 2 by dependency) |
| Unmapped tasks | 0 |
| Unresolved clarification markers | 0 |
| Findings of the validation against ADRs/contracts | 11, all corrected in the documents; 3 leave explicit pending decisions |
| Completed tasks | T014-001 and T014-010–013 |
| Future implementation/release tasks unchecked | 40 |

The prerequisite helper discovered spec, plan, tasks and all supporting design documents using:

```sh
SPECIFY_FEATURE_DIRECTORY=specs/014-workflow-primitives SPECIFY_FEATURE_NO_PERSIST=1 \
  .specify/scripts/bash/check-prerequisites.sh --json --require-spec --require-tasks --include-tasks
```

## First review: consistency among the feature's documents

The review covered definition/instance identity, acyclic scoped bodies, finite shared budgets,
transactional checkpoints and outbox, current authorization for approval and receipt replay,
chain-before-run locking, segment versus chain completion waits, child-chain finality, compatibility,
observability, tenant boundaries and truthful completion evidence. It resolved one finding: the
quickstart originally supplied only `SPECIFY_FEATURE`, which cannot locate the feature in a clean
checkout without local `.specify/feature.json`; it now supplies `SPECIFY_FEATURE_DIRECTORY` and
`SPECIFY_FEATURE_NO_PERSIST=1`, and the corrected command passed.

That review compared the documents with each other. It did not compare them with the code or the
accepted ADRs, which is why the findings below were not found by it.

## Second review: validation against the codebase and accepted ADRs

| # | Finding | Evidence | Correction |
|---|---|---|---|
| 1 | Compensation was described as "existing reverse-completion order, global". | FR-003-002, ADR-0005, `saga::reverse_dependencies`: reverse topological order respecting branch independence. | Iterations keep FR-003-002: reverse topological within an iteration, none between collection iterations, reverse iteration order for loops. Refines ADR-0005 in T014-002. |
| 2 | Query/Update return payload values through the API. | ADR-0028: no value disclosure without a new decision; `Run` has no output field. | Value-disclosure decision required before US06 release (pending). |
| 3 | New top-level error codes (`RESOURCE_NOT_FOUND`, `IDEMPOTENCY_CONFLICT`, …). | `Error.code` is a closed enum of ten values; refinements are `details.reason`. | Error table remapped to existing codes plus reasons, reusing existing reasons. |
| 4 | Approval policies named role ids as if custom roles existed. | Six built-in roles (`identity.rs`), G39; feature 013 excludes group mapping. | Subjects plus `admin`/`developer`/`operator` only; permissions renamed to `resource:action` with proposed grants. |
| 5 | Continuation drained "pending signals". | `runs.rs` `signal`: accepted only by a waiting step, never buffered (`signalRun`). | Pending-signal state removed; chain signals follow the existing rule. |
| 6 | A new `journalSequence` token duplicated `Run.version`/`expectedRunVersion`, and "expected revision" named two concepts. | `openapi.yaml` `Run.version`, `OperationRequest.expectedRunVersion`. | Update uses `expectedStateRevision`; Query returns `runVersion` and `stateRevision`. |
| 7 | "Work newly enabled by an update" had no mechanism. | No node waits on state; no node assigns state. | Update schedules nothing; state assignment and wait-on-state recorded as pending decisions. |
| 8 | Limits ignored the child-run tree budget. | `MAX_TREE_RUNS` = 1024, all runs under one root. | Budget stated for children in bodies and child-chain successors; WP-12 covers it. |
| 9 | No task projected scoped instances into the runtime graph. | Constitution VI, `/runs/{id}/graph`, ADR-0016/0017. | FR-014-017 extended; T014-045 added; Studio degrades instead of failing. |
| 10 | Chain lineage vs `rerunOf`, and the initiator of machine-started runs, were undefined. | `Run.rerunOf`, `Run.startedBy`. | RERUN_AS_NEW of a segment starts an unrelated run; initiator is `startedBy`. |
| 11 | Relevant ADRs were not cited. | ADR-0005/0010/0012/0016/0017/0023/0027/0028. | Added to research baseline and ADR proposal; next free ADR is ADR-0030. |

The validation also listed the product choices made without a `/speckit-clarify` session; they are
recorded for the owner's confirmation in `research.md` (Product decisions to confirm).

## Evidence and limits

Neither review executed the runtime tests; the US01 results are recorded in
[validation-report.md](validation-report.md). Planned coverage means that tasks exist for the
requirements; it does not mean the 40 future tasks are implemented.
