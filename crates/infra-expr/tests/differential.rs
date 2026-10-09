//! Differential tests: wherever the reference CEL implementation succeeds, the bounded subset must
//! return the same value (T002-009). Where the reference fails the subset is free to succeed (it
//! promotes mixed numbers) or fail; those cases are not compared.
//!
//! The reference runs on a thread with a large stack because it recurses without bound.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use cel::{Context, Program, Value as CelValue};
use proptest::prelude::*;
use sagat_infra_expr::{Compiled, Env};
use serde_json::{Value, json};

/// A value both implementations can be reduced to.
#[derive(Debug, Clone, PartialEq)]
enum Norm {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<Norm>),
    Map(BTreeMap<String, Norm>),
}

fn from_json(v: &Value) -> Norm {
    match v {
        Value::Null => Norm::Null,
        Value::Bool(b) => Norm::Bool(*b),
        Value::Number(n) => Norm::Num(n.as_f64().expect("finite")),
        Value::String(s) => Norm::Str(s.clone()),
        Value::Array(a) => Norm::List(a.iter().map(from_json).collect()),
        Value::Object(o) => Norm::Map(o.iter().map(|(k, v)| (k.clone(), from_json(v))).collect()),
    }
}

#[allow(clippy::cast_precision_loss)]
fn from_cel(v: &CelValue) -> Option<Norm> {
    Some(match v {
        CelValue::Null => Norm::Null,
        CelValue::Bool(b) => Norm::Bool(*b),
        CelValue::Int(i) => Norm::Num(*i as f64),
        CelValue::UInt(u) => Norm::Num(*u as f64),
        CelValue::Float(f) => Norm::Num(*f),
        CelValue::String(s) => Norm::Str(s.to_string()),
        CelValue::List(items) => Norm::List(items.iter().map(from_cel).collect::<Option<_>>()?),
        _ => return None,
    })
}

fn inputs() -> Vec<Value> {
    vec![
        json!({
            "paymentId": "p-42", "amount": 150, "ratio": 0.5, "country": "BR", "neg": -7,
            "tags": ["a", "b", "c"], "items": [{"sku": "x1", "qty": 2}, {"sku": "x2", "qty": 5}],
            "nested": {"deep": {"value": 7}}, "flag": true, "off": false, "none": null,
            "text": "héllo wörld"
        }),
        json!({"paymentId": "", "amount": 0, "ratio": 2.0, "country": "US", "neg": 0,
               "tags": [], "items": [], "nested": {"deep": {"value": 0}}, "flag": false,
               "off": true, "none": null, "text": ""}),
    ]
}

/// `None` when the reference cannot evaluate the expression (error, unsupported value kind, panic).
fn reference(src: &str, input: &Value) -> Option<Norm> {
    let src = src.to_owned();
    let input = input.clone();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let program = Program::compile(&src).ok()?;
            let mut ctx = Context::default();
            ctx.add_variable("input", input).ok()?;
            from_cel(&program.execute(&ctx).ok()?)
        })
        .expect("thread")
        .join()
        .ok()
        .flatten()
}

fn ours(src: &str, input: &Value) -> Result<Norm, sagat_infra_expr::ExprError> {
    let env = Env::new().with("input", input.clone());
    Compiled::compile(src)
        .and_then(|c| c.eval(&env))
        .map(|v| from_json(&v))
}

/// Compares one expression on every input; returns how many comparisons the reference allowed.
fn compare(src: &str) -> usize {
    let mut compared = 0;
    for input in inputs() {
        if let Some(expected) = reference(src, &input) {
            compared += 1;
            match ours(src, &input) {
                Ok(actual) => assert_eq!(actual, expected, "{src}  with {input}"),
                Err(error) => panic!(
                    "{src}: the reference returns {expected:?} but we fail: {error}  (input {input})"
                ),
            }
        }
    }
    compared
}

