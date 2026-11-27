# egm-review, Stage 7 (structural lens) of design 050 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: c0d2adcd
Reviewer: egm-review (Stage 7, egm-structural)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author to one paragraph per finding, every severity, location and fix kept. The reviewer found the template's sections in order, nesting without skips, the Constitution Check rows equal to the template and every table well formed.
Counts: Blocker 0, Major 2, Nit 6

1. **Major. Technical Plan 5, "Iterations".** One list item of about 43 lines renders as one paragraph, the bold "The source." mid-sentence, mixing the outcome rule, the counts and route, the storage and mapping, and the drawing. Fix: labelled paragraphs separated by blank lines, or level-4 headings.
2. **Major. Technical Plan 6, "Server, approvals." (and less so "Server, publication requests.").** About 46 lines in one paragraph hold the domain rule, the port, the principal, the order inside `decide`, the `409` change, paging, access filters, the per-run list, the run-end probe, the index and the items. Fix: labelled paragraphs.
3. **Nit. Technical Plans 3 to 8.** Three devices mark subsections (run-in labels on paragraphs, on bullets, none). Fix: one device, preferably level-4 headings; label the unlabelled paragraphs.
4. **Nit. The feature range.** Patched lines break the 120-column wrap. Fix: rewrap in a change that touches nothing else.
5. **Nit. Terms.** "Admitted iteration" also defines compensation and reconciliation; "Test terms" defines two. Fix: one entry per term.
6. **Nit. Decision 050-004.** Only a pointer, where every other entry gives options, choice and reason. Fix: a one-line summary.
7. **Nit. Overview, the link to Architecture decision 0101.** An absolute link to a branch that will be deleted when #267 merges. Fix: a relative link after #267, recorded in Task 050-1.
8. **Nit. Overview and Technical Plan 3.** The reviews link names the directory but targets one file; "0082" alone is the link text. Fix: link the directory; the whole phrase as link text.
