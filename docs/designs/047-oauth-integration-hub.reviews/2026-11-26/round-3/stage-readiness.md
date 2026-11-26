# Design 047, EGM stage at `4bc5767b` (2026-11-26)

The reviewer's output, condensed by the author; every finding and severity kept. Summarised and answered in `egm-review.md`.

## Implementation readiness (egm-implementation-readiness)

1. Major. The dependency list leaves out feature 021 tasks the steps need: Task 047-9 serves the `oauth` scheme through
   Task 021-084 and its test needs Tasks 021-024 and 021-025; Task 047-11 edits those executors; declarations and
   admission rely on Tasks 021-021 and 021-022.
2. Major. Technical Plan 6 refuses deployment registrations and offers until Architecture decision 0074 is accepted, while
   `rest-api.md` serves them as schema-owner `control-plane` commands until then.
3. Major. Files without one owner: `publishing.rs` and `runs/` (publication and admission checks) belong to no task;
   Requirement 047-014's admission pinning has no task; Task 047-14's tests sit in builder A's crates; Task 047-15 omits
   the gRPC handler in `crates/api-grpc` and the claim gating in `worker.rs` and `execution.rs`; Requirement 047-023's
   block check has no task.
4. Major. `rest-api.md` gives no request or response schemas beyond authorizations, callback and import: provider,
   registration, credential and offer bodies, the secret or key format, the delete parameter "with a disconnect request",
   a tenant route listing offers and an offer identifier are missing.
5. Major. The data model lacks a token value hash, revocation attempt columns, the bound Microsoft directory, the record of
   `forceRefresh` honoured once per access revision, and the grant for `refresh_stops_at`; `oauth_request_windows` puts
   `tenant_id` first while holding rows without a tenant; `grant` and `window` are reserved words; the acquire's "room"
   is a placeholder.
6. Major. Tests in no task (`a_withdrawn_offer_cannot_be_opted_into`, `the_next_attempt_after_reconnecting_succeeds`, the
   two vault-reference tests), and Phase A tests that need later phases (offers, deployment registrations, declarations).
7. Major. Task 047-4 migrates feature 028's tables before Task 028-063 exists, and Tasks 047-4 and 047-13 both claim
   `hub_record_revocation`.
8. Nit. The schedule does not mark the delivery tasks gated on 5.1.0.
9. Nit. The decision range and "present when probed" (Decision 047-064).
10. Nit. "Older than the lease plus the store deadlines" has no number in `timings.md`.
11. Nit. `SUSPECT` is never defined; required scopes appear on the provider and on the resource type.
12. Nit. `hostBindings` binds only `tokenEndpointPath`; `api_key_header` has no `headerName`.
13. Nit. Task 047-1 omits `sdks/error-catalog.json`; Task 047-10 names no tests for `contract_integration_hub`; Task
    047-18's CLI subcommands are not specified.

Counts: Blocker 0, Major 7, Nit 6
