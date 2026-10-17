//! Integration target reserved by T016-003. Its real PostgreSQL cases are introduced by the
//! implementation tasks that own each behavior; this registration makes those cases first-class.

mod dead_letters;
mod disclosure;
mod events;
mod grants;
mod journal;
mod pinning;
mod rest;
mod result;
#[path = "../../support/mod.rs"]
mod support;
