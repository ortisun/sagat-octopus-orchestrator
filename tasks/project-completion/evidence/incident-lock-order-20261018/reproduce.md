# Production incident/report binding order and public script-chain entry

Baseb308aa47. Disposable repository PostgreSQL. Unset SAGAT_TEST_DATABASE_URL; set
SAGAT_TEST_AS_RUNTIME_ROLE=1/CARGO_INCREMENTAL=0/CARGO_BUILD_JOBS=3/
RUST_MIN_STACK=16777216. Exact commands/exits/wall times in commands.json and
regression-commands.json. Optional warm target; no external CLI.

red.txt/test-red.diff/red-source.json:0/2 in3.09s. Real native RunTx.bound_patterns concurrent
with public record_incident produces PostgreSQL deadlocks0→1 plus incidentErrUnavailable.
This counter observation proves detected deadlock; generic Unavailable alone does not. The
second realpublicpublished/built/leased continuation script incident commits despite heldchain.
No fixture/compile errors. Production diff adds scoped memberchain and moves existingBindingX
after exactownrun/namespacevalidation, before unchangedStep/fence/postlockexpiry/audit checks.

green.txt:2/2 in2.12s. Native report-storage prefix and incident both succeed with no detected
deadlock, one audit; actual public completion and verify_run are subsequent positive controls,
not controlled concurrency of full public completion. Separate script-chain barrier requires
actual app-role wait and free ownrun NOWAIT, then audit/completion/journal verification.
Runtime option selects World's runner/store pool; waiter role is checked.

ADR0038 is required by AGENTS because ADR0022 explicitly specified the previous order; only
production serialization is refined. Current wire/tenant/fence/category/redaction/deduplication/
test-branch/ACK boundaries are unchanged. Operation remains audit-only, with no outbox or
external exactly-once promise. Regression security125/12561.19s, clippy73.93s/fmt/arch/diff pass.
No actual sandbox execution, allworkspace tests, SDK58/Compose/advisory/resource/nesting gate
claimed. New script fixture builds/leases via public platform; it does not execute user code
inline. B0 and separate renewal/worker-session postwait source/test gaps remain pending.
