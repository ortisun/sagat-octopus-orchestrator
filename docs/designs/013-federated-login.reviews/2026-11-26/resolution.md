# Resolution of the reviews of design 013 (2026-11-26)

Settled by the author under Owner decision 2026-11-24-4, on the approval rule of Owner decision 2026-11-25-2.

## egm-review Stage 7, structural lens (`structural_report.md`, at `c0d2adcd`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: the cell names "the operator commands of Technical Plan 12" | Technical Plan 2 |
| 2 | Major | Fixed: level-4 headings, with new groups Members and tenant policy, Sign-out, Provider calls and identity secrets, Clients; no requirement renumbered | Requirements |
| 3 | Major | Fixed: six level-4 subsections | Technical Plan 11 |
| 4 | Nit | Fixed: a row for the three token-endpoint reasons | Technical Plan 11 |
| 5 | Nit | Fixed: Settings, Operator commands, Rollout and Rollback labelled | Technical Plan 12 |
| 6 | Nit | Fixed: the duplicated clause removed, the sentence capitalised, the stop-signing rule stated once | Technical Plan 4 |
| 7 | Nit | Fixed: the nine parts are level-4 headings; Verify is split into four paragraphs | Technical Plan 4 |
| 8 | Nit | Fixed in part: the column-0 line indented. Settled: the rewrap of the whole range is a mechanical change kept out of the approval commit so that the reviewed diff stays readable; it waits for the next amendment of the design | The feature range |
| 9 | Nit | Fixed: one entry per term | Terms |
| 10 | Nit | Fixed: Decisions 013-001 and 013-008 summarise their options and choice; 013-009, 013-019 and 013-020 are marked as records | Decisions |
| 11 | Nit | Fixed: the Overview cites Gap 013-014; Success criterion 013-001 cites Decision 013-020 | Overview, Success criteria |

## egm-review style lens (`style_report.md`, at `4ac4065d`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: the `studio` lifetimes stated for the Studio only; the CLI's stay in Requirement 013-026 | Requirement 013-018 |
| 2 | Major | Fixed: one session per browser; the refusal with `SESSION_CONFLICT` unless `replace_session=true` after the person agrees | Requirement 013-018 |
| 3 | Major | Fixed: the conditional rule stated once in Technical Plan 13; Requirement 013-021 refers to it | Requirement 013-021, Technical Plan 13 |
| 4 | Major | Fixed: separate sentences; "If Task 050-3 has not merged when Task 013-10 runs" | Requirement 013-032 |
| 5 | Major | Fixed: MUST NOT in all four requirements | Requirements 013-002, 013-005, 013-011, 013-021 |
| 6 | Major | Fixed in part: Technical Plan 4 and 11 are now split under level-4 headings (structural settlement). Settled: the requirements keep the repository's one-bullet form with their acceptance, and their sub-rules are restated one per paragraph in the Technical Plan they cite | Requirements |
| 7 | Major | Fixed: "Per-request check", "Maintenance loop" and "Attempt cookie" in Terms; "platform session" replaced by "sign-in session" | Terms, throughout |
| 8 | Major | Fixed: the attempt-sealing key is in Requirement 013-025's list; its lifecycle is stated once in Technical Plan 12 (no publication wait, at most two in force, the previous one kept 10 minutes for opening) | Requirement 013-025, Technical Plans 4 and 12 |
| 9 | Major | Fixed: up to two client secret references, and the client authentication method | Technical Plan 3 |
| 10 | Major | Fixed: each decision names its review (security or architecture review of 2026-11-25); "Stage N review" defined in Open Questions | Decisions 013-013, 013-014, 013-017, Open Questions |
| 11 | Nit | Fixed | Requirement 013-016 |
| 12 | Nit | Fixed | Gap 013-004, Technical Plan 11, Decision 013-009 |
| 13 | Nit | Fixed: the parenthetical deleted; CUJ 3 merged. Settled: "both equal the refresh interval" states the two settings' values and stays | Requirement 013-019, CUJ 3 |
| 14 | Nit | Fixed: the redirect sentence split, "evicted", "a ceiling of zero refuses every request", "more often than that interval" | Technical Plan 4, Requirements 013-009, 013-024, 013-031 |
| 15 | Nit | Fixed: Open Questions as in design 050, "Start-up refuses", "the `remove` command refusing" | Open Questions, Technical Plan 12 |
