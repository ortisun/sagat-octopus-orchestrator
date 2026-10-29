//! Pin the released evaluator's event representation before sharing its implementation.
use sagat_application::{dispatch::EventView, execution::ExpressionEvaluator};
use sagat_infra_ir::JsonEvaluator;

#[test]
fn released_event_document_encoding_keeps_exact_bytes_and_null_fields() {
    let view = EventView {
        id: "evt-1",
        source: "provider",
        event_type: "payment.confirmed",
        subject: Some("order-1"),
        correlation_key: Some("payment-1"),
        occurred_at: None,
        received_at: "2026-10-27T00:00:00Z",
        data: br#"{"amount":12,"accepted":true}"#,
    };
    assert_eq!(JsonEvaluator.event_document(&view),br#"{"data":{"accepted":true,"amount":12},"event":{"correlationKey":"payment-1","id":"evt-1","occurredAt":null,"receivedAt":"2026-10-27T00:00:00Z","source":"provider","subject":"order-1","type":"payment.confirmed"}}"#);
}
