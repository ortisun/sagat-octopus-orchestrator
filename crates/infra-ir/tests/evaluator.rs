#![allow(clippy::expect_used)]

use sagat_application::execution::{ExpressionEvaluator, RunData};
use sagat_domain::execution::{ExecutionInstanceId, InstancePath};
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::JsonEvaluator;

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