const TABLE: &[&str] = &[
    // arithmetic
    "1 + 2",
    "2 + 3 * 4",
    "(2 + 3) * 4",
    "10 - 4 - 3",
    "7 / 2",
    "-7 / 2",
    "7 % 3",
    "-7 % 3",
    "2 * 3 % 4",
    "- -2",
    "-(1 + 2)",
    "1 + 2 + 3 + 4 + 5",
    "1 - 2 - 3",
    "100 / 10 / 5",
    "9223372036854775807 + 1",
    "1 / 0",
    "1 % 0",
    "1.5 + 2.5",
    "1.5 * 2.0",
    "5.0 / 2.0",
    "1.0 - 0.25",
    "-1.5",
    "input.amount - 150",
    "input.amount * 0",
    "input.neg + 7",
    "input.neg * -1",
    "input.ratio * 2.0",
    "input.ratio + 0.25",
    "input.nested.deep.value + 1",
    // comparisons
    "1 < 2",
    "2 <= 2",
    "3 > 4",
    "4 >= 4",
    "1 == 1",
    "1 != 1",
    "1 == 1.0",
    "'a' < 'b'",
    "'b' < 'a'",
    "'abc' == 'abc'",
    "'abc' != 'abd'",
    "'a' <= 'a'",
    "true == true",
    "true != false",
    "false < true",
    "1.5 < 2.5",
    "1.5 == 1.5",
    "input.amount > 100",
    "input.amount < 100",
    "input.amount == 150",
    "input.amount >= 150",
    "input.ratio < 1.0",
    "input.country == 'BR'",
    "input.country != 'BR'",
    "input.paymentId == 'p-42'",
    "input.tags == ['a', 'b', 'c']",
    "input.tags == ['a']",
    "input.none == null",
    "input.flag == true",
    "input.off == false",
    "input.neg < 0",
    "input.items[0].sku == 'x1'",
    "input.items[1].qty > input.items[0].qty",
    // logic
    "true && true",
    "true && false",
    "false || true",
    "false || false",
    "!true",
    "!false",
    "!(!true)",
    "true && !false",
    "!(1 > 2) && 2 > 1",
    "input.flag && input.off",
    "input.flag || input.off",
    "!input.off",
    "input.amount > 100 && input.country == 'BR'",
    "input.amount > 1000 || input.country == 'BR'",
    "false && input.missing",
    "true || input.missing",
    "input.missing && false",
    "input.missing || true",
    "input.missing && true",
    "input.missing || false",
    "1 > 2 || 2 > 1 && 3 > 2",
    "(1 > 2 || 2 > 1) && 3 > 2",
    // ternary
    "true ? 1 : 2",
    "false ? 1 : 2",
    "1 < 2 ? 'a' : 'b'",
    "false ? 1 : false ? 2 : 3",
    "true ? 1 : input.missing",
    "input.amount > 100 ? input.country : 'none'",
    // members, index, has
    "input.paymentId",
    "input.nested",
    "input.nested.deep",
    "input.nested.deep.value",
    "input.tags",
    "input.tags[0]",
    "input.tags[2]",
    "input.tags[3]",
    "input.items[1].sku",
    "input['paymentId']",
    "input.nested['deep'].value",
    "has(input.paymentId)",
    "has(input.nope)",
    "has(input.nested.deep)",
    "has(input.nested.nope)",
    "has(input.nope) ? input.nope : 'dflt'",
    "input.missing",
    "input.nested.missing",
    // in
    "'a' in input.tags",
    "'z' in input.tags",
    "1 in [1, 2, 3]",
    "4 in [1, 2, 3]",
    "'paymentId' in input",
    "'nope' in input",
    "input.country in ['BR', 'AR']",
    "input.country in []",
    "2 in [1, 2.0]",
    // functions and methods
    "size(input.tags)",
    "input.tags.size()",
    "size('hello')",
    "size('')",
    "size(input.text)",
    "input.text.size()",
    "size(input.nested)",
    "size(input.items) > 1",
    "string(1)",
    "string(true)",
    "string(1.5)",
    "string(input.paymentId)",
    "int('42')",
    "int(3.9)",
    "int(-3.9)",
    "int(7)",
    "double(3)",
    "double('2.5')",
    "double(2.5)",
    "'hello'.contains('ell')",
    "'hello'.contains('z')",
    "'hello'.startsWith('he')",
    "'hello'.startsWith('lo')",
    "'hello'.endsWith('lo')",
    "input.country.contains('R')",
    "input.paymentId.startsWith('p-')",
    "input.text.endsWith('wörld')",
    "input.paymentId.size()",
    // strings and lists
    "'a' + 'b'",
    "'a' + 'b' + 'c'",
    "'x' + string(1)",
    "'é' + '世'",
    "[1, 2] + [3]",
    "[] + []",
    "[1, 2, 3].size()",
    "size([1, 2, 3]) == 3",
    "[1, 2] == [1, 2]",
    "[1, 2] == [2, 1]",
    "'a' + input.country",
    "input.tags + ['d']",
    "size(input.tags + ['d'])",
    // mixed
    "size(input.tags) > 2 && 'a' in input.tags",
    "has(input.nested.deep) && input.nested.deep.value == 7",
    "input.items[0].qty * 10 + 1",
    "input.amount > 100 ? 'big' : 'small'",
    "(input.amount > 100 ? 1 : 2) + 5",
    "[input.amount, 1].size()",
];

