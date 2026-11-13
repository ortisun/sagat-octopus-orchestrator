//! Canonical event bytes are shared by expression evaluation and transactional retention.
use sagat_application::dispatch::{EventView, event_document};

fn view(data: &[u8]) -> EventView<'_> {
    EventView {
        id: "evt-1",
        source: "provider",
        event_type: "payment.confirmed",
        subject: Some("order-1"),
        correlation_key: Some("payment-1"),
        occurred_at: None,
        received_at: "2026-10-27T00:00:00Z",
        data,
    }
}

#[test]
fn canonical_event_document_has_the_existing_envelope_shape_and_exact_json_bytes() {
    assert_eq!(event_document(&view(br#"{"amount":12,"accepted":true}"#)),br#"{"data":{"accepted":true,"amount":12},"event":{"correlationKey":"payment-1","id":"evt-1","occurredAt":null,"receivedAt":"2026-10-27T00:00:00Z","source":"provider","subject":"order-1","type":"payment.confirmed"}}"#);
}

#[test]
fn canonical_event_document_preserves_legacy_null_fallback_for_invalid_stored_json() {
    assert_eq!(event_document(&view(b"not json")),br#"{"data":null,"event":{"correlationKey":"payment-1","id":"evt-1","occurredAt":null,"receivedAt":"2026-10-27T00:00:00Z","source":"provider","subject":"order-1","type":"payment.confirmed"}}"#);
}
