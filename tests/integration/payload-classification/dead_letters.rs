#![allow(clippy::expect_used)]

use sagat_application::clock::SystemClock;
use sagat_application::event_payloads::{
    DeadLetterData, DeadLetterStructure, EventPayloadService, EventPayloadUseCase, EventReveal,
};
use sagat_domain::connectors::ConnectorId;
use sagat_domain::identity::Role;
use sagat_infra_ir::JsonPayloadProjector;
use uuid::Uuid;

use super::support::engine::{ctx, world};

#[tokio::test]
async fn dead_letter_defaults_are_value_free_and_lists_ignore_reveal() {
    let w = world().await;
    let connector = ConnectorId::new(Uuid::new_v4());
    let letter = Uuid::new_v4();
    sqlx::query("INSERT INTO event_connectors (id,tenant_id,namespace_id,slug,kind,settings,rate_per_second,created_by) VALUES ($1,$2,$3,'letters-a','KAFKA','{\"servers\":[\"tls://broker:9092\"],\"topic\":\"a\",\"startFrom\":\"LATEST\"}',10,$4)").bind(connector.as_uuid()).bind(w.admin.tenant_id().as_uuid()).bind(w.namespace.id().as_uuid()).bind(w.admin.subject().as_str()).execute(&w.db.pool).await.expect("connector");
    sqlx::query("INSERT INTO connector_dead_letters (id,tenant_id,namespace_id,connector_id,position,reason,headers,body,truncated) VALUES ($1,$2,$3,$4,'p/0/1','BAD_ENVELOPE','{\"authorization\":\"header-pii\"}',$5,true)").bind(letter).bind(w.admin.tenant_id().as_uuid()).bind(w.namespace.id().as_uuid()).bind(connector.as_uuid()).bind(br#"{"canary":"body-pii""#.as_slice()).execute(&w.db.pool).await.expect("letter");
    let reader = w.member("letter-reader", Role::Developer).await;
    let service = EventPayloadService::new(w.store.clone(), SystemClock, JsonPayloadProjector);
    let detail = service
        .prepare_dead_letter(
            &reader,
            w.namespace.id(),
            connector,
            Some(letter),
            EventReveal::Structure,
            &ctx(),
        )
        .await
        .unwrap();
    let answer = service.dead_letter(detail).await.unwrap();
    assert!(matches!(
        answer.data,
        DeadLetterData::Structure(DeadLetterStructure::Opaque)
    ));
    let debug = format!("{answer:?}");
    assert!(!debug.contains("header-pii"));
    assert!(!debug.contains("body-pii"));
    let list = service
        .prepare_dead_letter(
            &reader,
            w.namespace.id(),
            connector,
            None,
            EventReveal::Pii,
            &ctx(),
        )
        .await
        .unwrap();
    let listed = service.dead_letters(list, None, 10).await.unwrap();
    assert!(matches!(listed[0].data, DeadLetterData::Structure(_)));
    assert!(listed[0].letter.headers.is_empty());
    assert!(listed[0].letter.body.as_slice().is_empty());
}
