Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: f6fef1d6c9277d7f88b5b159177b0cbd7af5decd
Reviewer: egm-style (egm-review stages 7 and 9)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 6, Nit 7

1. **Major.** Bare numbers "for 047-001" in Non-Functional Requirements collide with Requirement numbers.
2. **Major.** "The coordinating session" is undefined; the Principle V condition conflicts with "None blocks the design".
3. **Major.** The rollback sentence is too long and contradicts itself; split it.
4. **Major.** Journey 1 records the capability on a namespace; the registry is per deployment.
5. **Major.** Four token time limits with no stated relation; a table.
6. **Major.** Requirement 047-024 has no requirement keyword.
7. **Nit.** Bare feature and section numbers ("021's", "Technical Plan 10").
8. **Nit.** Undefined abbreviations: CI, IR, SaaS, auth, CUJs, protocol minor.
9. **Nit.** Subjective or informal phrases ("one click away", "in a minute", "without surprises", "the runs carry on", "says so instead of hiding it", "with what exists").
10. **Nit.** Three overpacked sentences (Overview, Requirement 047-014, the checks of Technical Plan 5).
11. **Nit.** "Never in a test run." has no subject.
12. **Nit.** "gives `RESOURCE_TYPE` its meaning" is vague.
13. **Nit.** `Refs:` of Tasks T047-17 and T047-20 omit the non-functional requirements their tests prove.
