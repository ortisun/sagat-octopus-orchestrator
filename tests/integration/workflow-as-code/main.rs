//! Integration tests of design 044, workflow as code (`docs/designs/044-workflow-as-code.md`), on real PostgreSQL
//! through Testcontainers. The modules land with the server tasks that make them pass (Tasks 044-6 to 044-33); the
//! tenant-boundary tests live in the module `security` (Task 044-24).
#![allow(clippy::expect_used, clippy::panic)]
