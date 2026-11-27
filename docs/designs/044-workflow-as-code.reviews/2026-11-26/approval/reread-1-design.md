# Targeted re-read of design 044 for approval: egm design lens, first pass (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: c6d9001946a1d641443784f7dae2d60cd115d281
Reviewer: egm-design, feature-spec standard, the text changed since round 11 only
Note: condensed by the author from the reviewer's output; every finding and severity is kept. All nine round-11
findings are closed, the sixth partly. Every finding below was fixed in the next commit.

## Major

1. Replays by the replay check, worker queries and the export cannot know the gated entries or `state.maxBytes`.
2. The SDK does not track the 4 MiB per-completion validation bound, so update results that together cross it review the
   run; the order in which results fail is unstated.
3. After `UNAVAILABLE` the lease expires uncounted with no backoff, and each cycle re-uploads and charges blobs.
4. Carried state must travel inline but has no inline budget.

## Nit

5. Which cancellation origins journal `run.cancel_requested`; "the last three" omits the request without grace.
6. The 16-blob limit scoped per lease in some places and per run in others; a test contradicts "the claim releases nothing".
7. The consumption command's body has no size bound.
8. `code_replay_checks` lacks the verifying subject and the expiry count; queued items lose expiries.
9. The register names the wrong chain lock function.
10. The gated-input limit is a per-activation sum, not per input.

Counts: Blocker 0, Major 4, Nit 6