#[test]
fn the_table_agrees_with_the_reference_wherever_the_reference_succeeds() {
    let compared: usize = TABLE.iter().map(|src| compare(src)).sum();
    // A vacuous pass (the reference rejecting everything) would prove nothing.
    assert!(compared > 250, "only {compared} comparisons were possible");
}

// ---------------------------------------------------------------- generated expressions

fn leaf() -> impl Strategy<Value = String> {
    prop_oneof![
        (0i64..40).prop_map(|n| n.to_string()),
        (0i64..40).prop_map(|n| format!("{n}.5")),
        Just("true".to_owned()),
        Just("false".to_owned()),
        Just("'a'".to_owned()),
        Just("'bc'".to_owned()),
        Just("''".to_owned()),
        Just("input.amount".to_owned()),
        Just("input.neg".to_owned()),
        Just("input.ratio".to_owned()),
        Just("input.country".to_owned()),
        Just("input.flag".to_owned()),
        Just("input.off".to_owned()),
        Just("input.tags".to_owned()),
        Just("input.paymentId".to_owned()),
        Just("input.missing".to_owned()),
        Just("input.nested.deep.value".to_owned()),
    ]
}

fn expression() -> impl Strategy<Value = String> {
    leaf().prop_recursive(4, 40, 3, |inner| {
        let op = prop_oneof![
            Just("+"),
            Just("-"),
            Just("*"),
            Just("/"),
            Just("%"),
            Just("<"),
            Just("<="),
            Just(">"),
            Just(">="),
            Just("=="),
            Just("!="),
            Just("&&"),
            Just("||"),
            Just("in"),
        ];
        prop_oneof![
            (inner.clone(), op, inner.clone()).prop_map(|(a, op, b)| format!("({a} {op} {b})")),
            // Parenthesised on purpose: the reference folds `!!x` into `x` without a type check (a
            // deviation from the CEL specification, where `!!'a'` is a type error), so a bare
            // double negation would make the oracle wrong, not this crate.
            inner.clone().prop_map(|a| format!("!({a})")),
            inner.clone().prop_map(|a| format!("-({a})")),
            (inner.clone(), inner.clone(), inner.clone())
                .prop_map(|(c, a, b)| format!("({c} ? {a} : {b})")),
            inner.clone().prop_map(|a| format!("size({a})")),
            inner.clone().prop_map(|a| format!("string({a})")),
            inner.clone().prop_map(|a| format!("[{a}, 1]")),
            (inner.clone(), inner).prop_map(|(a, b)| format!("{a}.contains({b})")),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn random_expressions_agree_with_the_reference(src in expression()) {
        // `compare` asserts equality whenever the reference succeeds.
        compare(&src);
    }

    #[test]
    fn arbitrary_text_never_panics_and_terminates(src in "\\PC{0,1200}") {
        let env = Env::new().with("input", inputs().remove(0));
        if let Ok(compiled) = Compiled::compile(&src) {
            let _ = compiled.eval(&env);
        }
    }

    #[test]
    fn expression_shaped_noise_never_panics(src in "[a-z0-9_.()\\[\\]'\"+*/%<>=!&|?:, -]{0,300}") {
        let env = Env::new().with("input", inputs().remove(0));
        if let Ok(compiled) = Compiled::compile(&src) {
            let _ = compiled.eval(&env);
        }
    }
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300)
}
