# egm-review, style lens (Stages 7 and 9) of design 013 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 4ac4065d
Reviewer: egm-review (Stages 7 and 9, egm-style)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author to one paragraph per finding, every severity, location and fix kept. Wrapping was excluded from the brief as a known, settled finding.
Counts: Blocker 0, Major 10, Nit 5

1. **Major. Requirement 013-018 against 013-026.** The Studio's lifetimes read as every redemption's, so the CLI's seven days break "at most 24". Fix: per-client lifetimes.
2. **Major. Requirement 013-018.** "Signs in to another ends the first" reads as automatic, then the next clause refuses it. Fix: state the refusal and the explicit replacement.
3. **Major. Technical Plan 13, Requirement 013-021.** The last-administrator lock is stated unconditionally, then conditionally with a restart. Fix: the conditional rule once; the requirement refers to it.
4. **Major. Requirement 013-032.** One sentence carries activation, refusals, derivation, field ownership and a conditional task; "when it has not merged Task 013-10" misparses. Fix: separate sentences.
5. **Major. Requirements 013-002, 013-005, 013-011, 013-021.** Negated MAY in security prohibitions. Fix: MUST NOT.
6. **Major. Requirements 013-001, 013-018, 013-019, 013-021, 013-025, 013-026; Technical Plan 4 "Lock order".** Many rules per sentence or parenthetical. Fix: a lead sentence and numbered sub-rules.
7. **Major. Throughout.** Several names each for the revocation check, the background work, the attempt cookie and the sign-in session. Fix: terms "Per-request check", "Maintenance loop", "attempt cookie", "sign-in session".
8. **Major. Technical Plans 3 and 4, Requirement 013-025, the operator commands.** The sealing key is missing from the secrets list, lives in a "signing keys" table, and has no stated lifecycle. Fix: list it and state its lifecycle once.
9. **Major. Technical Plan 3, `identity_connections`.** Two client secret references and then "the client secret reference" (singular). Fix: one cardinality.
10. **Major. Decisions 013-012 to 013-017.** "The same review" resolves to different reviews; "Stage 5 review" undefined. Fix: name each review; define the stage reviews once.
11. **Nit. Requirement 013-016.** A double negative. Fix: rewrite.
12. **Nit. Gap 013-004, Technical Plan 11, Decision 013-009.** "IdP-initiated", "T029-015" in prose, "Owner question 1440 and 1442". Fix: words in full.
13. **Nit. Requirement 013-019, CUJ 3, Technical Plan 4.** Redundant phrases. Fix: delete or merge.
14. **Nit. Technical Plan 4, Requirements 013-009, 013-024, 013-031.** Clauses that do not say what they mean ("always sends ... :", "expired first", "zero refuses", "more often"). Fix: rewrite.
15. **Nit. Open Questions, Rollback, operator commands.** Hidden actors. Fix: name them.
