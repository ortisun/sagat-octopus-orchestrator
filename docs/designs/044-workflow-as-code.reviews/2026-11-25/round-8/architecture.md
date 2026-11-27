# Architecture review of design 044, round 8 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 3884f9f3db9c7ec9c7405f2970a02985c4249711
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 7's Majors A, B
and D are closed, C partly closed. Layering, the Constitution Check's gating, the supersessions, the arithmetic and the
code references hold.

## Major

1. The node spec is a persisted format with no defined encoding or decoder: the canonical code-profile fragment merged
   with the entry's fields under the IR's `apiVersion`, decoded only through `IrCodec`; a round-trip test and a baseline.
2. The shared hold function cannot be compared byte for byte, since each design adds capabilities to its list, and its
   refinement of Architecture decision 0082 is recorded only on another branch: no list in the body, compare the rest,
   cite the refinement in Architecture decision 0105.
3. Feature 008's journal-derived attempts and spans do not know workflow tasks: arms in the projector for the
   `workflow_task.*` events; a test.
4. The move's verdict and first difference have no contract: item reasons for a mismatch and an envelope misfit, the
   dry-run outcome, `firstDifference` on `BatchItem`, retention, a metric verdict; tests.
5. A rerun of a moved code run starts on the code it was moved away from: start on the current code version; a test.

## Nit

6. `CAPABILITY_NOT_ACTIVE` 409 against design 045's 422 for the same condition.
7. The raw-insert rule should match case-insensitively and across line breaks.
8. Exhaustiveness of `is_visible` and `is_waking` is enforced by nothing; a lint on them.
9. A forward rule for `HistoryEventKind` against older minors.
10. The Studio's definition pages and a code draft; the approval screen's note has no task.
11. `worker_binding_workflow_types` has no key.
12. Architecture decision 0103's Enforced by omits the expiry parity test.
13. Two refusals of removal on `platform_requirements`: say which carries `code-workflows-v1`.
14. Run-level pause and manual-review hold on `~workflow`.

Counts: Blocker 0, Major 5, Nit 9
