//! Evaluation over JSON values with a step budget.
//!
//! Values are `serde_json::Value`. Integers and doubles are distinct, comparisons and arithmetic
//! between them promote to double (a deliberate superset of CEL, whose arithmetic refuses to mix
//! them). Unknown variables and fields are errors, never `null` or `false`, except inside `has()`.
//! `&&` and `||` absorb an error when the other side decides the result, as CEL specifies.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::error::{ExprError, clip};
use crate::limits::{MAX_LIST_ELEMENTS, MAX_STRING_BYTES};
use crate::parser::{BinOp, Expr, Function, Method, parse};

/// The variables an expression can see (`input`, `steps`, `run`, ...).
#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: BTreeMap<String, Value>,
}

impl Env {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with(mut self, name: &str, value: Value) -> Self {
        self.vars.insert(name.to_owned(), value);
        self
    }
}

/// A parsed expression, ready to evaluate any number of times.
#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    root: Expr,
}

impl Compiled {
    /// # Errors
    /// An [`ExprError`] for anything outside the bounded subset.
    pub fn compile(source: &str) -> Result<Self, ExprError> {
        Ok(Self {
            root: parse(source)?,
        })
    }

    /// The top-level variables the expression refers to, so callers can load only those.
    #[must_use]
    pub fn variables(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut stack = vec![&self.root];
        while let Some(expr) = stack.pop() {
            match expr {
                Expr::Var(name) => {
                    out.insert(name.clone());
                }
                Expr::Literal(_) => {}
                Expr::List(items) => stack.extend(items),
                Expr::Not(a) | Expr::Neg(a) | Expr::Field(a, _) | Expr::Has(a, _) => stack.push(a),
                Expr::Binary(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Index(a, b) => {
                    stack.push(a);
                    stack.push(b);
                }
                Expr::Cond(a, b, c) => {
                    stack.push(a);
                    stack.push(b);
                    stack.push(c);
                }
                Expr::Call(_, args) => stack.extend(args),
                Expr::Method(receiver, _, args) => {
                    stack.push(receiver);
                    stack.extend(args);
                }
            }
        }
        out
    }

    /// # Errors
    /// An [`ExprError`] when evaluation fails or exceeds its budget.
    pub fn eval(&self, env: &Env) -> Result<Value, ExprError> {
        Run { env }.eval(&self.root).map(Cow::into_owned)
    }

    /// Evaluates to a boolean.
    ///
    /// # Errors
    /// [`ExprError::Type`] when the result is not a boolean, plus any evaluation error.
    pub fn eval_bool(&self, env: &Env) -> Result<bool, ExprError> {
        match self.eval(env)? {
            Value::Bool(b) => Ok(b),
            _ => Err(ExprError::Type("a condition must evaluate to a boolean")),
        }
    }

    /// Evaluates to a string usable as a key: strings, integers, doubles and booleans.
    ///
    /// # Errors
    /// [`ExprError::NotScalar`] for null, lists and maps, plus any evaluation error.
    pub fn eval_key(&self, env: &Env) -> Result<String, ExprError> {
        scalar_text(&self.eval(env)?)
    }
}

pub(crate) fn scalar_text(value: &Value) -> Result<String, ExprError> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(match n.as_i64() {
            Some(i) => i.to_string(),
            None => n.as_f64().map_or_else(|| n.to_string(), format_double),
        }),
        _ => Err(ExprError::NotScalar),
    }
}

fn format_double(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{f:.0}")
    } else {
        f.to_string()
    }
}

#[derive(Clone, Copy)]
enum Num {
    Int(i64),
    Float(f64),
}

fn as_num(value: &Value) -> Option<Num> {
    match value {
        Value::Number(n) => n
            .as_i64()
            .map(Num::Int)
            .or_else(|| n.as_f64().map(Num::Float)),
        _ => None,
    }
}

#[allow(clippy::cast_precision_loss)] // promotion to double is the documented behaviour
const fn to_f64(n: Num) -> f64 {
    match n {
        Num::Int(i) => i as f64,
        Num::Float(f) => f,
    }
}

fn number_value(f: f64) -> Result<Value, ExprError> {
    serde_json::Number::from_f64(f)
        .map(Value::Number)
        .ok_or(ExprError::Overflow)
}

/// Evaluation visits every node at most once (there are no loops or macros), so its cost is linear
/// in the expression, which [`crate::limits::MAX_TOKENS`] bounds; no separate step budget is needed.
struct Run<'e> {
    env: &'e Env,
}

