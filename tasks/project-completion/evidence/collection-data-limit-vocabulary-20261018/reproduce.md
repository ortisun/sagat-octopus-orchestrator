# Reproduce

Set CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216. Exact commands are stored in codec.json, existing-domain-collections.json, clippy.json, fmt.json and architecture.json. These unit/domain gates need no PostgreSQL server or runtime-mode duplication. Source pins1109 match exactly.

Codec9/9 includes raw_collection_data_limit_drain_decodes_without_a_fictitious_member from the genuine preimplementation RED, plus the new roundtrip/cause case. Existing domain target counts are collection_reducer15, collection_restore_state5, scoped_compensation29 (one invocation, not reruns). New empty/checkpointed/restore/compensation reason tests remain separate. The two public empty-collection and the two collection End/instance-limit tests remain RED because no native correction is active.
