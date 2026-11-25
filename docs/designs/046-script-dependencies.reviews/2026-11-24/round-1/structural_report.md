Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cfb22df54df5b084f748531dbdb9b3546c01f60d
Reviewer: egm-structural (egm-review stage 7)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 3, Nit 10

Scope: lines 1 to 662. All sections present in template order; the mermaid fence is closed; tables consistent; Constitution Check rows start as required.

1. **Major.** Requirement identifiers (lines 84 to 197) use the long forms ("Requirement 046-001") while the template comment prescribes `FR-NNN-xxx`; `Refs:` lines are protected after approval. Fix: adopt the old forms, or record that the checker accepts the long form.
2. **Major.** Owner decisions needed says "None" although the read-only mount breaks ADR-0013's "nothing mounted", a security exception. Fix: add a line for it or state that Owner decision 2026-11-24-4 covers it, citing the research decision.
3. **Major.** Risks and Mitigations omits the gateway as a single point of failure for publication, a compromised gateway or poisoning of its shared public store, and the cost of the re-scan across tenants. Fix: add a row each.
4. **Nit.** User story identifiers use two digits.
5. **Nit.** Owner decisions are written both as "Owner decision 2026-11-24-1" and `OD-2026-11-24-4`.
6. **Nit.** Contract paths are code spans, not links, in two forms.
7. **Nit.** The volume-name code span breaks across a line.
8. **Nit.** `data-model.md` is said to be written with Task T046-2 but Task T046-1 lists it.
9. **Nit.** "Pass with conditions (Complexity Tracking)" carries extra text in the Result cell.
10. **Nit.** The rejection of a third-party repository manager gives no reason in the doc.
11. **Nit.** Tests cited by acceptance criteria are not named in the Verification Plan.
12. **Nit.** Windmill appears without introduction.
13. **Nit.** Some lines exceed 120 columns.
