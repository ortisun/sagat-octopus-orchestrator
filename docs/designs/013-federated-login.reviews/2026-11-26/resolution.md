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

## Security re-read, round 2 (`approval/reread-2-security.md`, at `fd2aad8c`)

All seven findings are fixed.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: a raise is any change that removes an accepted value; one floor at a time, selected and cleared by the maintenance loop; another raise waits (retryable `CONNECTION_STATE`); `a_second_raise_waits_for_the_marking_of_the_first` | Requirement 013-010, Technical Plans 3, 4 and 6, Task 013-20 |
| 2 | Nit | Fixed: epochs, not times: `session_epoch` on the connection, copied onto the session under redemption's `FOR SHARE`, incremented by a disable or a raise under `FOR UPDATE`; the barrier test re-enables | Requirement 013-019, Technical Plans 3, 4 and 13 |
| 3 | Nit | Fixed: renewal, refresh and the stream and upload re-checks use the per-request check's predicate; `a_reenabled_connections_old_session_cannot_renew` | Requirement 013-019, Technical Plan 13 |
| 4 | Nit | Fixed: the poll also requires the binding cookie; `a_link_poll_without_the_binding_cookie_is_not_found` | Technical Plans 4 and 11, Task 013-9 |
| 5 | Nit | Fixed: the override sets only the tested names, `sub` only empty, 2 KiB, behind `SAGAT_DEV_IDP_TEST_OVERRIDES=1`; Architecture decision 0140 decision 8 amended; a test | Technical Plan 9, Task 013-3, Architecture decision 0140 |
| 6 | Nit | Fixed: the two staged-change rows added; the `acr` row moved to Task 013-20 | Requirement 013-007, Tasks 013-9, 013-20 |
| 7 | Nit | Fixed: `memberships.status_changed_at`; row VII names the exception | Technical Plans 3 and 6, Constitution Check |

## Security re-read, round 3 (`approval/reread-3-security.md`, at `66189ab7`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Nit | Fixed: with no requirement every value is accepted, so the first requirement is a raise; the lowering test starts from no requirement | Requirement 013-010 |
| 2 | Nit | Fixed: the renewal test is in Task 013-9, covering renewal and the CLI's refresh | Task 013-9 |

## Architecture re-read, round 1 (`approval/reread-1-architecture.md`, at `dabbc4bc`)

All eight findings are fixed.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: a signing handle is obtained before `BEGIN`, or the request is refused before any lock; signing inside is CPU work only; the refresh grant does the same; `signing_inside_a_transaction_does_no_io` | Technical Plan 4, Requirement 013-026 |
| 2 | Major | Fixed: `FederationStore::signing_keys()` (adapter in `infra-postgres`), a fetcher in `api-http/src/auth/issuers.rs` mapping to a `JwkSet`, the signer given the active version by the use case; placement rows added | Technical Plan 2 |
| 3 | Major | Fixed: Task 013-10 makes the registry member keep the requirement set (unless 022 or 029 did first), with `a_replica_started_after_an_activation_advertises_it` and `a_replica_started_after_an_activation_offers_the_platform_issuer` | Task 013-10 |
| 4 | Nit | Fixed: "needs no configuration for federation" | Requirement 013-032, Architecture decision 0140 |
| 5 | Nit | Fixed: "reset" removed; the two tests in "Enforced by" | Architecture decision 0140 |
| 6 | Nit | Fixed | Decision 013-017 |
| 7 | Nit | Fixed: the in-transaction check has no lag; the in-memory view drives the Studio's offer and the name; the runbook's wait scoped to it | Technical Plan 13 |
| 8 | Nit | Fixed: the addition covers SCIM `POST` and pre-provisioning, the registry epoch first, and the demotion restart rule | Architecture decision 0050 |

## Correctness re-read, round 1 (`approval/reread-1-correctness.md`, at `5d451eb9`)

