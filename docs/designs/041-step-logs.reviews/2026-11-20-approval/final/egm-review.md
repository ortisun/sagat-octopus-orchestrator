# egm-review of design 041's revision on the owner's capped approval path (summary)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 543417134ff7ebda70a96690329bb95ddde55c8c
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.329
Note: the author's summary for the approval record (docs/designs/constitution-4.0.0-egm.md, section 3). The complete run of the five egm-review stages, in order, is the first round of the approval run, on `ff8033cd` (the stage reports beside this folder: `../zoom_out_report.md`, `../comprehension_report.md`, `../critic_report.md`, `../intermediate_zoom_out_report.md`, `../implementation_readiness_report.md`; the triage `../consolidated_review_report.md`). Under the owner's path "Full review, capped" (OD-2026-11-20-17) its findings were answered (`../resolution.md`), then re-read in two short rounds (`../round-2/`, at `4d6fe4cc`; `../round-3/`, at `c52b5d37`), and the last fixes were read once more at this commit by the same stage agents: `design.md` (egm-design, standing in for stages 5 to 8) and `readiness.md` (egm implementation readiness), both in this folder, verbatim. This summary carries what those two final reads leave open, plus the first round's Major Z-2, accepted as recommended in the first round and never re-raised. Each Major below is settled by one owner-decision row citing `#<n>`, OD-2026-11-21-200 to OD-2026-11-21-203, each an application of OD-2026-11-20-17 (a Major left after the capped rounds is accepted as recommended; a Blocker is only fixed). Finding Z-3 is closed by the owner's answer OD-2026-11-21-2 and finding I-6 by OD-2026-11-21-3; neither is counted.

| Final read | Blocker | Major | Nit |
| --- | ---: | ---: | ---: |
| stages 5 to 8 (`design.md`) | 0 | 3 | 7 |
| readiness (`readiness.md`) | 0 | 0 | 8 |
| first round, carried (Z-2) | 0 | 1 | 0 |

## The Majors, numbered

1. design final read, finding 1: the owner report on the p99 compares the median of all valid rounds against the largest of the three rounds used (+36.9% at 1 × 16, +9.5% at 8 × 128), so the accepted build would trigger it about half the time. Disposition: accepted as is under the cap (the report does not stop the release; the threshold is to be derived from the eight valid rounds when task T041-9 records them); owner row OD-2026-11-21-200.
2. design final read, finding 2: the pooler's server-pool sizing rule leaves little room for multiplexing, contradicts `docs/runbooks/scaling.md`'s reason for a pooler, and changes what the G041-027 regime of task T041-37 measures (128 server backends instead of 40). Disposition: accepted as is under the cap, the residual put to the owner with the record; owner row OD-2026-11-21-201.
3. design final read, finding 3 (first round D-12): the revision notes in the Overview, kept as the amendment rule requires. Disposition: accepted as recommended; owner row OD-2026-11-21-202.
4. first round, stage 5, Z-2 (`../zoom_out_report.md`): the exact per-writer quota leases kept over a simpler unlocked ceiling with a proved overshoot (ADR-0097 option 4). Disposition: accepted as recommended in the first round (the leases kept, OD-2026-11-20-15; Complexity Tracking names the rejected alternative); owner row OD-2026-11-21-203.

The Nits of both final reads stay open, recorded in `../resolution.md`, section "Final read", with the task that settles each.

Counts: Blocker 0, Major 4, Nit 15
