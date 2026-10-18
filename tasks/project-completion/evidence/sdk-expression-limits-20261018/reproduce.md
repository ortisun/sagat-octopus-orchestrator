# Shared expression boundaries

Use the same coordinated warm target at /Users/marcelo/Documents/Projects/
sagat-octopus-database-connections/target (join that wrapped path), CARGO_INCREMENTAL=0,
CARGO_BUILD_JOBS=3, RUST_MIN_STACK=16777216 and SAGAT_TEST_AS_RUNTIME_ROLE=1. Unset
SAGAT_TEST_DATABASE_URL, SAGAT_SDK_BLESS, SAGAT_SDK_LANGS, SAGAT_SDK_REUSE and
SAGAT_SDK_SOURCE_ROOT. Select SAGAT_SDK_SCENARIO=expression-limits for the focused aggregate,
then state-operations for the unchanged ordinary phase regression. Exact commands and actual
wall times are in commands.json; red-command.json retains the pre-implementation failure.

The shared stage explicitly separates expected HTTP publication failures from existing public
SDK query calls. No SDK API/runner or server behavior changes. Exact authored cases and complete
canonical/runtime expectations were written before any SDK execution, without blessing. RED
27.66 s proved old staging incorrectly required publication success for the intended negative.
First compile failed on Option<Uuid> serialization; retain source/log, then project Option<String>.

Final registry 1/1 passed 28.08 s. All-five expression aggregate passed 28.46 s; all-five ordinary
state regression passed 19.05 s. All ten complete outputs equal their full reviewed goldens;
1,241 tested tracked inputs remained unchanged. Later edits to README and conformance.md only
document this tested phase. Original tested hashes and documentary delta are retained separately.
No new full 59-case aggregate is represented by this focused run. No private configs retained.

Targeted SDK harness Clippy with warnings denied passed in 10.03 s; workspace format check passed
in 4.31 s; 24-crate architecture check passed in 0.46 s. Actual gate commands/logs are retained.
