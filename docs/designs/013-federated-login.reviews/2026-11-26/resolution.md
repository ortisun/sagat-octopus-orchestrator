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

## egm-review Stage 8, Intermediate Zoom Out (`intermediate_zoom_out_report.md`, at `88b3427f`)

All ten findings are accepted and fixed. No security property is weakened. Each fix keeps the guarantee and removes a
mechanism: `identity_revision`, the transactional revocation of a raised context, the `SEALING` key kind and its
command group, a capability name, an engine setting, most of the `break` switch and the silence alert.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: the subject rule and the tenant restriction are immutable with the issuer (Decision 013-014 extended). Only the sign-in path is staged, activation revokes nothing, and `identity_revision` with its predicate and batch branch is removed | Requirements 013-001, 013-019, Technical Plans 3, 4 and 6, Decision 013-014, Task 013-4 |
| 2 | Major | Fixed: the per-request check refuses a session whose recorded context does not meet the current requirement. Raising the context writes the connection row only, and the maintenance loop marks those sessions later | Requirements 013-010, 013-019, Technical Plans 4 and 13, Task 013-20 |
| 3 | Major | Fixed: every task belongs to one milestone. New Tasks 013-19 (B) and 013-20 (D) are added, and Task 013-17 holds all of C. Release 0.3.0 is A, B and D, and C ships when 029's migration lands. The required context moves to D, and each milestone's journey covers what it adds | Decision 013-020, Success criterion 013-001, the plan |
| 4 | Major | Fixed: the attempt is sealed by the existing `KeyRing`, whose ring is one entry of the identity mount, with `identity attempt-key rotate`. The `SEALING` kind and the sealing-key command group are removed | Technical Plans 3, 4 and 12, Requirement 013-025, Task 013-8 |
| 5 | Nit | Fixed: the session bound applies to blob uploads only | Requirement 013-018, Task 013-9 |
| 6 | Nit | Fixed: `identity.federation` only, and `config.json`'s platform issuer comes from the same mapping | Requirement 013-032, Technical Plan 10, Task 013-10 |
| 7 | Nit | Fixed: a 0.3.0 engine lists `identity-federation-v1` always, and only the control plane reads the setting | Requirement 013-032, Technical Plan 12 |
| 8 | Nit | Fixed: `break` covers `alg`, `nonce` and `acr`, and the other rules are domain unit tests | Requirement 013-007, Technical Plan 9, Tasks 013-3, 013-4 |
| 9 | Nit | Fixed: only the sign-in path is staged. The provisioning policy and the default role apply directly, audited, with the recomputation, and `a_default_role_change_applies_without_a_test_sign_in` is added | Requirement 013-001, Task 013-12 |
| 10 | Nit | Fixed: `IDENTITY_SCIM_FAILING` fires after ten consecutive refused requests, convergence is stated at the provider's next change of the resource, and the fixtures show which provider sends `If-Match` | Technical Plan 6, Risks, Constitution Check VII, Task 013-13 |

## egm-review Stage 9, codebase details lens (`codebase_details_report.md`, at `077e1473`)

