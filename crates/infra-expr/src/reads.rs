//! The places of its variables an expression reads, for a caller that classifies what an expression can see (feature
//! 022, the classification of a key, FR-022-057).
//!
//! A read is a variable and a path of literal steps into it (`input.account.id`, `steps.fetch.output["region"]`,
//! `input.items[0]`), and stands for the value at that place and everything under it: an expression may compare, size or
//! convert a whole structure, so nothing below a read is out of its reach. A step whose name or position is computed
//! (`input.items[input.at]`) ends the path where it stands, and the expression that computes it is read in turn. The
//! reading is conservative: it may name more than an evaluation touches (both arms of a condition), never less.

use serde_json::Value;

use crate::parser::Expr;

/// One step of a read's path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReadStep {
    /// A named field (`.name`, or an index by a string literal, `["name"]`).
    Field(String),
    /// A position of a list, by a non-negative integer literal (`[0]`).
    Position(u64),
}

/// A place an expression reads: the value of `variable` at `path`, and everything under it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Read {
    pub variable: String,
    pub path: Vec<ReadStep>,
}

/// Every place `root` reads, each once, in order.
pub(crate) fn reads(root: &Expr) -> Vec<Read> {
    let mut out = Vec::new();
    collect(root, &mut out);
    out.sort();
    out.dedup();
    out
}

/// The variable and literal path `expr` is, when it is nothing else.
fn place(expr: &Expr) -> Option<Read> {
    match expr {
        Expr::Var(name) => Some(Read {
            variable: name.clone(),
            path: Vec::new(),
        }),
        Expr::Field(inner, name) | Expr::Has(inner, name) => {
            let mut read = place(inner)?;
            read.path.push(ReadStep::Field(name.clone()));
            Some(read)
        }
        Expr::Index(inner, at) => {
            let step = match at.as_ref() {
                Expr::Literal(Value::String(name)) => ReadStep::Field(name.clone()),
                Expr::Literal(Value::Number(number)) => ReadStep::Position(number.as_u64()?),
                _ => return None,
            };
            let mut read = place(inner)?;
            read.path.push(step);
            Some(read)
        }
        _ => None,
    }
}

fn collect(expr: &Expr, out: &mut Vec<Read>) {
    if let Some(read) = place(expr) {
        out.push(read);
        return;
    }
    match expr {
        Expr::Literal(_) | Expr::Var(_) => {}
        Expr::List(items) | Expr::Call(_, items) => {
            items.iter().for_each(|item| collect(item, out))
        }
        // a field of something that is not a place (a condition's result, a list): everything the inner expression
        // reads, whole
        Expr::Not(inner) | Expr::Neg(inner) | Expr::Field(inner, _) | Expr::Has(inner, _) => {
            collect(inner, out);
        }
        // a computed step: the place up to it, whole, and what computes it
        Expr::Index(inner, at) => {
            collect(inner, out);
            collect(at, out);
        }
        Expr::Binary(_, left, right) | Expr::And(left, right) | Expr::Or(left, right) => {
            collect(left, out);
            collect(right, out);
        }
        Expr::Cond(condition, then, otherwise) => {
            collect(condition, out);
            collect(then, out);
            collect(otherwise, out);
        }
        Expr::Method(receiver, _, args) => {
            collect(receiver, out);
            args.iter().for_each(|arg| collect(arg, out));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Read, ReadStep};
    use crate::Compiled;

    fn read(variable: &str, path: &[&str]) -> Read {
        Read {
            variable: variable.to_owned(),
            path: path
                .iter()
                .map(|step| {
                    step.parse::<u64>()
                        .map_or_else(|_| ReadStep::Field((*step).to_owned()), ReadStep::Position)
                })
                .collect(),
        }
    }

    fn reads(source: &str) -> Vec<Read> {
        Compiled::compile(source).expect("compiles").reads()
    }

    #[test]
    fn a_literal_path_is_read_to_its_end() {
        assert_eq!(
            reads("input.account.id"),
            [read("input", &["account", "id"])]
        );
        assert_eq!(
            reads("steps.fetch.output['region']"),
            [read("steps", &["fetch", "output", "region"])]
        );
        assert_eq!(reads("input.items[0]"), [read("input", &["items", "0"])]);
        assert_eq!(reads("has(input.tier)"), [read("input", &["tier"])]);
    }

    #[test]
    fn a_computed_step_ends_the_path_and_its_computation_is_read_too() {
        assert_eq!(
            reads("input.items[input.at].name"),
            [read("input", &["at"]), read("input", &["items"])]
        );
        assert_eq!(reads("input.items[-1]"), [read("input", &["items"])]);
    }

    #[test]
    fn every_operand_call_and_arm_is_read() {
        assert_eq!(
            reads("input.a + '-' + string(state.b) + (input.c ? steps.x.output.d : input.e).f"),
            [
                read("input", &["a"]),
                read("input", &["c"]),
                read("input", &["e"]),
                read("state", &["b"]),
                read("steps", &["x", "output", "d"]),
            ]
        );
        assert_eq!(
            reads("input.name.startsWith(input.prefix)"),
            [read("input", &["name"]), read("input", &["prefix"])]
        );
        assert_eq!(reads("'fixed'"), []);
        assert_eq!(reads("input"), [read("input", &[])]);
    }
}
