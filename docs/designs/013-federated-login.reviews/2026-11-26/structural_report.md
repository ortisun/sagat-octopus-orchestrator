# egm-review, Stage 7 (structural lens) of design 013 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: c0d2adcd
Reviewer: egm-review (Stage 7, egm-structural)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author to one paragraph per finding, every severity, location and fix kept. The reviewer found the template's sections in order, the Constitution Check rows equal to the template, the diagram closed and every relative link resolving.
Counts: Blocker 0, Major 3, Nit 8

1. **Major. Technical Plan 2, the Placement table.** An unescaped pipe in `create|rotate` splits the cell, so `apps/control-plane` is lost and the code span breaks; the cell also lists fewer commands than Technical Plan 12. Fix: escape or reword, and point to Technical Plan 12.
2. **Major. Requirements, the bold group labels.** "Sessions and tokens" covers 013-018 to 013-027, of which only three are about sessions; the labels have no anchors. Fix: level-4 headings and new groups without renumbering.
3. **Major. Technical Plan 11.** The route tables and the reasons, token endpoint, SCIM, battery and alerts hang under two bullets they do not belong to; no blank line after the last route row. Fix: level-4 subsections (Routes, Errors, SCIM, Cross-tenant battery and access classes, Alerts, Documents and CI).
4. **Nit. Technical Plan 11.** The table claims every reason but omits the three of the token endpoint. Fix: a row, or say so.
5. **Nit. Technical Plan 12.** The rollout steps lost their label; the earlier resolution cites "Rollout". Fix: labels for operator commands, rollout and rollback.
6. **Nit. Technical Plan 4, Verify and Signing keys.** A duplicated "and the request path resolves", a sentence starting in lower case, and the stop-signing rule stated twice. Fix: delete, capitalise, merge.
7. **Nit. Technical Plan 4.** Nine run-in labels are cited by name as if they were anchors; Verify is one block of about 20 lines. Fix: level-4 headings; split Verify.
8. **Nit. The feature range.** Patched lines break the 120-column wrap; one line of a list item starts at column 0. Fix: rewrap; indent the line.
9. **Nit. Terms.** Several entries define two to four terms. Fix: one entry per term.
10. **Nit. Decisions 013-001, 013-008, 013-009, 013-019, 013-020.** Pointers or statements without options. Fix: a one-line summary, or mark them as records.
11. **Nit. Overview, Success criterion 013-001.** The Overview cites Technical Plan 6 for provider timing (it is Gap 013-014); milestones are used before Decision 013-020 defines them. Fix: both references.
