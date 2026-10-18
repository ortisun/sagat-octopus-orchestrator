# Current dependency advisory gate

On25a197df, the exact CI command `cargo audit --deny warnings` passes (exit0,2.22s) with cargo-audit0.22.2, after fetching the RustSec database. It loads1338 advisories and scans627 crate dependencies. Advisory database commit is retained separately.

The existing `.cargo/audit.toml` exceptions remain unchanged. This is a pass under that policy, not an assertion of zero known advisories or a new acceptance of those exceptions. No dependency, lockfile or CI source changed. Input hashes were captured after execution and are labelled as such; no invented before/after pin claim. This completes only this dependency advisory observation within T014-043; other gates and canonical task closure remain pending.
