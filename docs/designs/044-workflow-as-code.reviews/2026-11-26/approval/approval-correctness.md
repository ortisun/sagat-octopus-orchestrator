# Targeted re-read of design 044 for approval: correctness, confirmation (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 14513c8533b0a0a475d77fe21f3886c4e07c52cc
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Scope: the text changed
since f4f55b14 (the first correctness pass, `reread-1-correctness.md`). All 19 findings of that pass are closed, three
with a caveat raised below. The Majors are settled in writing under OD-2026-11-24-4 (rows OD-2026-11-26-441 upward in
`docs/owner-decisions.md`), each as a follow-up task of the design.

## Major

1. A crashed lease's command-value blobs keep counting until the 24-hour upload sweep: sweep an uncommitted, unheld
   row once its lease is no longer current.
2. Held deferral blobs count toward the tenant and namespace ceilings and can form a circular wait with children.
3. Uncounted expiries live only on the step row, so `verify_run` and a reset disagree with the stored count.
4. The SDK cannot check carried state against an `active` successor it does not know; the completion's refusal is unnamed.
5. The consumption list's inline bytes are not reserved against gated inputs.
6. The upload path fails the workflow task under the step-row lock, outside the run transaction's lock order.
7. A sweep racing the commit of an earlier lease's blob reviews the run as `COMMAND_INVALID`.

## Nit

8. "A tenant above 4 MiB" judged after the acquisition.
9. No claim-time column for the replay check's 3,600 s cap.
10. Stale register entries (3 MiB, uncounted refusals, the 300 s lease, missing decisions).
11. `WithoutGrace` also journals `run.stop_requested`.
12. The can-act index includes `~workflow` unless filtered.
13. The order of failed update results; the definition of blob-carried bytes.
14. A failed session check's outcome; binding changes closing sessions of uncovered workflow types.
15. A misplaced clause in Architecture decision 0108.

Counts: Blocker 0, Major 7, Nit 8
