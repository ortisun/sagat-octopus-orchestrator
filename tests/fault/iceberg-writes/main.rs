//! Feature 035 crash, duplicate, race and lost-reply cases for Iceberg writes, driven by barriers and held
//! locks, never sleeps. No case exists yet; the I2 fault tasks of tasks.md add them.
#![allow(clippy::expect_used, clippy::panic)]
