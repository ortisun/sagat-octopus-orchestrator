# EGM stage 8 of the approval run of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 882e56fcd513fa4d0de4205b95532113522f223a
Reviewer: egm-review (stage 8)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Note: a fresh `egm:egm-zoom-out` agent, after stages 5 to 7, given their report `egm-stages-5-7-approval.md`. Recorded condensed.

## Merge

- Major: C-1 with T-3: define "the bound" as SC-041-007's completion p99 within 10% and drop the self-referential wording.
- Major: K-1: restate the Goal (every logging step's completion also waits up to 2 s for its final flush, likely through `RETRY_LATER` with 2 tickets) and add admission to T041-31; no reserved capacity.
- Major: T-1: name the answers that short-circuit (`LEASE`, `SESSION`, `CAPTURE_OFF`); the rest are deferred.
- Major: T-2: "none beyond the two accessors of the ADR-0027 Amendment".
- Nit: K-2 (downgraded): the delete route answers 204 with no count.
- Nit: Z-4: answer `RETRY_LATER` after the finalizer's raise instead of a second transaction.
- C-2, C-3, S-1, S-2 and the Nits: text fixes.

## Merge partly

- Nit: Z-3 (downgraded): drop the review tags in the text and move tool versions into T041-3; keep the file:line anchors.

## Reject

- Major (process): Z-1 pre-empts O041-020, which the owner left conditional on T041-31.
- Major: Z-2 as written contradicts OD-2026-11-15-9 (no root-only limit).

## Missed

- Major: key the script marker on the execution (run, node, attempt, execution id), which is already one per (run, node, attempt), and drop the runner fields, the resend fallback, the verification and `skipped_identity`; this also answers C-3's second half.
- Nit: `SAGAT_STEP_LOGS_READ_ENABLED` is a switch no owner decision asks for.
- Nit: the Draft status beside OD-2026-11-15-17.

Counts: Blocker 0, Major 8, Nit 5
