# Incomplete first current60 aggregate — retained failure

At execution base8d15bacc, run the exact command in command.json with all five SDKs, no
selection/reuse/blessing/source override, SAGAT_TEST_AS_RUNTIME_ROLE=1 and a dedicated held
PostgreSQL16.15 test server (fresh per-World databases). Do not use a production database.

Result: exit101 after593.74s (harness570.55s),31 complete scenarios and135 complete normalized
outputs independently equal their reviewed goldens. At the next scenario, database-connections,
the initial managed-secret PUT returned503/UNAVAILABLE before a language runtime output existed.
The response and trace/request IDs are preserved in aggregate.txt; it contains no supplied
credential value. All1672 frozen Rust/SDK/shared-contract input hashes are identical before/after.

The same source and runtime-role setup immediately passed the focused database-connections
case in all five SDKs,25.53s (harness24.66s), using only already-prepared runners. There was no
code, provider configuration or golden change. The underlying503 cause was not established;
this is not labeled an observed readiness, permission or product defect. No speculative product
fix or write retry was added. A subsequent complete unfiltered run is required for T014-041.

partial-output-index.json retains hashes for the135 partial outputs, not their full contents.
Earlier successful full58/59 complete-output artifacts remain separately retained. The failed
aggregate and focused retry are never combined into a claimed full60 pass.
