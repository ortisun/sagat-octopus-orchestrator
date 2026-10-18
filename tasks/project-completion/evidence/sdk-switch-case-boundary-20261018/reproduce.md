# Five-SDK switch case boundary

Refs:T014-041, WP-07/12, FR-014-009/015. Existing switch-builders scenario now authors32cases,
publishes/executes the actual document and rejects the otherwise valid33-case document through
each public builder. Canonical expected IR and structured expected errors were authored before
execution; no SDK output/golden blessing. Existing full runtime golden is unchanged.

Actual focused all-five aggregate passed1/1 in28.14s (harness27.32s), runtime sagat_app, all five
prepare scripts, no reuse/bless/old source/language filter. Exact command in command.json selects
only SAGAT_SDK_SCENARIO=switch-builders. Shared warm target, CARGO_INCREMENTAL=0,
CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216,SAGAT_TEST_AS_RUNTIME_ROLE=1; remove
SAGAT_TEST_DATABASE_URL/SAGAT_SDK_BLESS/SAGAT_SDK_LANGS/SAGAT_SDK_REUSE/SAGAT_SDK_SOURCE_ROOT.

Five complete current normalized outputs independently equal unchanged reviewed runtime golden.
This is a focused boundary refresh, not a newly rerun full58-case aggregate. No implementation,
wire protocol or server semantics change. T014-041 remains open for expression/list semantics and
any remaining full contract observations. No optional Rust workspace suite represented as rerun.