impl<'e> Run<'e> {
    /// A thin dispatcher: the recursion goes through here, so its frame must stay small. Each arm's
    /// real work lives in a helper that returns before the next recursive call, which keeps the
    /// stack cost per nesting level to a few hundred bytes even in a debug build.
    fn eval<'x>(&mut self, expr: &'x Expr) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        match expr {
            Expr::Literal(v) => Ok(Cow::Owned(v.clone())),
            Expr::Var(name) => self.variable(name),
            Expr::List(items) => self.list(items),
            Expr::Not(inner) => self.not(inner),
            Expr::Neg(inner) => self.negate(inner),
            Expr::And(left, right) => self.logical(left, right, false),
            Expr::Or(left, right) => self.logical(left, right, true),
            Expr::Cond(c, t, f) => self.conditional(c, t, f),
            Expr::Field(base, name) => self.field(base, name),
            Expr::Has(base, name) => self.has(base, name),
            Expr::Index(base, index) => self.index(base, index),
            Expr::Binary(op, left, right) => self.binary_op(*op, left, right),
            Expr::Call(function, args) => self.function(*function, &args[0]),
            Expr::Method(receiver, method, args) => self.method(*method, receiver, args),
        }
    }

    #[inline(never)]
    fn variable(&self, name: &str) -> Result<Cow<'e, Value>, ExprError> {
        self.env
            .vars
            .get(name)
            .map(Cow::Borrowed)
            .ok_or_else(|| ExprError::UndeclaredVariable(clip(name)))
    }

    #[inline(never)]
    fn list<'x>(&mut self, items: &'x [Expr]) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            out.push(self.eval(item)?.into_owned());
        }
        Ok(Cow::Owned(Value::Array(out)))
    }

    #[inline(never)]
    fn not<'x>(&mut self, inner: &'x Expr) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        match self.eval(inner)?.as_ref() {
            Value::Bool(b) => Ok(Cow::Owned(Value::Bool(!b))),
            _ => Err(ExprError::Type("'!' needs a boolean")),
        }
    }

    #[inline(never)]
    fn negate<'x>(&mut self, inner: &'x Expr) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        let value = self.eval(inner)?;
        match as_num(value.as_ref()) {
            Some(Num::Int(i)) => i
                .checked_neg()
                .map(|n| Cow::Owned(Value::from(n)))
                .ok_or(ExprError::Overflow),
            Some(Num::Float(f)) => Ok(Cow::Owned(number_value(-f)?)),
            None => Err(ExprError::Type("unary '-' needs a number")),
        }
    }

    #[inline(never)]
    fn conditional<'x>(
        &mut self,
        condition: &'x Expr,
        if_true: &'x Expr,
        if_false: &'x Expr,
    ) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        let chosen = match self.eval(condition)?.as_ref() {
            Value::Bool(true) => if_true,
            Value::Bool(false) => if_false,
            _ => return Err(ExprError::Type("'?:' needs a boolean condition")),
        };
        self.eval(chosen)
    }

    #[inline(never)]
    fn field<'x>(&mut self, base: &'x Expr, name: &str) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        match self.eval(base)? {
            Cow::Borrowed(Value::Object(map)) => map
                .get(name)
                .map(Cow::Borrowed)
                .ok_or_else(|| ExprError::NoSuchKey(clip(name))),
            Cow::Owned(Value::Object(mut map)) => map
                .remove(name)
                .map(Cow::Owned)
                .ok_or_else(|| ExprError::NoSuchKey(clip(name))),
            _ => Err(ExprError::Type("only a map has fields")),
        }
    }

    #[inline(never)]
    fn has<'x>(&mut self, base: &'x Expr, name: &str) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        match self.eval(base)?.as_ref() {
            Value::Object(map) => Ok(Cow::Owned(Value::Bool(map.contains_key(name)))),
            _ => Err(ExprError::Type("has() needs a map")),
        }
    }

    #[inline(never)]
    fn index<'x>(&mut self, base: &'x Expr, index: &'x Expr) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        let base = self.eval(base)?;
        let index = self.eval(index)?;
        select(base, index.as_ref())
    }

    #[inline(never)]
    fn binary_op<'x>(
        &mut self,
        op: BinOp,
        left: &'x Expr,
        right: &'x Expr,
    ) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        let left = self.eval(left)?;
        let right = self.eval(right)?;
        binary(op, &left, &right).map(Cow::Owned)
    }

    #[inline(never)]
    fn function<'x>(
        &mut self,
        function: Function,
        arg: &'x Expr,
    ) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        let value = self.eval(arg)?;
        call(function, &value).map(Cow::Owned)
    }

    #[inline(never)]
    fn method<'x>(
        &mut self,
        method: Method,
        receiver: &'x Expr,
        args: &'x [Expr],
    ) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        let receiver = self.eval(receiver)?;
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push(self.eval(arg)?);
        }
        method_call(method, &receiver, &values).map(Cow::Owned)
    }

    /// `&&` (`short_circuit_on == false`) and `||` (`true`), with CEL's error absorption.
    #[inline(never)]
    fn logical<'x>(
        &mut self,
        left: &'x Expr,
        right: &'x Expr,
        short_circuit_on: bool,
    ) -> Result<Cow<'e, Value>, ExprError>
    where
        'x: 'e,
    {
        let decided = Value::Bool(short_circuit_on);
        let left = self.eval(left).and_then(|v| boolean(&v));
        match left {
            Ok(b) if b == short_circuit_on => Ok(Cow::Owned(decided)),
            Ok(_) => self
                .eval(right)
                .and_then(|v| boolean(&v))
                .map(|b| Cow::Owned(Value::Bool(b))),
            Err(error) => match self.eval(right).and_then(|v| boolean(&v)) {
                Ok(b) if b == short_circuit_on => Ok(Cow::Owned(decided)),
                _ => Err(error),
            },
        }
    }
}

