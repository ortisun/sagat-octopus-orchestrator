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
