# Targeted re-read 3 of design 053: security (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 9bcd696c3ef472e13471fd28367afccde29d4c1c
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation of the
design after the other reviewers' re-reads (`connect_command`, the descriptors, the widening's one lock, phase 2's lock
order and retries, the fence's serialization path, `journal-buckets-v1` without a fence), checked with no Blocker or
Major; re-read 2's Nits confirmed fixed. The reviewer read files only and ran nothing.

No Blocker, no Major.

## Nit

1. `sagat_retirer` lacks what every connection's opening needs (`sagat_server_identity()` in `after_connect`, `SELECT`
   on `registry_first_confirmation`), so retirement fails closed, and the quick fix (membership of `sagat_app`) would
   hand it the runtime role. Fix: name both grants in designs 053, 048 and 049 and the catalog allowlist; never a
   membership; test the real `connect_command` path as `sagat_retirer`.
2. Nothing keeps `mixed-writers-fixture` and its migrations out of a published binary; its directory has no owner and
   no lint scope. Fix: the `test-support` treatment (refused in normal dependencies, `cargo tree` on release
   artifacts); owners and the security rules for the directory.
3. The descriptor test accepts a drain check with no fence silently. Fix: an explicit "no fence" variant with its
   reason, the capabilities using it listed by name.

Counts: Blocker 0, Major 0, Nit 3

All three were fixed at the next commit.