/// Indexing a list by position or a map by key; borrows from the input where it can.
fn select<'e>(base: Cow<'e, Value>, index: &Value) -> Result<Cow<'e, Value>, ExprError> {
    let position = |n: &serde_json::Number| n.as_u64().and_then(|i| usize::try_from(i).ok());
    match (base, index) {
        (Cow::Borrowed(Value::Array(items)), Value::Number(n)) => position(n)
            .and_then(|i| items.get(i))
            .map(Cow::Borrowed)
            .ok_or(ExprError::IndexOutOfRange),
        (Cow::Owned(Value::Array(mut items)), Value::Number(n)) => {
            let i = position(n)
                .filter(|i| *i < items.len())
                .ok_or(ExprError::IndexOutOfRange)?;
            Ok(Cow::Owned(items.swap_remove(i)))
        }
        (Cow::Borrowed(Value::Object(map)), Value::String(key)) => map
            .get(key)
            .map(Cow::Borrowed)
            .ok_or_else(|| ExprError::NoSuchKey(clip(key))),
        (Cow::Owned(Value::Object(mut map)), Value::String(key)) => map
            .remove(key)
            .map(Cow::Owned)
            .ok_or_else(|| ExprError::NoSuchKey(clip(key))),
        _ => Err(ExprError::Type("cannot index this value with that key")),
    }
}

fn boolean(value: &Value) -> Result<bool, ExprError> {
    match value {
        Value::Bool(b) => Ok(*b),
        _ => Err(ExprError::Type("'&&' and '||' need booleans")),
    }
}

fn binary(op: BinOp, left: &Value, right: &Value) -> Result<Value, ExprError> {
    match op {
        BinOp::Eq => Ok(Value::Bool(equal(left, right))),
        BinOp::Ne => Ok(Value::Bool(!equal(left, right))),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let ordering = compare(left, right)?;
            Ok(Value::Bool(match op {
                BinOp::Lt => ordering.is_lt(),
                BinOp::Le => ordering.is_le(),
                BinOp::Gt => ordering.is_gt(),
                _ => ordering.is_ge(),
            }))
        }
        BinOp::In => match right {
            Value::Array(items) => Ok(Value::Bool(items.iter().any(|item| equal(left, item)))),
            Value::Object(map) => match left {
                Value::String(key) => Ok(Value::Bool(map.contains_key(key))),
                _ => Err(ExprError::Type("'in' on a map needs a string key")),
            },
            _ => Err(ExprError::Type("'in' needs a list or a map")),
        },
        BinOp::Add => add(left, right),
        BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => arithmetic(op, left, right),
    }
}

fn add(left: &Value, right: &Value) -> Result<Value, ExprError> {
    match (left, right) {
        (Value::String(a), Value::String(b)) => {
            if a.len() + b.len() > MAX_STRING_BYTES {
                return Err(ExprError::ResultTooLarge);
            }
            Ok(Value::String(format!("{a}{b}")))
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() + b.len() > MAX_LIST_ELEMENTS {
                return Err(ExprError::ResultTooLarge);
            }
            Ok(Value::Array(a.iter().chain(b).cloned().collect()))
        }
        _ => arithmetic(BinOp::Add, left, right),
    }
}

