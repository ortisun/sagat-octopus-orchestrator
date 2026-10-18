# Required refused output and compensation — 2026-10-18

## Behavior

A real public collection executes 17 forward tasks with 250,000-byte reservation results. Sixteen outputs (4,000,000 bytes) are retained. The seventeenth success is recorded truthfully as SUCCEEDED while its refused output stays absent and the Run stops for RUN_DATA_LIMIT. Normal scheduling produces actual compensation leases.

One strict fixture worker handler parses each lease's real task input and releases the parsed reservation from a test-owned in-memory set. All 16 retained outputs pass this same handler. The refused output cannot supply the required reservation ID: the genuine handler failure is reported through public FailStep using fixture code FIXTURE_RESERVATION_OUTPUT_REQUIRED. The actual compensation target instance and idempotency key equal the refused forward lease.

Persisted WAITING/OPERATOR plus MANUAL_REVIEW retains the exact open saga entry, with no fabricated completion/skip or run.finished. Normal scheduling does not silently finalize it. Existing authorized RESOLVE_STEP FAILED targets the full instance, emits the exact resolution audit/journal event, and eventually yields COMPENSATION_FAILED. The refused reservation remains unreleased; retained ones reach the same successful handler regardless of independent iteration ordering. Journal verification passes before and after resolution.

## Executed gates

C2 (before query-model lint correction): focused 1/1 default 68.14 s and runtime 9.06 s; all byte regressions 23/23 in 18.48 s. Both focused invocations actually run Flow as sagat_app. Strict Clippy failed on two long query tuple types; the failure is preserved.

C3 replaces only those result tuples with named FromRow fields/SQL aliases. Final-source focused 1/1 default 60.45 s (49.40 s build) and runtime 9.92 s; strict integration Clippy passed 44.87 s, formatting 4.90 s and architecture 0.47 s. Native 1122 input hashes match for each separately pinned revision. The full 23-case regression was not repeated after that query-model-only correction. Exact commands, exits, logs and executed patches are retained.

Review required exact target-key/full-instance comparison; accepted and corrected. No other required C1 findings. The subsequent SQL result model correction adds no branch or production behavior.

## Limits

This is current-version engine/worker application behavior. In-memory reservation state is a fixture, not a durable external provider. No provider undo, exactly-once, old-binary compatibility, OS restart, new engine input precheck or new built-in error code is claimed. Q10c/other quota recovery controls and all pending canonical 014 tasks remain open.
