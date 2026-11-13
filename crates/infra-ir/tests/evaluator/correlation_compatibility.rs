//! A prior adapter implements the existing required methods but leaves the additive default intact.
use super::*;
use sagat_application::{
    dispatch::EventView,
    execution::{ExpressionFailure, ReconcileAnswer},
    loop_progress::{LoopCheckpointEvaluation, LoopEntryEvaluation},
};
use sagat_domain::execution::CorrelationKey;

struct PriorEvaluator;

// Forward the prior required surface to the real evaluator, deliberately omitting the new method.
macro_rules! prior_methods {
    ($($name:ident($($arg:ident:$kind:ty),*) -> $result:ty;)*) => {
        $(fn $name(&self,$($arg:$kind),*) -> $result { JsonEvaluator.$name($($arg),*) })*
    };
}

impl ExpressionEvaluator for PriorEvaluator {
    prior_methods! {
        condition(expression:&Expression,data:&RunData) -> Result<bool,ExpressionFailure>;
        correlation_key(expression:&Expression,data:&RunData) -> Result<CorrelationKey,ExpressionFailure>;
        idempotency_key(template:&Expression,data:&RunData,node:&NodeId) -> Result<String,ExpressionFailure>;
        child_input(expression:Option<&Expression>,data:&RunData) -> Result<Vec<u8>,ExpressionFailure>;
        run_result(expression:&Expression,data:&RunData) -> Result<Vec<u8>,ExpressionFailure>;
        continuation_value(expression:&Expression,data:&RunData) -> Result<Vec<u8>,ExpressionFailure>;
        task_input(data:&RunData,node:&NodeId) -> Vec<u8>;
        collection_items(expression:&Expression,data:&RunData) -> Result<Vec<Vec<u8>>,ExpressionFailure>;
        collection_return(expression:&Expression,data:&RunData,instance:&ExecutionInstanceId) -> Result<Vec<u8>,ExpressionFailure>;
        collection_member_result(expression:&Expression,data:&RunData,instance:&ExecutionInstanceId,body_result:&[u8]) -> Result<Vec<u8>,ExpressionFailure>;
        loop_entry(initial:&Expression,condition:Option<&Expression>,data:&RunData) -> Result<LoopEntryEvaluation,ExpressionFailure>;
        loop_checkpoint(result:&Expression,carry:&Expression,condition:&Expression,data:&RunData,return_instance:&ExecutionInstanceId) -> Result<LoopCheckpointEvaluation,ExpressionFailure>;
        reconcile_answer(output:&[u8]) -> Result<ReconcileAnswer,ExpressionFailure>;
        event_document(event:&EventView<'_>) -> Vec<u8>;
        event_filter(filter:&Expression,document:&[u8]) -> Result<bool,ExpressionFailure>;
        is_json(bytes:&[u8]) -> bool;
        is_json_object(bytes:&[u8]) -> bool;
    }
}

#[test]
fn prior_adapter_default_delegates_root_and_refuses_otherwise_valid_scoped_calls() {
    let data = RunData {
        input: br#"{"key":"own-input"}"#.to_vec(),
        ..RunData::default()
    };
    let root = ExecutionInstanceId::root(node("notified"));
    let scoped = ExecutionInstanceId::in_path(
        InstancePath::root().child(node("lines"), 0).expect("path"),
        node("notified"),
    );
    for text in ["input.key", "'constant'", "17", "null"] {
        let expression = Expression::parse(text).expect("expression");
        assert_eq!(
            PriorEvaluator.correlation_key_instance(&expression, &data, &root),
            PriorEvaluator.correlation_key(&expression, &data)
        );
    }
    for text in ["input.key", "'constant'"] {
        let expression = Expression::parse(text).expect("expression");
        assert!(PriorEvaluator.correlation_key(&expression, &data).is_ok());
        assert!(
            JsonEvaluator
                .correlation_key_instance(&expression, &data, &scoped)
                .is_ok()
        );
        assert_eq!(
            PriorEvaluator.correlation_key_instance(&expression, &data, &scoped),
            Err(ExpressionFailure(
                "scoped correlation evaluation is unsupported".into()
            ))
        );
    }
}