All ten findings are fixed.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: `identity_role_recompute` rows and a per-connection job row with `rules_revision` and a cursor, written in the change's transaction, cleared by compare-and-set; two tests | Technical Plans 3, 4 and 6, Task 013-17 |
| 2 | Major | Fixed: the lock strength is chosen before the first lock, never upgraded; a restart rolls back; the barrier tests assert one success and one `LOCKOUT_GUARD`; Architecture decision 0050's addition says so | Technical Plan 13, Architecture decision 0050 |
| 3 | Major | Fixed: the cookie is read before the transaction, so a refusal leaves the code unspent; after expiry the Studio starts a new sign-in with the person's choice; the test redeems one code twice | Technical Plan 4 |
| 4 | Major | Fixed: the group change commits, the refused demotion is skipped, audited and counted on the connection, not retried until rules or groups change; `removing_the_last_admin_from_its_group_keeps_scim_working` | Technical Plan 6, Task 013-17 |
| 5 | Nit | Fixed: redemption, renewal and refresh check the handle's `kid` in the transaction and roll back | Technical Plan 4 |
| 6 | Nit | Fixed: the attempt key is added as an opening key first; a test | Technical Plan 12, Task 013-9 |
| 7 | Nit | Fixed: the grace covers a generation whose later generations are all unused; `two_lost_refresh_responses_keep_the_session` | Requirement 013-026, Task 013-9 |
| 8 | Nit | Fixed: a tombstone of the attempt id outlives the purged code | Requirement 013-009, Technical Plan 6 |
| 9 | Nit | Fixed: re-read under the shared fence, restart with the exclusive one; a barrier test | Technical Plan 4, Task 013-17 |
| 10 | Nit | Fixed: the provider's path after a `409` named, the refusal does not count toward `scimFailing`, the fixtures show each provider | Technical Plan 6 |

## egm-review re-read, round 1 (`approval/reread-1-egm-review.md`, at `9c893782`)

All eight findings are fixed.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: the confirmation takes the connection row, re-checks it, takes the advisory lock, then the membership; the path in `lock_order.rs`; a barrier test; Architecture decision 0050's addition | Technical Plan 4, Architecture decision 0050, Task 013-9 |
| 2 to 8 | Nit | Fixed: the two tests in Task 013-9; `signin_refresh_generations` and the code row kept with its identity cleared; `identity_connection_health` written outside the request; the gauge `identity_connections_scim_failing`; a guarded outcome for recomputation callers; the lock order reworded; the compare-and-set rules stated | Technical Plans 3, 4, 6, 11 and 13 |

## Security re-read, round 4 (`approval/reread-4-security.md`, at `2d71be9c`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: promotions before demotions; a guarded row recorded durably and retried every pass; two tests | Technical Plan 6, Task 013-17 |
| 2 | Major | Fixed: the refresh token is `<sid>.<generation>.<32 random bytes>`, opaque and unsigned; a lower generation outside the grace revokes however old; step 6 signs only the access token; two tests | Requirement 013-026, Technical Plans 3 and 4, Task 013-9 |
| 3 | Nit | Fixed: client, redirect URI and PKCE are checked before the cookie comparison, every mismatch `invalid_grant` | Technical Plan 4 |
| 4 | Nit | Fixed: the restart rule and the test's name added to the contract | `specs/029-custom-roles-folders/contracts/group-source.md` |

The deadlock the reviewer noted outside its lens is fixed too: the link and test confirmation takes the advisory lock
first, then the connection row, as redemption does, and Architecture decision 0050's addition says so.

## Security re-read, round 5 (`approval/reread-5-security.md`, at `86b9f140`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: every generation's hash is kept until the session ends; every path, the grace included, checks the presented secret first, and a failure is `invalid_grant` with no effect and the same answer whether the session exists; `a_forged_lower_generation_does_not_revoke_the_session`, `a_forged_token_within_the_grace_gets_no_successor` | Requirement 013-026, Technical Plan 3, Task 013-9 |

