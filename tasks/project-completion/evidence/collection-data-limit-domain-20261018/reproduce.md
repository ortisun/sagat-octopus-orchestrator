# Reproduce Q1a domain behavior

Set CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216; execute commands from domain.json, clippy.json, fmt.json, architecture.json. These pure domain targets need no PostgreSQL/runtime-role duplicate. All1109 inputs are pinned unchanged.

One invocation passes collection_reducer17, collection_restore_state6 and scoped_compensation30. Four new tests account for +2/+1/+1 compared to the preceding Q1 gate; counts are not combined across revisions. They prove empty/all-checkpointed no-member drain, exact failure code/no aggregate, full equality after event replay and projection restoration, unchanged actual Failed-member restriction, and domain compensation lease/completion required before closing. The compensation test is a pure domain harness, not a real gRPC worker/provider/OS restart claim. No prior RED is invented for an undeclared enum; genuine raw-decoder/public quota REDs are separately retained.

Native new reason production remains unavailable. PostgreSQL full/path reader acceptance, schema, current producer, real legacy worker/compensation races and actual binary upgrade remain pending.
