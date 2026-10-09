//! Behaviour and bounds of the expression subset (T002-009; FR-002-001, NFR-002-SEC).
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

use sagat_infra_expr::limits::{MAX_DEPTH, MAX_LIST_LITERAL, MAX_SOURCE_BYTES, MAX_TOKENS};
use sagat_infra_expr::{Compiled, Env, ExprError, Template};
use serde_json::{Value, json};

fn input() -> Value {
    json!({
        "paymentId": "p-42",
        "amount": 150,
        "ratio": 0.5,
        "country": "BR",
        "tags": ["a", "b", "c"],
        "items": [{"sku": "x1", "qty": 2}, {"sku": "x2", "qty": 5}],
        "nested": {"deep": {"value": 7}},
        "flag": true,
        "none": null
    })
}

fn env() -> Env {
    Env::new()
        .with("input", input())
        .with("run", json!({"id": "11111111-2222-3333-4444-555555555555"}))
        .with(
            "steps",
            json!({"validate": {"output": {"approved": true, "score": 0.9}}}),
        )
}

fn eval(src: &str) -> Result<Value, ExprError> {
    Compiled::compile(src)?.eval(&env())
}

fn ok(src: &str) -> Value {
    eval(src).unwrap_or_else(|e| panic!("{src:?} failed: {e}"))
}

// ---------------------------------------------------------------- values and operators

#[test]
fn literals() {
    assert_eq!(ok("42"), json!(42));
    assert_eq!(ok("1.5"), json!(1.5));
    assert_eq!(ok("1e3"), json!(1000.0));
    assert_eq!(ok("'it\\'s'"), json!("it's"));
    assert_eq!(ok("\"a\\tb\\n\""), json!("a\tb\n"));
    assert_eq!(ok("true"), json!(true));
    assert_eq!(ok("null"), json!(null));
    assert_eq!(ok("[1, 'x', true]"), json!([1, "x", true]));
    assert_eq!(ok("[]"), json!([]));
    assert_eq!(ok("'é世'"), json!("é世"));
}

#[test]
fn arithmetic_follows_the_usual_precedence() {
    assert_eq!(ok("2 + 3 * 4"), json!(14));
    assert_eq!(ok("(2 + 3) * 4"), json!(20));
    assert_eq!(ok("10 - 4 - 3"), json!(3));
    assert_eq!(ok("2 * 3 % 4"), json!(2));
    assert_eq!(ok("-2 + 3"), json!(1));
    assert_eq!(ok("- -2"), json!(2));
    assert_eq!(ok("7 / 2"), json!(3), "integer division truncates");
    assert_eq!(ok("-7 / 2"), json!(-3), "toward zero");
    assert_eq!(ok("-7 % 3"), json!(-1));
    assert_eq!(ok("input.amount * 2"), json!(300));
    assert_eq!(ok("input.amount + input.nested.deep.value"), json!(157));
}

#[test]
fn integers_and_doubles_promote_when_mixed() {
    assert_eq!(ok("1 + 0.5"), json!(1.5));
    assert_eq!(ok("input.amount * 1.1"), json!(165.0));
    assert_eq!(ok("input.ratio * 4"), json!(2.0));
    assert!(ok("1 < 1.5") == json!(true) && ok("2 == 2.0") == json!(true));
}

#[test]
fn comparisons() {
    assert_eq!(ok("input.amount > 100"), json!(true));
    assert_eq!(ok("input.amount <= 100"), json!(false));
    assert_eq!(ok("'abc' < 'abd'"), json!(true));
    assert_eq!(ok("false < true"), json!(true));
    assert_eq!(ok("input.country == 'BR'"), json!(true));
    assert_eq!(ok("input.country != 'BR'"), json!(false));
    assert_eq!(ok("input.tags == ['a', 'b', 'c']"), json!(true));
}

#[test]
fn equality_across_types_is_false_not_an_error() {
    assert_eq!(ok("1 == 'a'"), json!(false));
    assert_eq!(ok("null == false"), json!(false));
    assert_eq!(ok("input.none == null"), json!(true));
}

#[test]
fn membership() {
    assert_eq!(ok("'b' in input.tags"), json!(true));
    assert_eq!(ok("'z' in input.tags"), json!(false));
    assert_eq!(ok("'paymentId' in input"), json!(true));
    assert_eq!(ok("'missing' in input"), json!(false));
    assert_eq!(ok("input.country in ['BR', 'AR']"), json!(true));
}

