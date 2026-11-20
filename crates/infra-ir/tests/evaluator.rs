#![allow(clippy::expect_used)]

use sagat_application::execution::{ExpressionEvaluator, RunData, ScopeInput};
use sagat_domain::execution::{ExecutionInstanceId, InstancePath};
use sagat_domain::workflow::{Expression, NodeId};
use sagat_infra_ir::JsonEvaluator;
use serde_json::{Value, json};

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node")
}

#[test]
fn scoped_conditions_read_only_their_exact_iteration_and_local_outputs() {
    let mut data = RunData {
        input: br#"{"enabled":true}"#.to_vec(),
        ..RunData::default()
    };
    data.insert_root_output(node("reserve"), br#"{"index":99}"#.to_vec());
    let expression =
        Expression::parse("input.enabled && item > 0 && steps.reserve.output.index == index")
            .expect("predicate");
    for (index, item) in [(0, 1), (1, -1)] {
        let path = InstancePath::root()
            .child(node("lines"), index)
            .expect("path");
        data.scope_inputs.insert(
            path.clone(),
            ScopeInput::Collection {
                index,
                item: item.to_string().into_bytes(),
            },
        );
        data.insert_output(
            ExecutionInstanceId::in_path(path, node("reserve")),
            serde_json::to_vec(&json!({"index":index})).expect("output"),
        );
    }
    for index in [0, 1] {
        let instance = ExecutionInstanceId::in_path(
            InstancePath::root()
                .child(node("lines"), index)
                .expect("path"),
            node("choose"),
        );
        assert_eq!(
            JsonEvaluator
                .condition_instance(&expression, &data, &instance)
                .expect("scoped boolean"),
            index == 0
        );
    }
    let absent = ExecutionInstanceId::in_path(
        InstancePath::root().child(node("lines"), 2).expect("path"),
        node("choose"),
    );
    assert!(
        JsonEvaluator
            .condition_instance(&expression, &data, &absent)
            .is_err(),
        "a missing scoped input cannot fall back to another iteration"
    );
}

#[test]
fn root_condition_evaluation_keeps_its_context_and_non_boolean_failure() {
    let data = RunData {
        input: br#"{"enabled":true}"#.to_vec(),
        ..RunData::default()
    };
    let root = ExecutionInstanceId::root(node("choose"));
    for text in [
        "input.enabled",
        "true",
        "17",
        "input.absent",
        "node.id == null",
    ] {
        let expression = Expression::parse(text).expect("expression");
        assert_eq!(
            JsonEvaluator.condition_instance(&expression, &data, &root),
            JsonEvaluator.condition(&expression, &data)
        );
    }
}

#[test]
fn root_task_context_remains_byte_identical_when_scoped_outputs_exist() {
    let reserve = node("reserve");
    let scoped = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(node("items"), 0)
            .expect("valid path"),
        reserve.clone(),
    );
    let mut data = RunData {
        input: br#"{"order":7}"#.to_vec(),
        ..RunData::default()
    };
    data.insert_root_output(reserve, br#"{"reservation":"root-reservation"}"#.to_vec());
    data.insert_output(
        scoped,
        br#"{"reservation":"iteration-reservation"}"#.to_vec(),
    );

    let context = JsonEvaluator.task_input(&data, &node("ship"));

    assert_eq!(
        context,
        br#"{"input":{"order":7},"node":"ship","run":{"id":null},"steps":{"reserve":{"output":{"reservation":"root-reservation"}}}}"#
    );
}

#[test]
fn continuation_value_uses_the_locked_run_snapshot() {
    let data = RunData {
        input: br#"{"next":{"segment":2}}"#.to_vec(),
        ..RunData::default()
    };
    let expression = Expression::parse("input.next").expect("expression");

    assert_eq!(
        JsonEvaluator
            .continuation_value(&expression, &data)
            .expect("selected continuation input"),
        br#"{"segment":2}"#
    );
}

#[test]
fn collection_instances_receive_only_their_item_and_local_outputs() {
    let first_path = InstancePath::root()
        .child(node("items"), 0)
        .expect("valid path");
    let second_path = InstancePath::root()
        .child(node("items"), 1)
        .expect("valid path");
    let first = ExecutionInstanceId::in_path(first_path.clone(), node("ship"));
    let second = ExecutionInstanceId::in_path(second_path.clone(), node("ship"));
    let mut data = RunData {
        input: br#"{"order":7}"#.to_vec(),
        state: Some(br#"{"region":"south"}"#.to_vec()),
        state_revision: Some(0),
        ..RunData::default()
    };
    data.scope_inputs.insert(
        first_path.clone(),
        ScopeInput::Collection {
            index: 0,
            item: br#"{"sku":"first"}"#.to_vec(),
        },
    );
    data.scope_inputs.insert(
        second_path.clone(),
        ScopeInput::Collection {
            index: 1,
            item: br#"{"sku":"second"}"#.to_vec(),
        },
    );
    data.insert_output(
        ExecutionInstanceId::in_path(first_path, node("reserve")),
        br#"{"code":"first-output"}"#.to_vec(),
    );
    data.insert_output(
        ExecutionInstanceId::in_path(second_path, node("reserve")),
        br#"{"code":"second-output"}"#.to_vec(),
    );

    let first_context: Value = serde_json::from_slice(
        &JsonEvaluator
            .task_input_instance(&data, &first)
            .expect("first context"),
    )
    .expect("json context");
    let second_context: Value = serde_json::from_slice(
        &JsonEvaluator
            .task_input_instance(&data, &second)
            .expect("second context"),
    )
    .expect("json context");

    assert_eq!(first_context["item"], json!({"sku": "first"}));
    assert_eq!(first_context["index"], 0);
    assert_eq!(first_context["state"], json!({"region": "south"}));
    assert_eq!(
        first_context["steps"]["reserve"]["output"],
        json!({"code": "first-output"})
    );
    assert_eq!(second_context["item"], json!({"sku": "second"}));
    assert_eq!(second_context["index"], 1);
    assert_eq!(
        second_context["steps"]["reserve"]["output"],
        json!({"code": "second-output"})
    );
}

#[test]
fn scoped_idempotency_uses_the_exact_iteration_context() {
    let path = InstancePath::root()
        .child(node("items"), 3)
        .expect("valid path");
    let instance = ExecutionInstanceId::in_path(path.clone(), node("charge"));
    let mut data = RunData::default();
    data.scope_inputs.insert(
        path.clone(),
        ScopeInput::Collection {
            index: 3,
            item: br#"{"paymentId":"pay-7"}"#.to_vec(),
        },
    );
    data.insert_output(
        ExecutionInstanceId::in_path(path, node("reserve")),
        br#"{"reservationId":"res-9"}"#.to_vec(),
    );
    let template = Expression::parse(
        "charge:${item.paymentId}:${index}:${steps.reserve.output.reservationId}",
    )
    .expect("template expression");

    assert_eq!(
        JsonEvaluator
            .idempotency_key_instance(&template, &data, &instance)
            .expect("scoped key"),
        "charge:pay-7:3:res-9"
    );
}

#[test]
fn scoped_materialization_fails_closed_without_an_admission_input() {
    let instance = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(node("items"), 0)
            .expect("valid path"),
        node("ship"),
    );
    let error = JsonEvaluator
        .task_input_instance(
            &RunData {
                input: b"{}".to_vec(),
                ..RunData::default()
            },
            &instance,
        )
        .expect_err("missing admitted input must not fall back to root context");
    assert!(error.to_string().contains("iteration input is unavailable"));
}

#[test]
fn loop_instance_receives_its_iteration_and_carry() {
    let path = InstancePath::root()
        .child(node("retry"), 4)
        .expect("valid path");
    let instance = ExecutionInstanceId::in_path(path.clone(), node("attempt"));
    let mut data = RunData {
        input: b"{}".to_vec(),
        ..RunData::default()
    };
    data.scope_inputs.insert(
        path,
        ScopeInput::Loop {
            iteration: 4,
            carry: br#"{"remaining":2}"#.to_vec(),
        },
    );

    let context: Value = serde_json::from_slice(
        &JsonEvaluator
            .task_input_instance(&data, &instance)
            .expect("loop context"),
    )
    .expect("json context");
    assert_eq!(context["iteration"], 4);
    assert_eq!(context["carry"], json!({"remaining": 2}));
}

#[test]
fn loop_decisions_bind_the_condition_to_the_new_carry() {
    let evaluator = JsonEvaluator;
    let entry = evaluator
        .loop_entry(
            &Expression::parse("input.start").expect("initial"),
            Some(&Expression::parse("carry < 3").expect("condition")),
            &RunData {
                input: br#"{"start":1}"#.to_vec(),
                ..RunData::default()
            },
        )
        .expect("entry decision");
    assert_eq!(entry.carry, b"1");
    assert_eq!(entry.condition, Some(true));

    let path = InstancePath::root()
        .child(node("poll"), 0)
        .expect("iteration path");
    let returned = ExecutionInstanceId::in_path(path.clone(), node("returned"));
    let mut data = RunData {
        input: br#"{"start":1}"#.to_vec(),
        ..RunData::default()
    };
    data.scope_inputs.insert(
        path.clone(),
        ScopeInput::Loop {
            iteration: 0,
            carry: b"1".to_vec(),
        },
    );
    data.insert_output(
        ExecutionInstanceId::in_path(path, node("work")),
        b"2".to_vec(),
    );
    let checkpoint = evaluator
        .loop_checkpoint(
            &Expression::parse("steps.work.output").expect("result"),
            &Expression::parse("bodyResult + 1").expect("carry"),
            &Expression::parse("carry >= 3").expect("condition"),
            &data,
            &returned,
        )
        .expect("checkpoint decision");
    assert_eq!(checkpoint.result, b"2");
    assert_eq!(checkpoint.carry, b"3");
    assert!(checkpoint.condition);
}

#[test]
fn loop_checkpoint_fails_without_the_exact_iteration_context() {
    let error = JsonEvaluator
        .loop_checkpoint(
            &Expression::parse("bodyResult").expect("result"),
            &Expression::parse("bodyResult").expect("carry"),
            &Expression::parse("carry == 1").expect("condition"),
            &RunData::default(),
            &ExecutionInstanceId::in_path(
                InstancePath::root()
                    .child(node("poll"), 0)
                    .expect("iteration path"),
                node("returned"),
            ),
        )
        .expect_err("missing loop context must fail closed");
    assert!(error.to_string().contains("iteration input is unavailable"));
}

#[test]
fn collection_expressions_keep_item_order_and_exact_scope() {
    let evaluator = JsonEvaluator;
    let path = InstancePath::root()
        .child(node("items"), 1)
        .expect("valid path");
    let returned = ExecutionInstanceId::in_path(path.clone(), node("returned"));
    let mut data = RunData {
        input: br#"{"items":[{"sku":"first"},{"sku":"second"}]}"#.to_vec(),
        ..RunData::default()
    };
    data.scope_inputs.insert(
        path.clone(),
        ScopeInput::Collection {
            index: 1,
            item: br#"{"sku":"second"}"#.to_vec(),
        },
    );
    data.insert_output(
        ExecutionInstanceId::in_path(path, node("work")),
        br#"{"amount":7}"#.to_vec(),
    );

    assert_eq!(
        evaluator
            .collection_items(
                &Expression::parse("input.items").expect("items expression"),
                &data,
            )
            .expect("items"),
        [
            br#"{"sku":"first"}"#.to_vec(),
            br#"{"sku":"second"}"#.to_vec()
        ]
    );
    assert_eq!(
        evaluator
            .collection_return(
                &Expression::parse("steps.work.output").expect("return expression"),
                &data,
                &returned,
            )
            .expect("return"),
        br#"{"amount":7}"#
    );
    assert_eq!(
        evaluator
            .collection_member_result(
                &Expression::parse("bodyResult.amount").expect("result expression"),
                &data,
                &returned,
                br#"{"amount":7}"#,
            )
            .expect("mapped result"),
        b"7"
    );
}

#[test]
fn scoped_correlation_reads_its_own_item_index_and_local_outputs() {
    let mut data = RunData {
        input: br#"{"key":"root-input"}"#.to_vec(),
        ..RunData::default()
    };
    data.insert_root_output(node("reserve"), br#"{"key":"root-output"}"#.to_vec());
    for index in [0, 1] {
        let path = InstancePath::root()
            .child(node("lines"), index)
            .expect("path");
        data.scope_inputs.insert(
            path.clone(),
            ScopeInput::Collection {
                index,
                item: serde_json::to_vec(&json!({"key":format!("item-{index}")})).expect("item"),
            },
        );
        data.insert_output(
            ExecutionInstanceId::in_path(path, node("reserve")),
            serde_json::to_vec(&json!({"key":format!("local-{index}")})).expect("output"),
        );
    }
    for index in [0, 1] {
        let instance = ExecutionInstanceId::in_path(
            InstancePath::root()
                .child(node("lines"), index)
                .expect("path"),
            node("notified"),
        );
        for (text, expected) in [
            ("item.key", format!("item-{index}")),
            ("steps.reserve.output.key", format!("local-{index}")),
            ("index", index.to_string()),
            (
                "index == 0 ? item.key : steps.reserve.output.key",
                if index == 0 {
                    "item-0".into()
                } else {
                    "local-1".into()
                },
            ),
        ] {
            let key = JsonEvaluator
                .correlation_key_instance(
                    &Expression::parse(text).expect("expression"),
                    &data,
                    &instance,
                )
                .expect("exact scoped correlation");
            assert_eq!(key.as_str(), expected);
        }
    }
}

#[test]
fn root_correlation_instance_keeps_all_legacy_scalar_and_failure_semantics() {
    let mut data = RunData {
        input: br#"{"key":"root-input"}"#.to_vec(),
        ..RunData::default()
    };
    data.insert_root_output(node("reserve"), br#"{"key":"root-output"}"#.to_vec());
    let root = ExecutionInstanceId::root(node("notified"));
    for text in [
        "input.key",
        "steps.reserve.output.key",
        "17",
        "true",
        "null",
        "[1]",
        "input.absent",
        "node.id",
    ] {
        let expression = Expression::parse(text).expect("expression");
        assert_eq!(
            JsonEvaluator.correlation_key_instance(&expression, &data, &root),
            JsonEvaluator.correlation_key(&expression, &data)
        );
    }
}

#[test]
fn scoped_correlation_never_falls_back_to_another_paths_item_or_output() {
    let path = InstancePath::root().child(node("lines"), 0).expect("path");
    let other = InstancePath::root().child(node("lines"), 1).expect("other");
    let instance = ExecutionInstanceId::in_path(path.clone(), node("notified"));
    let mut data = RunData {
        input: br#"{"key":"root-input"}"#.to_vec(),
        ..RunData::default()
    };
    data.scope_inputs.insert(
        other.clone(),
        ScopeInput::Collection {
            index: 1,
            item: br#"{"key":"other-item"}"#.to_vec(),
        },
    );
    data.insert_root_output(node("reserve"), b"malformed-root-output".to_vec());
    data.insert_output(
        ExecutionInstanceId::in_path(other, node("reserve")),
        b"malformed-sibling-output".to_vec(),
    );
    for text in ["item.key", "index", "steps.reserve.output.key"] {
        assert!(
            JsonEvaluator
                .correlation_key_instance(
                    &Expression::parse(text).expect("expression"),
                    &data,
                    &instance
                )
                .is_err(),
            "{text} cannot fall back"
        );
    }
    data.scope_inputs.insert(
        path.clone(),
        ScopeInput::Collection {
            index: 0,
            item: br#"{"key":"own-item"}"#.to_vec(),
        },
    );
    assert_eq!(
        JsonEvaluator
            .correlation_key_instance(
                &Expression::parse("item.key").expect("expression"),
                &data,
                &instance
            )
            .expect("own item")
            .as_str(),
        "own-item"
    );
    data.insert_output(
        ExecutionInstanceId::in_path(path, node("reserve")),
        br#"{"key":"own-output"}"#.to_vec(),
    );
    assert_eq!(
        JsonEvaluator
            .correlation_key_instance(
                &Expression::parse("steps.reserve.output.key").expect("expression"),
                &data,
                &instance
            )
            .expect("own output")
            .as_str(),
        "own-output"
    );
}

#[test]
fn loop_correlation_uses_its_own_iteration_and_carry_without_collection_fallback() {
    let path = InstancePath::root().child(node("poll"), 3).expect("path");
    let instance = ExecutionInstanceId::in_path(path.clone(), node("notified"));
    let mut data = RunData::default();
    data.scope_inputs.insert(
        path,
        ScopeInput::Loop {
            iteration: 3,
            carry: br#"{"key":"carry-3"}"#.to_vec(),
        },
    );
    for (text, expected) in [("carry.key", "carry-3"), ("iteration", "3")] {
        assert_eq!(
            JsonEvaluator
                .correlation_key_instance(
                    &Expression::parse(text).expect("expression"),
                    &data,
                    &instance
                )
                .expect("loop key")
                .as_str(),
            expected
        );
    }
    assert!(
        JsonEvaluator
            .correlation_key_instance(
                &Expression::parse("item.key").expect("expression"),
                &data,
                &instance
            )
            .is_err()
    );
}

#[test]
fn scoped_correlation_keeps_root_scalar_bounds_and_redacts_invalid_payload_values() {
    let path = InstancePath::root().child(node("lines"), 0).expect("path");
    let instance = ExecutionInstanceId::in_path(path.clone(), node("notified"));
    let root_expression = Expression::parse("input.value").expect("root expression");
    let expression = Expression::parse("item.value").expect("expression");
    let marker = "sensitive-correlation-test-marker";
    for value in [
        json!(null),
        json!([]),
        json!({"hidden":marker}),
        json!(""),
        json!("a\nb"),
        json!("x".repeat(256)),
        json!("x".repeat(257)),
        json!("é".repeat(128)),
        json!("é".repeat(129)),
        json!(42),
        json!(1.5),
        json!(true),
    ] {
        let bytes = serde_json::to_vec(&json!({"value":value})).expect("payload");
        let mut data = RunData {
            input: bytes.clone(),
            ..RunData::default()
        };
        data.scope_inputs.insert(
            path.clone(),
            ScopeInput::Collection {
                index: 0,
                item: bytes,
            },
        );
        let expected = JsonEvaluator.correlation_key(&root_expression, &data);
        let actual = JsonEvaluator.correlation_key_instance(&expression, &data, &instance);
        assert_eq!(actual, expected);
        if let Err(error) = actual {
            assert!(!error.to_string().contains(marker));
        }
    }
}

#[test]
fn scoped_correlation_evaluates_requested_run_fields_lazily_without_claiming_admission() {
    let instance = ExecutionInstanceId::in_path(
        InstancePath::root().child(node("lines"), 7).expect("path"),
        node("notified"),
    );
    let run = sagat_domain::execution::RunId::generate();
    let data = RunData {
        run_id: Some(run),
        input: br#"{"key":"own-input"}"#.to_vec(),
        state: Some(br#"{"key":"own-state"}"#.to_vec()),
        state_revision: Some(3),
        ..RunData::default()
    };
    assert!(data.scope_inputs.is_empty());
    for (text, expected) in [
        ("'constant'", "constant".to_owned()),
        ("input.key", "own-input".to_owned()),
        ("state.key", "own-state".to_owned()),
        ("run.id", run.to_string()),
    ] {
        assert_eq!(
            JsonEvaluator
                .correlation_key_instance(
                    &Expression::parse(text).expect("expression"),
                    &data,
                    &instance
                )
                .expect("requested run context")
                .as_str(),
            expected
        );
    }
    for text in ["item.key", "carry.key", "index", "iteration"] {
        assert!(
            JsonEvaluator
                .correlation_key_instance(
                    &Expression::parse(text).expect("expression"),
                    &data,
                    &instance
                )
                .is_err(),
            "missing iteration context for {text}"
        );
    }
}

#[path = "evaluator/correlation_compatibility.rs"]
mod correlation_compatibility;

#[test]
fn scoped_correlation_errors_do_not_echo_dynamic_lookup_values() {
    let marker = "sensitive-correlation-lookup-marker";
    let path = InstancePath::root().child(node("lines"), 0).expect("path");
    let instance = ExecutionInstanceId::in_path(path.clone(), node("notified"));
    let mut data = RunData {
        input: b"{}".to_vec(),
        ..RunData::default()
    };
    data.scope_inputs.insert(
        path,
        ScopeInput::Collection {
            index: 0,
            item: serde_json::to_vec(&json!({"lookup":marker})).expect("item"),
        },
    );
    for text in ["input[item.lookup]", "steps[item.lookup].output"] {
        let failure = JsonEvaluator
            .correlation_key_instance(
                &Expression::parse(text).expect("expression"),
                &data,
                &instance,
            )
            .expect_err("missing dynamic key");
        assert!(
            !failure.to_string().contains(marker),
            "scoped correlation error cannot echo a value from the payload"
        );
    }
}