## Security re-read, round 6 (`approval/reread-6-security.md`, at `10338cd5`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed (option b, with a rate limit as well): the token is `<sid>.<generation>.<key id>.<mac>`, an HMAC-SHA256 under the `cli-refresh` purpose key of the session id, the generation and a random value on the session; the server recomputes any generation's MAC, so it keeps no per-generation secret and the grace rows live 30 seconds; at most six refreshes a minute per session, a refusal leaving the token unspent; two tests | Requirement 013-026, Technical Plans 3 and 4, Task 013-9 |

## Security re-read, round 7 (`approval/reread-7-security.md`, at `4dea6eaa`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: each refresh deletes the session's grace rows older than 30 seconds, the loop purges those of ended sessions, the grace is decided by time, and the test asserts at most seven rows | Technical Plans 3 and 6, Task 013-9 |
| 2 | Major | Fixed: `KeyRing::derive_purpose_key(key_id, label, info)` keeps bytes by id; tokens verify under their own key, successors mint under the active key, an unknown id is `invalid_grant`, a key retires after 7 days; two tests | Requirement 013-026, Task 013-9 |
| 3 | Nit | Fixed: MAC first in constant time, then the class, then the limit; the window on `signin_sessions`; a test | Requirement 013-026, Technical Plan 3 |
| 4 | Nit | Fixed: the highest issued generation plus one, never reused; a valid MAC above the current generation is `invalid_grant`; a restore rotates the random values | Requirement 013-026 |
| 5 | Nit | Fixed: the row holds the random value and the generations | Technical Plan 3 |
| 6 | Nit | Fixed: `Mac::verify_slice`; `a_forged_refresh_token_cannot_sign_out` | Requirement 013-026, Task 013-9 |

## Security re-read, round 8 (`approval/reread-8-security.md`, at `9a19411f`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: a dedicated `cli-refresh` ring in the identity mount, never `SAGAT_SECRETS_KEYS` (an architecture rule refuses the label there); two-step rotation by `identity cli-refresh-key rotate`, which keeps the old key 7 days and one interval; a test of the command; the `KeyRing` change owned by whichever of Tasks 050-4 and 013-9 runs first | Requirement 013-026, Technical Plan 12, Task 013-9 |
| 2 | Nit | Fixed: the bound stated as 12 for the fixed window; the test bursts across a boundary | Requirement 013-026, Task 013-9 |
| 3 | Nit | Fixed: a lower generation with no row is outside the grace and its reuse revokes; the test includes a deleting refresh | Technical Plan 3, Task 013-9 |

## Security re-read, round 9 (`approval/reread-9-security.md`, at `dcfd91b1`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: Task 013-8 builds `IdentityKeyRing`, the `cli-refresh` commands, the `KeyRing` derivation (unless Task 050-4 landed it) and the rule with its fixture; Task 013-9 uses the ring for the MAC; four rules in Technical Plan 2; the architecture check in the verification | Tasks 013-8, 013-9, Technical Plan 2 |
| 2 | Nit | Fixed: `IdentityKeyRing` wraps `KeyRing`, built only by the identity-mount loader; the refresh code takes that type; the label is confined to its module | Requirement 013-026, Technical Plan 2 |
| 3 | Nit | Fixed: `rotate` on an empty ring creates an active key; rollout runs it; start-up refuses an empty ring and never falls back | Requirement 013-026, Technical Plan 12 |
| 4 | Nit | Fixed: `identity cli-refresh-key revoke` replaces the key and rotates every CLI session's random value; the Risks row names it | Requirement 013-026, Risks |

