# Supplied runtime pool backs the public Worker API

P0 only. A narrow test-support constructor accepts the caller's exact PgPool and shares the existing complete identity/execution/REST/gRPC startup body. Existing constructors keep their normal runtime/owner selection,8connections and configured timeouts. Caller-supplied configuration/lifecycle remains caller-owned.

## Actual acceptance

The new public test reopens the exact owned database with the original durable worker/session/lease and current JWT. It uses the unchanged unarmed relay's single sagat_app backend, explicit30s session statement/idle bounds, and captures database/user/application/PID. An exact receipt advisory gate observes THAT public command waiting on THAT backend through the owner pool; idle identity alone cannot pass. Real250000-byte completion is verified against exact stored payload bytes, original session token hash and independently computed canonical SHA256 receipt fingerprint. Original full receipt and public ACK replay remain stable; worker authority is unchanged. Cleanup drops client/API references, bounded-closes the supplied pool, then drops the unarmed abort guard without calling the fault-resolution close method.

Final guarded api_reopen:: invocation passes2/2 in default mode3.04s (harness2.03s) and explicit runtime mode2.66s (harness1.74s), zero ignored/failures. Each includes the existing original reopen regression and new supplied-pool case. Both actualAPIpools are sagat_app; this is not an owner/runtime privilege matrix.1125nativeinput hashes match before/after and the C2 source used by static gates. Direct read-only owner inspection of all four final owned DBs verifies terminalRuns, sole session/receipt, exact expected250000-byte payloads and removed gate triggers without exposing private values. Initial inspection SQL used a nonexistent runs.run_id; corrected runs.id query and the diagnostic are recorded separately from test results.

## Lifecycle execution boundary

The internal90s Tokio timeout is cooperative and cannot preempt synchronous shared server initialization/destruction. Every final acceptance run therefore uses GNU timeout around the entire cargo/test process: TERM180s, KILLafter5s, no --foreground/--preserve-status. Nonzero/timeout or missing exact2/0ignored count fails. Actual GNUversion and exact commands are retained. This execution profile uses root-owned external PostgreSQL, with no Testcontainers PostgreSQL teardown in the test. No forced Docker cleanup or intrinsic90s whole-scenario guarantee is claimed. See process-deadline-contract.md and final-guarded outputs.

## Reviews / static gates

C1 constructor/backend wiring review None; receipt/lifecycle review found four actionable gaps: actual stored bytes, session credential, owner SQL deadlines and canonical hash. C2 fixes those. Fresh C2 identified synchronous lifecycle outside cooperative timeout; C3 execution protocol uses the reviewer's external-process alternative. Fresh C3 required findings None; bounded stop reached. Original drafts/notes/deltas/snapshots and preliminary results remain preserved. No runtime RED is invented for static findings. GPT-5.6 Sol drafted; fresh GPT-6 Astra reviewed; root alone applied/executed. Previously declined optional external review was not repeated.

Final source strict integration Clippy18.07s, fmt4.91s, architecture1.02s pass. Preliminary C1 Clippy35.01s and C2 unguarded default2/2 at52.42s are separate, not final hard-deadline evidence. Suggested scripts/check-agent-config.py does not exist and was not executed. Source/doc whitespace checked separately from retained raw artifacts.

## Reproduction / remaining work

With GNU timeout installed, the absolute coordinated external CARGO_TARGET_DIR and SAGAT_TEST_DATABASE_URL pointing to the owned pg_stat_statements-enabled PostgreSQL, run the exact final-guarded command JSON in default and SAGAT_TEST_AS_RUNTIME_ROLE=1 environments. The180s process bound includes compilation; prebuild on slower cold machines rather than weakening timeout/nonzero handling.

This proves supplied-pool public bootstrap only. Scoped refusal path rollback/full reopen, actual COMMIT acknowledgement loss and unknown-outcome recovery are separate prerequisites/tests. No OS restart, process death, provider undo, production pool option, migration, grant or release activation is claimed. Full Q7b/Q10c and all canonical014tasks remain open.