#[test]
fn logic_short_circuits_and_absorbs_errors_like_cel() {
    assert_eq!(ok("false && input.missing > 1"), json!(false));
    assert_eq!(
        ok("input.missing > 1 && false"),
        json!(false),
        "an error is absorbed by a decisive false"
    );
    assert_eq!(ok("true || input.missing > 1"), json!(true));
    assert_eq!(ok("input.missing > 1 || true"), json!(true));
    assert!(eval("input.missing > 1 && true").is_err());
    assert!(eval("input.missing > 1 || false").is_err());
    assert_eq!(ok("!false && !(1 > 2)"), json!(true));
}

#[test]
fn ternary_is_right_associative_and_lazy() {
    assert_eq!(ok("input.amount > 100 ? 'big' : 'small'"), json!("big"));
    assert_eq!(ok("false ? 1 : false ? 2 : 3"), json!(3));
    assert_eq!(ok("true ? 1 : input.missing"), json!(1));
    assert!(
        eval("1 ? 2 : 3").is_err(),
        "the condition must be a boolean"
    );
}

#[test]
fn members_indexing_and_has() {
    assert_eq!(ok("input.nested.deep.value"), json!(7));
    assert_eq!(ok("input.tags[1]"), json!("b"));
    assert_eq!(ok("input.items[1].qty"), json!(5));
    assert_eq!(ok("input['paymentId']"), json!("p-42"));
    assert_eq!(ok("steps.validate.output.approved"), json!(true));
    assert_eq!(ok("run.id"), json!("11111111-2222-3333-4444-555555555555"));
    assert_eq!(ok("has(input.paymentId)"), json!(true));
    assert_eq!(ok("has(input.nope)"), json!(false));
    assert_eq!(ok("has(input.nested.deep)"), json!(true));
    assert_eq!(
        ok("has(input.nope) ? input.nope : 'default'"),
        json!("default")
    );
}

#[test]
fn functions_and_methods() {
    assert_eq!(ok("size(input.tags)"), json!(3));
    assert_eq!(ok("input.tags.size()"), json!(3));
    assert_eq!(ok("size('é世')"), json!(2), "characters, not bytes");
    assert_eq!(ok("size(input.nested)"), json!(1));
    assert_eq!(ok("string(input.amount)"), json!("150"));
    assert_eq!(ok("string(true)"), json!("true"));
    assert_eq!(ok("int('42')"), json!(42));
    assert_eq!(ok("int(3.9)"), json!(3));
    assert_eq!(ok("int(-3.9)"), json!(-3));
    assert_eq!(ok("double(3)"), json!(3.0));
    assert_eq!(ok("double('2.5')"), json!(2.5));
    assert_eq!(ok("input.country.contains('R')"), json!(true));
    assert_eq!(ok("input.paymentId.startsWith('p-')"), json!(true));
    assert_eq!(ok("input.paymentId.endsWith('-43')"), json!(false));
    assert_eq!(ok("'a' + 'b' + string(1)"), json!("ab1"));
    assert_eq!(ok("[1, 2] + [3]"), json!([1, 2, 3]));
}

// ---------------------------------------------------------------- runtime errors are typed

