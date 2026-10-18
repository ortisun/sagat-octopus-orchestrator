Sol/high fresh read-only routine extraction review: None. Exact diff imports the genuine shared
identity, changes canonical to crate-private reader inputs actual locked pin/revision and updates
its sole caller. Query/binds/finite timestamp/decode/errors/replay/side effects unchanged; no DTO,
authority, re-export or unrelated change. Static public type visibility and Copy pin checked.
No Rust tests/builds run by reviewer.
