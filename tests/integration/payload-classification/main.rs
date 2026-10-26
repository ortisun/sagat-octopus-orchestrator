//! Integration target reserved by T016-003. Its real PostgreSQL cases are introduced by the
//! implementation tasks that own each behavior; this registration makes those cases first-class.

mod canary;
mod compat;
mod dead_letters;
mod disclosure;
mod events;
mod grants;
mod journal;
mod pinning;
mod pinning_routes;
mod rest;
mod result;
mod security;
mod step_outputs;
#[path = "../../support/mod.rs"]
mod support;
