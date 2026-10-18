# Current all-five SDK 58-scenario aggregate

Execution started at dc212855 on feature/016-project-completion. During execution, only the
conditional scoped binding documentary decision was committed (1bb9eed6); Rust/SDK/contracts
inputs remain frozen. source-before.json and source-after.json record tracked input hashes.
Actual commands/status/wall times are retained in commands.json; local tools in toolchains.json.

Reuse the coordinated warm CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216. No second Rust target/cache was created.
The subprocess environment removes SAGAT_TEST_DATABASE_URL, SAGAT_SDK_BLESS, SAGAT_SDK_SCENARIO,
SAGAT_SDK_LANGS, SAGAT_SDK_REUSE and SAGAT_SDK_SOURCE_ROOT, sets SAGAT_TEST_AS_RUNTIME_ROLE=1.
Five current prepare scripts execute; no archived clients or blessed goldens are selected.

First run cargo test --locked -p sagat-tests --test sdk_conformance
 every_scenario_declares_phases_with_reviewed_expected_documents -- --exact.
Then cargo test --locked -p sagat-tests --test sdk_conformance
 every_sdk_conforms_to_every_scenario -- --exact --ignored --nocapture.
The registry has 58 scenarios with 58 reviewed workflow goldens and 52 runtime/publication goldens.
Six scenarios are build-only; builders is publication-only; 51 perform live workflow/protocol work.
Thus this is a shared authoring/runtime matrix, not 58 live workflow scenarios.

Harness constructs each current language document, compares canonical content across the five
SDKs and to reviewed workflow goldens, then gives each runtime language an independent real
platform/database and compares its complete normalized result across languages and to the golden.
Builder-only cases never publish/execute synthetic references. Private configs remain outside this
evidence directory; retain only normalized result artifacts for the current harness PID.

A successful aggregate closes the current-client 58-case subgate only. Active-old-run/old-worker
compatibility is separate, as are nested capacity, source producer/C0, large benchmarks and full
feature014 release acceptance. No general nested runtime acceptance follows from build-only depth8.

## Actual result and independent inspection

Registry 1/1 passed 23.14 s. Full aggregate1/1 passed1055.10s (harness 1054.80 s), with all 58 exact unique
scenario headers naming 5 languages and no FAIL line. All 1238 tracked source inputs are identical.
Exactly 260 current normalized outputs from harness PID81293 cover every 52 runtime/publication case
and every 5 language combination, with no duplicate/missing pair; all complete JSON equals its
reviewed golden. Retained bytes 2914540. See inspection.json and normalized/<scenario>/<language>.json.

Fresh actual-runtime-role native compatibility 8/8 passed15.02s (harness 12.53 s). Command/log retained
in commands.json/native-compatibility.txt; exact inputs in native-compatibility-source.json.
These are existing native old-schema/reader/worker fixtures, not a newly executed old engine binary.
No optional unchanged Rust lint/workspace suite is represented as rerun for this evidence-only slice.
