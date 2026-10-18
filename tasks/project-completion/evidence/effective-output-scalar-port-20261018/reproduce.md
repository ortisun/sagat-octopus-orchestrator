# Reproduce

Set CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216. clippy.json checks every application target with -D warnings (19.63 s); fmt.json records actual formatting check. These are compile/static gates, no executed-test count. All1109 inputs unchanged. No dependency/architecture edge, native writer, public API, SDK or grant changed. Existing real G21 result regression acceptance remains separately pinned at da9201b2.
