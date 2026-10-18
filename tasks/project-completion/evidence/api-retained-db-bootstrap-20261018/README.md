# Retained-database API bootstrap — 2026-10-18

This test-only bootstrap dependency now reopens a caller-owned, already migrated database through the existing real HTTP/gRPC API wiring. Default `start_with` still creates and migrates a fresh database. `attach_existing_db` validates the harness database name and actual connected identity; it performs no migration, provisioning or fallback creation. The caller owns PostgreSQL lifetime.

## Executed evidence

- C1 behavioral RED: 0 passed, 1 failed, exit 101, 76.22 s. Second fresh-database API returned HTTP 401 instead of 200 for the original principal. Read-only diagnostic found the new empty base and the original base with its leased task and stored session.
- C2 GREEN: 1/1, 50.94 s including compilation (harness 1.47 s). Both APIs use actual `sagat_app`; owner inspection remains separate. Exact original tenant, memberships, bindings and sole session are unchanged through bootstrap, reconnect, authenticated polling and submission of the original completion request. Current JWT performs current auth lookups. Run reaches SUCCEEDED.
- Existing default-bootstrap collection HTTP regression: 3/3, 3.64 s, explicit runtime-role mode.
- Strict integration Clippy: passed, 20.86 s. This also checks the final C3 orphan-payload oracle from the preceding report/replay increment. Formatting: passed, 4.69 s. Architecture: passed, 24 crates, 0.51 s.
- GREEN native source inputs: 1122 files, before/after identical. Exact commands, exits and raw output are retained alongside each run.

C1 and C2 patches and review reconciliation are preserved. C2's stronger RED patch was not executed; only C1 supplies the real behavioral counterexample.

## Limits

This is a current/current API **instance** restart in one OS process. It does not prove immutable-old/current compatibility, OS process recovery, secret/blob key continuity, report/drain order or external provider effects. It closes only the retained-database harness dependency and no canonical feature 014 task. No deployment or merge occurred.
