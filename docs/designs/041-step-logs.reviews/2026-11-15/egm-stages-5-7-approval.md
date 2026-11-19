# EGM stages 5 to 7 of the approval run of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 882e56fcd513fa4d0de4205b95532113522f223a
Reviewer: egm-review (stages 5, 6 and 7)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Note: fresh agents of the registered types `egm:egm-zoom-out` (stage 5, lines 1-1486), `egm:egm-comprehension` (stage 6, both ranges), `egm:egm-design`, `egm:egm-structural` and `egm:egm-style` (stage 7, lines 1-1109), dispatched directly and in parallel with stage 9 and the repository reviewers; their findings are recorded here condensed. Stage 8 (`egm-stage-8-approval.md`) read this file.

## Stage 5, zoom-out (Blocker 0, Major 3, Nit 1)

- Z-1 Major: the exact run ceiling (`run_log_usage`) adds a table, a lock-order step, G041-009, the fan-out spike and O041-020; recommend the unlocked sum (O041-020 (b)) as the starting design.
- Z-2 Major: the script marker's instance-identity mechanism (runner report fields, resend fallback, verification) costs more than it buys while `script_executions` keeps one record per (run, node, attempt); use the root-step rule only.
- Z-3 Major: the Technical Plan pins code-level detail (tool versions, private line numbers, SQL shapes, SDK signatures) that an approved design cannot change without re-review; review-trail citations belong in resolution.md.
- Z-4 Nit: the second transaction after the finalizer's raise only attributes drops.

## Stage 6, comprehension (Blocker 0, Major 4, Nit 13)

- C-1 Major: the completion-wait "bound" is never stated as a number (lines 19-20, 231, 268, 640-647).
- C-2 Major: how "ended without its final flush" is computed is not stated.
- C-3 Major: "stream" has several meanings and a script marker's stream id is in no listing.
- C-4 Major: contracts.md says `expiresAt` is "at the retention in force now", against the frozen retention.
- Nits: the Draft status beside OD-2026-11-15-17; D1958 not introduced; G041-005's "below 1.6"; gap range G041-014 against 015; research G041-008 still says "row id"; the "go on to step 7" exception for step 2; "five quota resources" wording; undefined project terms; terms used before definition; rollout wording; CUJ 2's poll attribution; "negotiation is unchanged" against T041-7; the §10 reference.

## Stage 7, design critic (Blocker 0, Major 2, Nit 7)

- K-1 Major: ingest capacity is on every logging step's completion path (2 tickets per tenant per replica, `RETRY_LATER` before the final batch); no sizing argument; T041-31 does not model admission or the final-flush wait.
- K-2 Major: the delete route tells a caller without `run:logs:read` or the PII grant how many streams a run has, against ADR-0085 decision 6.
- Nits: the runtime role does hold `SELECT` on `tenant_retained_usage`; the single-caller test covers only some files; "goes on to step 7" should be "skips to"; records above `ceiling_sequence` must be refused; K against tickets × R; filtered reads have no per-page scan bound; `safelog.go` path.

## Stage 7, structural (Blocker 0, Major 2, Nit 9)

- S-1 Major: Constitution Check rows II and VIII use a result outside the template ("Pass, to be verified").
- S-2 Major: principle V's condition G041-008 has no Complexity Tracking row; G041-015 is missing from V and Risks.
- Nits: status against OD-2026-11-15-17; answered decisions under "needed"; Alternatives format; CUJ 5 without a story and no CUJ for the marker; Goals miss the Studio; NFR id style; gap range; long lines; resolution.md not a link.

## Stage 7, style (Blocker 0, Major 3, Nit 17)

- T-1 Major: step 2's control flow contradicts "goes on to step 7".
- T-2 Major: "no `SECURITY DEFINER` function" (lines 671, 1097) against the two accessors.
- T-3 Major: the bound is defined by its own measurement; no pass threshold.
- Nits: "MUST be sent"; markers "hold no text" against labels; the five-resources list; "stream" for three things; forward references; saturation wording; poll attribution; "high mark" terms; K against R; "remaining calls"; reason against verdict; `SESSION` resend clause; two "fixed" quantities; review tags in the text; jargon; rollout sentence; long lines.

Counts: Blocker 0, Major 14, Nit 47
