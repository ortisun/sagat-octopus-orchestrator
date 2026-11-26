# Resolution of round 8 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed commit of round 8: 3884f9f3db9c7ec9c7405f2970a02985c4249711

How the Majors of round 8 were answered, by the author under the owner's delegation (OD-2026-11-20-6,
OD-2026-11-24-4). The decisions are Decision 044-189 to Decision 044-200 and Gap 044-026. Every Nit was applied.

Counts before fixes: design re-read Blocker 0, Major 5, Nit 7; implementation readiness re-read 0, 3, 9; security 0, 3,
8; architecture 0, 5, 9; correctness 0, 2, 10. Total Blocker 0, Major 18, Nit 43. Round 7 had Blocker 0, Major 21,
Nit 38; the correctness review closed all six of its round-7 Majors.

| Findings | Answer |
|---|---|
| design A (can-act set and nested scopes) | Closed children stay compensable by the parent; counted by `root_scope`; an index; a test (Decision 044-189) |
| design B (quadratic plan, no outcome sequence) | `steps.outcome_seq`; linear pairs; a 2,048-member budget test (Decision 044-189) |
| design C (graph-only trees changed) | Rows only in trees with a code run (Decision 044-190) |
| design D; security Nit (no legal encoding for large values) | Message budget split; blobs for every value but the state; state at most 128 KiB (Decision 044-191) |
| design E (terminal command with an open scope on the cancellation path) | The SDK aborts every open scope first; a conformance case (Decision 044-189) |
| readiness 1 (goldens test) | A guard in Task 044-30; the red side in Task 044-5 (Decision 044-200) |
| readiness 2 (analyzer dependencies) | Pinned, placed and reviewed; the Constitution Check updated (Decision 044-200) |
| readiness 3 (`NodeMissing` on the wire) | `INTERNAL` with `NODE_MISSING`, the run unchanged (Decision 044-200) |
| architecture 1 (node spec encoding) | Canonical fragment under `apiVersion`, decoded through `IrCodec`; a baseline (Decision 044-192) |
| architecture 2 (shared hold function) | No list in the body; comparison of the rest; the refinement of Architecture decision 0082 recorded; Gap 044-026 (Decision 044-193) |
| architecture 3 (attempts and spans) | Projector arms for `workflow_task.*` (Decision 044-194) |
| architecture 4 (move verdict contract) | Item reasons, `firstDifference`, retention, a metric verdict (Decision 044-195) |
| architecture 5 (rerun of a moved run) | Starts on the current code version (Decision 044-196) |
| security 1 (worker runner self-approves) | Binding grants authorship, needs `definition:publish`; the binder is refused (Decision 044-190) |
| security 2 (failure message unclassified) | Closed code; message as a `pii` payload (Decision 044-198) |
| security 3 (overlay cache misses) | Keyed by run; only new rows read (Decision 044-199) |
| correctness A (continuation pass cannot hand off a code run) | The pass's code arm specified; the resolved version recorded (Decision 044-197) |
| correctness B (no command record for continue-as-new and acknowledgement) | `run.continue_requested` and `run.stop_requested` (Decision 044-197) |
