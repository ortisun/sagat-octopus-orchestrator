//! The intent key has a quiesced replacement procedure, not a mixed-key rolling window.
use super::support::engine::{ctx, world};
use sagat_application::{
    clock::SystemClock,
    error::AppError,
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
    secret_provider::SecretProvider,
};
use sagat_domain::secrets::SecretValue;
use std::sync::Arc;
use uuid::Uuid;

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let mut w = world().await;
    w.store = super::cleanup::runtime_store(&w).await;
    let service = |key| {
        ManagedSecretService::new(
            w.store.clone(),
            SystemClock,
            provider.clone(),
            SecretValue::new(vec![key; 32]).unwrap(),
        )
        .unwrap()
    };
    let old = service(42);
    let new = service(43);
    let ns = w.namespace.id();
    let request = Uuid::new_v4();
    let write = || ManagedSecretWrite {
        name: "key-rotation-credential",
        value: SecretValue::new(b"rotation-procedure-canary".to_vec()).unwrap(),
        expected_version: 0,
        request_id: request,
    };
    let intent = old.prepare(&w.admin, ns, &write(), &ctx()).await.unwrap();
    provider
        .create(&intent.reference, &write().value)
        .await
        .unwrap();
    // Replacing the key with an unresolved receipt cannot activate unverified bytes.
    assert!(matches!(
        new.reconcile_request(&w.admin, ns, request, &ctx()).await,
        Err(AppError::StateConflict(_))
    ));
    assert!(
        w.secrets
            .list(&w.admin, ns, &ctx())
            .await
            .unwrap()
            .is_empty()
    );
    // Finish outstanding provider receipts under the old key before stopping all replicas.
    assert_eq!(
        old.reconcile_request(&w.admin, ns, request, &ctx())
            .await
            .unwrap()
            .version,
        1
    );
    // Activated metadata and provider resolution are independent of the old HMAC key.
    assert_eq!(
        new.reconcile_request(&w.admin, ns, request, &ctx())
            .await
            .unwrap()
            .version,
        1
    );
    assert_eq!(
        provider
            .read(&intent.reference)
            .await
            .unwrap()
            .unwrap()
            .expose(),
        write().value.expose()
    );
    // A previous PUT body/key is not reinterpreted under a new binding key.
    assert!(matches!(
        new.put(&w.admin, ns, write(), &ctx()).await,
        Err(AppError::IdempotencyKeyReused)
    ));
    let rotated = new
        .put(
            &w.admin,
            ns,
            ManagedSecretWrite {
                expected_version: 1,
                request_id: Uuid::new_v4(),
                value: SecretValue::new(b"replacement-target-credential".to_vec()).unwrap(),
                ..write()
            },
            &ctx(),
        )
        .await
        .unwrap();
    assert_eq!(rotated.version, 2);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM secret_provider_operations WHERE namespace_id=$1")
            .bind(ns.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert_eq!(
        count, 2,
        "replacement never duplicates the original provider operation"
    );
}
