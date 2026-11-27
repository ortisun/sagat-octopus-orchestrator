Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: f6fef1d6c9277d7f88b5b159177b0cbd7af5decd
Reviewer: egm-structural (egm-review stage 7)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 3, Nit 6

Sections and order match the template; requirements numbered without gaps.

1. **Major.** Three alternatives are dismissed only by pointing to Proposed architecture decisions; give each a reason in the doc, one shape per bullet.
2. **Major.** Open Questions says none while the Principle V row waits on an undefined "coordinating session"; the Result cell carries extra text.
3. **Major.** Risks omits scalability limits (proactive pass load, credential bounds, hub growth) and failure modes stated elsewhere (forward-only rollback, revocation unknown, `PENDING` after a crash, one redirect URI to register everywhere).
4. **Nit.** "### Journey N:" departs from the template's "### CUJ N:" (the abbreviation is refused by the repository's words-in-full rule; the author keeps "Journey").
5. **Nit.** "not applicable" versus the template's "n/a" (the same rule; the checker reads only the row prefix).
6. **Nit.** Bare numbers in Non-Functional Requirements.
7. **Nit.** Non-functional requirements lack *Acceptance:* clauses; Requirement 047-024 has no keyword.
8. **Nit.** The diagram reuses `CP` as a node and a subgraph id.
9. **Nit.** Lines past 120 columns.
