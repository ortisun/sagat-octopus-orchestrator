Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cfb22df54df5b084f748531dbdb9b3546c01f60d
Reviewer: egm-style (egm-review stages 7 and 9)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 7, Nit 8

1. **Major.** Prose mixes task forms ("Task T046-12"); the new form in prose is "Task 046-12"; only the checklist lines keep `T046-1`.
2. **Major.** Prose and link text use "ADR-0120"; new prose writes "Architecture decision 0120" (link targets exempt).
3. **Major.** "for 046-001; for 046-002" in Non-Functional Requirements collides with Requirement numbers; write "Non-functional requirement 046-001".
4. **Major.** "CUJ", "CUJs", "CUJ 1" are abbreviations; write "Critical user journey 1".
5. **Major.** Task T046-21 cites Requirement 046-018 (should be Non-functional requirement 046-005); Task T046-23 cites Requirement 046-001 (should be Non-functional requirement 046-003).
6. **Major.** Several sentences of 50 to 90 words chained with semicolons (Overview, Requirements 046-004 and 046-010, the verdict sentence of Technical Plan 6, Technical Plans 11 and 12). Split; turn the refusal conditions and the lock rules into lists.
7. **Major.** "This removes dependency confusion by construction" is absolute, and an unscoped private name not yet uploaded is still served from the public registry. Narrow the claim.
8. **Nit.** "LRU" and "n/a"; write them in full.
9. **Nit.** SBOM, OSV, CI, CVSS and SPDX are not expanded on first use.
10. **Nit.** "MUST never run" is not a requirement keyword; write "MUST NOT run".
11. **Nit.** "before 023's check": write "feature 023's".
12. **Nit.** The quoted "delegate (OD-2026-11-24-4)" beside the new form of the same decision.
13. **Nit.** "under the version rows' locks" reuses the defined term "lock"; write "row locks".
14. **Nit.** Four names for bundle version 5; use one.
15. **Nit.** Vague or figurative phrases ("cited by numeral", "two most used languages", "stronger guarantees than Windmill", "a second wall", "unchanged in substance", "hostile battery" before its definition).
