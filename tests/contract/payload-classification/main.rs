//! Canonical contract tests for payload classification (feature 016).
#![allow(clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

mod events;
mod grants;
mod openapi;
mod rest;
mod schema;

#[path = "../../support/mod.rs"]
mod support;
mod validate;