All 22 findings are accepted and fixed; both Blockers are fixed.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Blocker | Fixed: `step_log_usage_if_revealed` is the third plain read, moved and routed through `sagat_active_membership`, with its own refusal test | Technical Plan 13, Task 013-5 |
| 2 | Blocker | Fixed: the opening window polls `GET /v1/auth/links/{attempt}` (bearer, every 2 seconds), and the link page only closes. The Studio's opener policy stays, with the reason recorded against `studio.rs`. The route is in the route table | Technical Plans 4 and 11 |
| 3 | Major | Fixed (the second option): no `AlertKind`. The administrator sees `scimFailing`, `credentialExpiresAt` and the disabled owner's member count on the connection. The operator gets five Prometheus rules over closed-label metrics, with runbook sections and the rule test. Break-glass is an audit record plus a counter | Requirement 013-001, 013-021, Technical Plans 4, 6, 11 and 13, Tasks 013-4, 013-13 |
| 4 | Major | Fixed: `CountingStore` counts `resolve_caller`, `FakeStore` gains it, and the throttling test is in the verification | Task 013-10 |
| 5 | Major | Fixed: `usable_key` keeps PS256, with an explicit rule for RSA keys without `alg` and a unit test. The deployment verifier's list is unchanged | Task 013-6 |
| 6 | Major | Fixed: the manifest and the ALWAYS triggers are named, along with the two targets with the runtime role. Members are charged to `TenantContentBytes`, and `QUOTA_EXCEEDED` is the answer | Technical Plan 6, Task 013-5 |
| 7 | Major | Fixed: after the latest migration when the task starts, with a validate migration | Task 013-5 |
| 8 | Major | Fixed: the image test is new, built from the Dockerfile's `control-plane` target, in the images job | Task 013-3, Technical Plan 9, Requirement 013-033 |
| 9 | Major | Fixed: `fault_platform_foundation` and `contract_platform_foundation` in the verification | Task 013-6 |
| 10 | Major | Fixed: only `worker dev` refuses a key-store token. `doctor` uses it for the control plane | Task 013-15 |
| 11 to 22 | Nit | Fixed: `config.json` is read per request with `scope`; `iat` is in `VerifiedCaller`; the fetcher and the re-exports are listed; the filters and the battery's target are named; `Slot::IdentityAdvisory` is added; the organization comes from `--org`, `SAGAT_ORG` or the login default; per-route body limits are set; Architecture decision 0101 is cited with #267; the reuse is named; `ON DELETE SET NULL (owning_connection_id)`; a validate migration is added; feature 029's contract is amended and the 2-second `lock_timeout` stated | Technical Plans 3 to 13, Tasks, `specs/029-custom-roles-folders/contracts/group-source.md` |

## Security re-read, round 1 (`approval/reread-1-security.md`, at `db1bea08`)

All nine findings are fixed; a security finding is never accepted. The Stage 8 resolution's sentence "no security
property is weakened" was wrong for findings 2 to 5; each is restored below.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: the poll answers only the attempt's tenant and subject, otherwise `404`; state and identity only, `no-store`; in the battery; `a_link_poll_by_another_subject_is_not_found` | Technical Plans 4 and 11, Task 013-9 |
| 2 | Major | Fixed: a raise records a context floor and its time, and the per-request check refuses sessions signed in before it without meeting it; a disable records `sessions_revoked_before`; `lowering_the_context_revives_no_refused_session`, `a_reenabled_connection_revives_no_session` | Requirements 013-010, 013-019, Technical Plans 3 and 4, Tasks 013-10, 013-20 |
| 3 | Major | Fixed: within 10 minutes of a suspension, a request without `If-Match` that would reactivate is refused with SCIM `409` and audited; `a_late_reactivation_after_a_deactivation_is_refused` | Technical Plan 6, Task 013-11 |
| 4 | Major | Fixed: `breakGlassLastUsedAt` and the 30-day count on `GET /identity/settings` and the identity screens; `a_break_glass_use_shows_on_the_identity_settings` | Requirement 013-021, Technical Plan 11, Task 013-20 |
| 5 | Major | Fixed: one table-driven integration test breaks every claim rule through the callback with the development provider's `claims` override | Requirement 013-007, Technical Plan 9, Tasks 013-3, 013-9 |
| 6 | Nit | Fixed: the poll needs the bearer and `X-Sagat-Renew`, not `Origin` | Technical Plans 4 and 11 |
| 7 | Nit | Fixed: a trigger refuses a change of the immutable columns, checked by `runtime_privileges` | Technical Plan 13 |
| 8 | Nit | Fixed: `config.json` is built from the process's in-memory view of the registry | Technical Plan 7 |
| 9 | Nit | Fixed: Architecture decision 0140 decision 9 says the engine lists the capability always | Architecture decision 0140 |
