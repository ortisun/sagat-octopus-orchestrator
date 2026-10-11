//! A bounded subset of CEL for workflow conditions, correlation keys and idempotency templates.
//!
//! Expressions are written by tenants and evaluated inside the control plane, so this crate is
//! deliberately not a general CEL engine:
//!
//! * the reference implementation recurses without bound on hostile input (a nesting depth of 12
//!   overflows a 2 MiB stack in a debug build), and its comprehension macros (`all`, `map`, ...) can
//!   burn unbounded CPU while a run lock is held;
//! * this crate parses with a hard nesting depth, token and node budget ([`limits`]), evaluates with
//!   a step budget, offers no macros, no regular expressions and no side effects, and allocates
//!   nothing it has not accounted for.
//!
//! The subset is checked against the reference implementation in `tests/differential.rs`: wherever
//! the reference succeeds, this crate returns the same value. Where they differ it is by design and
//! documented in [`eval`]: numbers promote to double when mixed, and a division by zero is an error
//! even for doubles (JSON has no infinity).

mod error;
mod eval;
mod lexer;
pub mod limits;
mod parser;
mod template;

pub use error::ExprError;
pub use eval::{Compiled, Env};
pub use template::Template;