fn arithmetic(op: BinOp, left: &Value, right: &Value) -> Result<Value, ExprError> {
    let (Some(a), Some(b)) = (as_num(left), as_num(right)) else {
        return Err(ExprError::Type("arithmetic needs two numbers"));
    };
    if let (Num::Int(a), Num::Int(b)) = (a, b) {
        let result = match op {
            BinOp::Add => a.checked_add(b),
            BinOp::Sub => a.checked_sub(b),
            BinOp::Mul => a.checked_mul(b),
            BinOp::Div | BinOp::Rem if b == 0 => return Err(ExprError::DivisionByZero),
            BinOp::Div => a.checked_div(b),
            _ => a.checked_rem(b),
        };
        return result.map(Value::from).ok_or(ExprError::Overflow);
    }
    let (a, b) = (to_f64(a), to_f64(b));
    match op {
        BinOp::Add => number_value(a + b),
        BinOp::Sub => number_value(a - b),
        BinOp::Mul => number_value(a * b),
        BinOp::Div if b == 0.0 => Err(ExprError::DivisionByZero),
        BinOp::Div => number_value(a / b),
        _ => Err(ExprError::Type("'%' needs two integers")),
    }
}

fn compare(left: &Value, right: &Value) -> Result<std::cmp::Ordering, ExprError> {
    if let (Some(a), Some(b)) = (as_num(left), as_num(right)) {
        return match (a, b) {
            (Num::Int(a), Num::Int(b)) => Ok(a.cmp(&b)),
            (a, b) => to_f64(a)
                .partial_cmp(&to_f64(b))
                .ok_or(ExprError::Type("numbers are not comparable")),
        };
    }
    match (left, right) {
        (Value::String(a), Value::String(b)) => Ok(a.cmp(b)),
        (Value::Bool(a), Value::Bool(b)) => Ok(a.cmp(b)),
        _ => Err(ExprError::Type("cannot order these values")),
    }
}

/// Structural equality; numbers compare by value across integer and double.
fn equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(_), Value::Number(_)) => match (as_num(left), as_num(right)) {
            (Some(Num::Int(a)), Some(Num::Int(b))) => a == b,
            (Some(a), Some(b)) => to_f64(a) == to_f64(b),
            _ => false,
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| equal(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, x)| b.get(key).is_some_and(|y| equal(x, y)))
        }
        (a, b) => a == b,
    }
}

fn call(function: Function, value: &Value) -> Result<Value, ExprError> {
    match function {
        Function::Size => size(value),
        Function::String => match value {
            Value::Null => Ok(Value::String("null".into())),
            Value::Array(_) | Value::Object(_) => {
                Err(ExprError::Type("string() needs a scalar value"))
            }
            other => scalar_text(other).map(Value::String),
        },
        Function::Int => match value {
            Value::String(s) => s
                .trim()
                .parse::<i64>()
                .map(Value::from)
                .map_err(|_| ExprError::Type("int() could not parse the string")),
            other => match as_num(other) {
                Some(Num::Int(i)) => Ok(Value::from(i)),
                #[allow(clippy::cast_possible_truncation)] // range-checked just above
                Some(Num::Float(f)) if f.is_finite() && f.abs() < 9.2e18 => {
                    Ok(Value::from(f.trunc() as i64))
                }
                Some(Num::Float(_)) => Err(ExprError::Overflow),
                None => Err(ExprError::Type("int() needs a number or a string")),
            },
        },
        Function::Double => match value {
            Value::String(s) => s
                .trim()
                .parse::<f64>()
                .map_err(|_| ExprError::Type("double() could not parse the string"))
                .and_then(number_value),
            other => as_num(other)
                .map(to_f64)
                .ok_or(ExprError::Type("double() needs a number or a string"))
                .and_then(number_value),
        },
    }
}

fn size(value: &Value) -> Result<Value, ExprError> {
    let n = match value {
        Value::String(s) => s.chars().count(),
        Value::Array(a) => a.len(),
        Value::Object(o) => o.len(),
        _ => return Err(ExprError::Type("size() needs a string, list or map")),
    };
    i64::try_from(n)
        .map(Value::from)
        .map_err(|_| ExprError::Overflow)
}

fn method_call(
    method: Method,
    receiver: &Value,
    args: &[Cow<'_, Value>],
) -> Result<Value, ExprError> {
    if method == Method::Size {
        return if args.is_empty() {
            size(receiver)
        } else {
            Err(ExprError::Type("size() takes no arguments"))
        };
    }
    let (Value::String(text), [arg]) = (receiver, args) else {
        return Err(ExprError::Type(
            "this method needs a string and one argument",
        ));
    };
    let Value::String(needle) = arg.as_ref() else {
        return Err(ExprError::Type("this method needs a string argument"));
    };
    Ok(Value::Bool(match method {
        Method::Contains => text.contains(needle.as_str()),
        Method::StartsWith => text.starts_with(needle.as_str()),
        _ => text.ends_with(needle.as_str()),
    }))
}
