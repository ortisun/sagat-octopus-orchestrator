#![allow(clippy::expect_used)]

use sagat_application::clock::SystemClock;
use sagat_application::event_payloads::{
    DeadLetterData, DeadLetterStructure, EventPayloadError, EventPayloadService,
    EventPayloadUseCase, EventReveal,
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

#[tokio::test]
async fn empty_dead_letter_pages_recheck_current_event_read_authority() {
    let w = world().await;
    let connector = ConnectorId::new(Uuid::new_v4());
    let empty_connector = ConnectorId::new(Uuid::new_v4());
    for (id, slug) in [
        (connector, "letters-page"),
        (empty_connector, "letters-empty"),
    ] {
        sqlx::query("INSERT INTO event_connectors (id,tenant_id,namespace_id,slug,kind,settings,rate_per_second,created_by) VALUES ($1,$2,$3,$4,'KAFKA','{\"servers\":[\"tls://broker:9092\"],\"topic\":\"a\",\"startFrom\":\"LATEST\"}',10,$5)")
            .bind(id.as_uuid()).bind(w.admin.tenant_id().as_uuid()).bind(w.namespace.id().as_uuid()).bind(slug).bind(w.admin.subject().as_str()).execute(&w.db.pool).await.expect("connector");
    }
    let letter = Uuid::new_v4();
    sqlx::query("INSERT INTO connector_dead_letters (id,tenant_id,namespace_id,connector_id,position,reason,headers,body) VALUES ($1,$2,$3,$4,'p/0/2','BAD_ENVELOPE','{}','{}')")
        .bind(letter).bind(w.admin.tenant_id().as_uuid()).bind(w.namespace.id().as_uuid()).bind(connector.as_uuid()).execute(&w.db.pool).await.expect("letter");
    let reader = w.member("demoted-letter-reader", Role::Developer).await;
    let service = EventPayloadService::new(w.store.clone(), SystemClock, JsonPayloadProjector);
    let empty = service
        .prepare_dead_letter(
            &reader,
            w.namespace.id(),
            empty_connector,
            None,
            EventReveal::Structure,
            &ctx(),
        )
        .await
        .unwrap();
    let empty_page = service
        .prepare_dead_letter(
            &reader,
            w.namespace.id(),
            connector,
            None,
            EventReveal::Structure,
            &ctx(),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE memberships SET role='worker' WHERE tenant_id=$1 AND subject=$2")
        .bind(w.admin.tenant_id().as_uuid())
        .bind(reader.subject().as_str())
        .execute(&w.db.pool)
        .await
        .unwrap();
    for result in [
        service.dead_letters(empty, None, 10).await,
        service
            .dead_letters(empty_page, Some(Uuid::nil()), 10)
            .await,
    ] {
        assert!(matches!(
            result,
            Err(EventPayloadError::Application(
                sagat_application::error::AppError::Forbidden { .. }
            ))
        ));
    }
}
