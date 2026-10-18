# Current complete SDK60 evidence — 2026-10-18

Execution base: `6b34f227daa739cce16c05eddd96257bae61f6bd`; documentary head at independent
inspection: `4f3b6ac0ead1162151e9c909d0de63331f63925f`. See `inspection.json`, `command.json`,
`aggregate.txt`, `source-before.json`, `source-after.json`, `output-index.json` and all270
complete `normalized/<scenario>/<language>.json` results. All1672 frozen hashes matched
before/after the actual run and independent inspection. Post-test Markdown updates are
explicitly listed in `post-test-documentary-delta.json`; executable inputs did not change.

```sh
unset SAGAT_SDK_LANGS SAGAT_SDK_SCENARIO SAGAT_SDK_BLESS SAGAT_SDK_REUSE SAGAT_SDK_SOURCE_ROOT
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
cargo test --locked -p sagat-tests --test sdk_conformance every_sdk_conforms_to_every_scenario -- --exact --ignored --nocapture
```

The loopback URL is the dedicated disposable PostgreSQL16.15 test server, trust authentication;
World creates a unique database per test. Provision an equivalent disposable server when
reproducing. No production database is used. Every registered case ran in all five languages.
Six authoring-only cases create30 plays, `builders` creates5 publication-only plays, and the
remaining53 create265 live workflow/protocol plays. The54 server-phase cases each retain five
complete normalized results. Independent structured JSON comparison against reviewed goldens
preserves boolean/numeric type distinctions. No blessing/reuse/filter or historical source.

Result: one aggregate test passed, 1067.94s wall /1067.57s harness. The native compatibility8/8
run is retained in `../scoped-finish-downgrade-20261018/compatibility.txt`; comparison against
that tested commit found no change in any frozen executable input. The final bounded public
mapping review is retained in `../sdk60-public-mapping-20261018/`; its documentary corrections
predate this full pass. T014-041 closes. Nested/C0/resource/final release gates remain open.

The first full attempt and same-source focused retry are separately retained in
`../sdk60-first-failure-20261018/`. Initial failure: managed-secret PUT503 before database
scenario outputs; cause not established. This subsequent full aggregate passed unchanged.
No provider-health inference follows from an optional monitor with zero observations.
Actual archived v0.1.0 lifecycle evidence remains separately scoped; this run uses current SDKs.