#[test]
fn runtime_errors_are_specific() {
    assert_eq!(
        eval("ghost"),
        Err(ExprError::UndeclaredVariable("ghost".into()))
    );
    assert_eq!(
        eval("input.ghost"),
        Err(ExprError::NoSuchKey("ghost".into()))
    );
    assert_eq!(
        eval("input['ghost']"),
        Err(ExprError::NoSuchKey("ghost".into()))
    );
    assert_eq!(eval("input.tags[3]"), Err(ExprError::IndexOutOfRange));
    assert_eq!(eval("input.tags[-1]"), Err(ExprError::IndexOutOfRange));
    assert_eq!(eval("1 / 0"), Err(ExprError::DivisionByZero));
    assert_eq!(eval("1 % 0"), Err(ExprError::DivisionByZero));
    assert_eq!(eval("1.5 / 0"), Err(ExprError::DivisionByZero));
    assert_eq!(eval("9223372036854775807 + 1"), Err(ExprError::Overflow));
    assert_eq!(eval("-9223372036854775807 - 2"), Err(ExprError::Overflow));
    assert_eq!(eval("3037000500 * 3037000500"), Err(ExprError::Overflow));
    assert_eq!(eval("int(1e30)"), Err(ExprError::Overflow));
    for bad in [
        "1 + 'a'",
        "!1",
        "-'a'",
        "'a' < 1",
        "input.amount.field",
        "input.tags.field",
        "1 in 2",
        "1 in input",
        "int('x')",
        "double('x')",
        "size(1)",
        "string([1])",
        "1.5 % 2",
        "input.country.contains(1)",
        "input.amount.contains('a')",
        "has(1.x)",
    ] {
        assert!(eval(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_result_kind_is_checked_by_the_typed_entry_points() {
    let env = env();
    assert_eq!(
        Compiled::compile("input.amount > 1").and_then(|c| c.eval_bool(&env)),
        Ok(true)
    );
    assert!(matches!(
        Compiled::compile("input.amount").and_then(|c| c.eval_bool(&env)),
        Err(ExprError::Type(_))
    ));
    assert_eq!(
        Compiled::compile("input.paymentId").and_then(|c| c.eval_key(&env)),
        Ok("p-42".into())
    );
    assert_eq!(
        Compiled::compile("input.amount").and_then(|c| c.eval_key(&env)),
        Ok("150".into())
    );
    assert_eq!(
        Compiled::compile("2.0").and_then(|c| c.eval_key(&env)),
        Ok("2".into())
    );
    assert_eq!(
        Compiled::compile("2.5").and_then(|c| c.eval_key(&env)),
        Ok("2.5".into())
    );
    assert_eq!(
        Compiled::compile("true").and_then(|c| c.eval_key(&env)),
        Ok("true".into())
    );
    for not_a_key in ["null", "input.tags", "input.nested"] {
        assert_eq!(
            Compiled::compile(not_a_key).and_then(|c| c.eval_key(&env)),
            Err(ExprError::NotScalar),
            "{not_a_key}"
        );
    }
}

#[test]
fn variables_lists_what_the_expression_reads() {
    let names = |src: &str| -> Vec<String> {
        Compiled::compile(src)
            .expect("compiles")
            .variables()
            .into_iter()
            .collect()
    };
    assert_eq!(names("1 + 2"), Vec::<String>::new());
    assert_eq!(names("input.a > 1 && steps.x.output.y"), ["input", "steps"]);
    assert_eq!(
        names("has(run.id) ? input[steps.k] : [input.a].size()"),
        ["input", "run", "steps"]
    );
}

// ---------------------------------------------------------------- what the subset refuses

#[test]
fn comprehension_macros_and_other_functions_are_refused_at_compile_time() {
    for (src, name) in [
        ("input.tags.all(t, t != '')", "all"),
        ("input.tags.exists(t, t == 'a')", "exists"),
        ("input.tags.map(t, t + t)", "map"),
        ("input.tags.filter(t, t != 'a')", "filter"),
        ("input.tags.exists_one(t, t == 'a')", "exists_one"),
        ("input.paymentId.matches('p-.*')", "matches"),
        ("timestamp('2020-01-01T00:00:00Z')", "timestamp"),
        ("type(1)", "type"),
        ("input.paymentId.lowerAscii()", "lowerAscii"),
    ] {
        assert_eq!(
            Compiled::compile(src),
            Err(ExprError::Unsupported(name.into())),
            "{src}"
        );
    }
}

#[test]
fn a_field_may_share_a_name_with_a_refused_function() {
    let env = Env::new().with("input", json!({"map": 1, "filter": 2, "all": 3}));
    assert_eq!(
        Compiled::compile("input.map + input.filter + input.all").and_then(|c| c.eval(&env)),
        Ok(json!(6))
    );
}

#[test]
fn syntax_errors_carry_a_position() {
    for src in [
        "",
        "1 +",
        "(1",
        "1)",
        "[1,",
        "a.",
        "a.1",
        "1 2",
        "a ? b",
        "has(1)",
        "has(a, b)",
        "size()",
        "1 = 2",
        "a & b",
        "a | b",
        "$x",
        "1 ? : 2",
    ] {
        assert!(Compiled::compile(src).is_err(), "{src:?} must not compile");
    }
    assert!(matches!(
        Compiled::compile("1 + )"),
        Err(ExprError::Syntax { position: 4, .. })
    ));
}

#[test]
fn malformed_lexemes_are_reported() {
    assert!(matches!(
        Compiled::compile("'abc"),
        Err(ExprError::UnterminatedString(0))
    ));
    assert!(matches!(
        Compiled::compile("'a\\q'"),
        Err(ExprError::InvalidEscape(2))
    ));
    assert!(matches!(
        Compiled::compile("12abc"),
        Err(ExprError::InvalidNumber(0))
    ));
    assert!(matches!(
        Compiled::compile("99999999999999999999"),
        Err(ExprError::InvalidNumber(0))
    ));
    assert!(matches!(
        Compiled::compile("1 # comment"),
        Err(ExprError::UnexpectedCharacter(2))
    ));
}

// ---------------------------------------------------------------- bounds

#[test]
fn source_length_is_capped() {
    let long = format!("'{}'", "x".repeat(MAX_SOURCE_BYTES));
    assert_eq!(Compiled::compile(&long), Err(ExprError::TooLong));
    let fits = format!("'{}'", "x".repeat(MAX_SOURCE_BYTES - 2));
    assert!(Compiled::compile(&fits).is_ok());
}

#[test]
fn the_token_count_is_capped() {
    // 257 operands and 256 operators: far more than the cap, whatever the token kinds.
    let tokens = vec!["1"; MAX_TOKENS + 1].join("+");
    assert_eq!(Compiled::compile(&tokens), Err(ExprError::TooComplex));
    // Exactly at the cap is accepted: 128 operands and 127 operators = 255 tokens.
    let at_the_cap = vec!["1"; (MAX_TOKENS - 1) / 2 + 1].join("+");
    assert!(Compiled::compile(&at_the_cap).is_ok());
    let over = vec!["1"; (MAX_TOKENS - 1) / 2 + 2].join("+");
    assert_eq!(Compiled::compile(&over), Err(ExprError::TooComplex));
    let fine = vec!["1"; 100].join("+");
    assert_eq!(
        Compiled::compile(&fine).and_then(|c| c.eval(&Env::new())),
        Ok(json!(100))
    );
}

#[test]
fn nesting_depth_is_capped_for_every_shape() {
    let n = MAX_DEPTH;
    let shapes: Vec<(&str, String)> = vec![
        ("parens", format!("{}1{}", "(".repeat(n), ")".repeat(n))),
        ("lists", format!("{}1{}", "[".repeat(n), "]".repeat(n))),
        ("not", format!("{}true", "!".repeat(n + 1))),
        ("neg", format!("{}1", "-".repeat(n + 1))),
        ("ternary", format!("{}2", "true ? 1 : ".repeat(n + 1))),
        (
            "calls",
            format!("{}1{}", "size(".repeat(n + 1), ")".repeat(n + 1)),
        ),
        ("index", format!("a{}", "[0]".repeat(100))),
    ];
    for (name, src) in shapes {
        match Compiled::compile(&src) {
            Err(ExprError::TooDeep | ExprError::TooComplex) => {}
            Ok(_) if name == "index" => {}
            other => panic!("{name}: expected a depth rejection, got {other:?}"),
        }
    }
}

#[test]
fn the_deepest_accepted_expression_fits_in_a_small_stack() {
    // Parsing and evaluating the worst accepted shapes must not need more than a sliver of stack,
    // even in a debug build: the reference implementation overflows 2 MiB at depth 12. Measured
    // worst case with this crate: 192 KiB in a debug build, 32 KiB in release.
    let deepest_parens = format!(
        "{}1{}",
        "(".repeat(MAX_DEPTH / 2 - 1),
        ")".repeat(MAX_DEPTH / 2 - 1)
    );
    let widest_chain = vec!["1"; 120].join("+");
    let left_deep_members = format!("input{}", ".a".repeat(100));
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            for src in [deepest_parens, widest_chain, left_deep_members] {
                let _ = Compiled::compile(&src).map(|c| c.eval(&env()));
            }
        })
        .expect("thread");
    handle.join().expect("no stack overflow on a 512 KiB stack");
}

#[test]
fn list_literals_are_capped() {
    let big = format!("[{}]", vec!["1"; MAX_LIST_LITERAL + 1].join(","));
    assert!(matches!(
        Compiled::compile(&big),
        Err(ExprError::TooComplex)
    ));
}

#[test]
fn built_strings_and_lists_are_capped() {
    // 'x' doubled 17 times would be 128 KiB: the cap stops it long before.
    let mut src = String::from("'xxxxxxxxxxxxxxxx'");
    for _ in 0..4 {
        src = format!("({src} + {src})");
    }
    assert!(Compiled::compile(&src).is_ok());
    let env = Env::new().with("s", Value::String("x".repeat(40_000)));
    let doubled = Compiled::compile("s + s").expect("compiles");
    assert_eq!(doubled.eval(&env), Err(ExprError::ResultTooLarge));
    let list = Env::new().with("l", Value::Array(vec![json!(1); 3000]));
    assert_eq!(
        Compiled::compile("l + l").expect("compiles").eval(&list),
        Err(ExprError::ResultTooLarge)
    );
}

#[test]
fn error_messages_do_not_echo_unbounded_input() {
    let long = "x".repeat(500);
    let errors = [
        eval(&long).expect_err("undeclared"),
        eval(&format!("input.{long}")).expect_err("no key"),
        Compiled::compile(&format!("{long}(1)")).expect_err("unsupported"),
    ];
    for error in errors {
        assert!(error.to_string().len() < 120, "{error}");
    }
}

// ---------------------------------------------------------------- templates

#[test]
fn templates_render_scalars_into_text() {
    let t = Template::compile("reserve:${run.id}").expect("template");
    assert_eq!(
        t.render(&env()),
        Ok("reserve:11111111-2222-3333-4444-555555555555".into())
    );
    let t = Template::compile("submit:${input.paymentId}:${input.amount}:${input.flag}")
        .expect("template");
    assert_eq!(t.render(&env()), Ok("submit:p-42:150:true".into()));
    assert_eq!(
        Template::compile("plain").expect("t").render(&env()),
        Ok("plain".into())
    );
    assert_eq!(
        Template::compile("").expect("t").render(&env()),
        Ok(String::new())
    );
    let t = Template::compile("${input.country == 'BR' ? 'brazil' : 'other'}-x").expect("t");
    assert_eq!(t.render(&env()), Ok("brazil-x".into()));
}

#[test]
fn a_brace_inside_a_string_does_not_end_the_part() {
    let t = Template::compile("${'}'}!").expect("template");
    assert_eq!(t.render(&Env::new()), Ok("}!".into()));
    let t = Template::compile("${\"a\\\"}\"}").expect("template");
    assert_eq!(t.render(&Env::new()), Ok("a\"}".into()));
}

#[test]
fn templates_reject_what_cannot_become_a_key() {
    assert!(matches!(
        Template::compile("a${input.paymentId"),
        Err(ExprError::UnterminatedTemplate(1))
    ));
    assert!(matches!(
        Template::compile("${1 +}"),
        Err(ExprError::Syntax { .. })
    ));
    assert_eq!(
        Template::compile("${input.tags}")
            .expect("t")
            .render(&env()),
        Err(ExprError::NotScalar)
    );
    assert!(matches!(
        Template::compile("${input.ghost}")
            .expect("t")
            .render(&env()),
        Err(ExprError::NoSuchKey(_))
    ));
    let many = "${1}".repeat(9);
    assert_eq!(
        Template::compile(&many),
        Err(ExprError::TooManyTemplateParts)
    );
    assert!(Template::compile(&"${1}".repeat(8)).is_ok());
    let long = Template::compile(&format!("{}${{input.paymentId}}", "k".repeat(510))).expect("t");
    assert_eq!(long.render(&env()), Err(ExprError::ResultTooLarge));
    assert_eq!(
        Template::compile(&"x".repeat(MAX_SOURCE_BYTES + 1)),
        Err(ExprError::TooLong)
    );
}

#[test]
fn template_variables_are_the_union_of_the_parts() {
    let t = Template::compile("${run.id}/${input.a}/${steps.x.output.y}/${1}").expect("t");
    assert_eq!(
        t.variables().into_iter().collect::<Vec<_>>(),
        ["input", "run", "steps"]
    );
}

#[test]
fn an_error_never_names_a_key_that_came_from_the_data() {
    // `input.table[input.customerToken]`: the key is a value of the run, here a credential.
    let env = Env::new().with(
        "input",
        json!({"table": {"known": 1}, "customerToken": "tok_live_51Hx9SECRETSECRETSECRET"}),
    );
    let compiled = Compiled::compile("input.table[input.customerToken] == 1").expect("compiles");
    let error = compiled.eval(&env).expect_err("the key is missing");
    let text = format!("{error} {error:?}");
    assert!(
        !text.contains("tok_live") && !text.contains("SECRET"),
        "{text}"
    );
    // A key written in the expression is the author's own text and may be named.
    let named = Compiled::compile("input.table.missing").expect("compiles");
    let error = named.eval(&env).expect_err("missing");
    assert!(format!("{error}").contains("missing"), "{error}");
}