The test of the rotation is renamed `a_cli_refresh_key_rotation_keeps_a_session_refreshing_across_it`, since a CLI
session idle for a day expires (the reviewer's note for correctness).

## Security re-read, round 10 (`approval/reread-10-security.md`, at `db9e8a50`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Nit | Fixed: `IdentityKeyRing` lives in `crates/infra-secrets`; the identity-mount loader in the same module is its only constructor; no public constructor, no `From<KeyRing>` | Requirement 013-026 |
| 2 | Nit | Fixed: revoke drops every key it did not create and cancels an unfinished rotation; the test starts mid-rotation | Requirement 013-026, Task 013-8 |
| 3 | Nit | Fixed: Task 013-8 owns the derivation; the commands paragraph names `rotate|revoke`; Task 013-8's Refs include Requirement 013-026 | Requirement 013-026, Technical Plan 12, Task 013-8 |

## Security re-read, round 11 (`approval/reread-11-security.md`, at `0aa18b84`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Nit | Fixed: every write of the ring is a KV version 2 check-and-set; `rotate`'s second step promotes only from its own version, otherwise stops with an audited "rotation cancelled"; `a_rotation_resumed_after_a_revoke_writes_nothing` | Requirement 013-026, Task 013-8 |

## Security re-read, round 12 (`approval/reread-12-security.md`, at `9a760bd7`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Nit | Fixed: `revoke` must win: on a failed check-and-set it re-reads and writes again up to five times, then fails audited; it rotates the random values only after its write commits; `a_revoke_racing_a_rotation_promotion_still_replaces_every_key` | Requirement 013-026, Task 013-8 |

## Architecture re-read, round 2 (`approval/reread-2-architecture.md`, at `239577a1`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: ports `RefreshTokenMac` and `AttemptSealer` in the application, adapters in `infra-secrets`; the use case keeps the order; placement rows | Technical Plans 2 and 4, Task 013-9 |
| 2 | Major | Fixed: Architecture decision 0140 decision 11, with the rejected options and the consequences; "Enforced by"; a Complexity Tracking row | Architecture decision 0140, Complexity Tracking |
| 3 | Major | Fixed: the control plane starts with an empty ring and refuses only CLI redemption and refresh, as a missing signing key degrades; the rollout creates the keys before enabling federation; the test renamed | Requirement 013-026, Technical Plan 12 |
| 4 | Nit | Fixed | Requirement 013-025 |
| 5 | Nit | Fixed: read at start and on refresh, kept on a failed read; after a revoke a stale replica mints at most one interval | Requirement 013-026 |
| 6 | Nit | Fixed: bounded batches tenant by tenant; a failure after the ring write is safe; a re-run finishes | Requirement 013-026 |
| 7 | Nit | Fixed: Architecture decision 0050 says `FOR UPDATE` for an administrator target and `FOR SHARE` otherwise; the contract's `lock_timeout` applies to an exclusive taker, as Technical Plan 4 says | Architecture decision 0050, `specs/029-custom-roles-folders/contracts/group-source.md` |

## Correctness re-read, round 2 (`approval/reread-2-correctness.md`, at `051e5f47`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: a replica without a successful ring read for two intervals refuses CLI redemption and refresh (unspent); an unknown key id triggers one re-read; promotion after two intervals; a second `rotate` resumes an unfinished one by check-and-set; a test | Requirement 013-026 |
| 2 | Nit | Fixed: `429 REFRESH_RATE_LIMITED` with `Retry-After` and a retryable `503 IDENTITY_SIGNING_UNAVAILABLE`, both unspent; a CLI test | Requirement 013-026, Technical Plan 11, Task 013-15 |
| 3 | Nit | Fixed: grace rotations are exempt from the limit | Requirement 013-026 |
| 4 | Nit | Fixed: a retired generation has no row and its use revokes; a test | Requirement 013-026 |
| 5 | Nit | Fixed: the MAC is verified and the successor minted from the session row locked `FOR UPDATE`; a barrier test | Requirement 013-026 |
| 6 | Nit | Fixed: the guarded row is upserted and audited once; retried when the administrator count changed | Technical Plan 6 |
| 7 | Nit | Fixed: the barrier test of the restart | Task 013-12 |
| 8 | Nit | Fixed: the attempt is consumed after the membership lock; `a_confirmation_sent_twice_links_once` | Technical Plan 4, Task 013-12 |
