//! Feature 035 Iceberg write steps against real PostgreSQL and the disposable lake fixtures of
//! `infra/docker/compose.lake.yaml`. The cases of the write steps themselves start at T035-013; the first case
//! here proves the fixtures (T035-004).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod isolation;
mod lake_fixture;
mod lake_reader;
