//! Real provider retirement, crash recovery and active-attempt retention.
use super::support::engine::ctx;
use super::world;
use sagat_application::{
    clock::SystemClock,
    error::AppError,
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
    secret_cleanup::{SecretCleanupService, SecretCleanupStore},
    secret_provider::{ProviderError, ProviderFuture, ProviderReference, SecretProvider},
};
use sagat_domain::secrets::SecretValue;
use std::sync::Arc;
use uuid::Uuid;

struct Down(Arc<dyn SecretProvider>);
impl SecretProvider for Down {
    fn create<'a>(
        &'a self,
        r: &'a ProviderReference,
        v: &'a SecretValue,
    ) -> ProviderFuture<'a, ()> {
        self.0.create(r, v)
    }
    fn read<'a>(&'a self, r: &'a ProviderReference) -> ProviderFuture<'a, Option<SecretValue>> {
        self.0.read(r)
    }
    fn delete<'a>(&'a self, _: &'a ProviderReference) -> ProviderFuture<'a, ()> {
        Box::pin(async { Err(ProviderError::Unavailable) })
    }
}

pub async fn age_operations(pool: &sqlx::PgPool) {
    // Only the isolated fixture owner adjusts immutable creation time to exercise the real DB clock.
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql("ALTER TABLE secret_provider_operations DISABLE TRIGGER secret_provider_immutable;
        UPDATE secret_provider_operations SET created_at=clock_timestamp()-interval '25 hours',activated_at=CASE WHEN activated_at IS NULL THEN NULL ELSE clock_timestamp()-interval '25 hours' END;
        ALTER TABLE secret_provider_operations ENABLE ALWAYS TRIGGER secret_provider_immutable;").execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}
async fn expire_claim(pool: &sqlx::PgPool, reference: &ProviderReference) {
    sqlx::query("UPDATE secret_provider_operations SET cleanup_expires_at=clock_timestamp()-interval '1 second' WHERE tenant_id=$1 AND id=$2")
        .bind(reference.tenant.as_uuid()).bind(reference.operation).execute(pool).await.unwrap();
}
pub async fn run(provider: Arc<dyn SecretProvider>) {
    let mut w = world().await;
    w.store = runtime_store(&w).await;
    let ns = w.namespace.id();
    let secrets = ManagedSecretService::new(
        w.store.clone(),
        SystemClock,
        provider.clone(),
        SecretValue::new(vec![42; 32]).unwrap(),
    )
    .unwrap();
    let write = |name| ManagedSecretWrite {
        name,
        value: SecretValue::new(b"retirement-canary".to_vec()).unwrap(),
        expected_version: 0,
        request_id: Uuid::new_v4(),
    };
    let request = write("expired-intent");
    let intent = secrets
        .prepare(&w.admin, ns, &request, &ctx())
        .await
        .unwrap();
    provider
        .create(&intent.reference, &request.value)
        .await
        .unwrap();
    assert!(
        w.store
            .claim_provider_cleanup(&intent.reference)
            .await
            .unwrap()
            .is_none()
    );
    age_operations(&w.db.pool).await;
    let first = w
        .store
        .claim_provider_cleanup(&intent.reference)
        .await
        .unwrap()
        .unwrap();
    assert!(
        w.store
            .claim_provider_cleanup(&intent.reference)
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        secrets
            .reconcile(&w.admin, ns, intent.reference.operation, &ctx())
            .await,
        Err(AppError::NotFound)
    ));
    let audit:i64=sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='secret.provider_abandoned'")
        .bind(w.admin.tenant_id().as_uuid()).fetch_one(&w.db.pool).await.unwrap();
    assert_eq!(audit, 1, "abandonment is durable before provider I/O");
    let charged: i64 =
        sqlx::query_scalar("SELECT content_bytes FROM tenant_retained_usage WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    // Destroy succeeded; response/finalization was lost. Retry must retain a permanent CAS fence.
    provider.delete(&first.reference).await.unwrap();
    expire_claim(&w.db.pool, &intent.reference).await;
    let second = w
        .store
        .claim_provider_cleanup(&intent.reference)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first.lease, second.lease);
    assert!(!w.store.finish_provider_cleanup(&first).await.unwrap());
    provider.delete(&second.reference).await.unwrap();
    assert!(w.store.finish_provider_cleanup(&second).await.unwrap());
    assert!(!w.store.finish_provider_cleanup(&second).await.unwrap());
    let after: i64 =
        sqlx::query_scalar("SELECT content_bytes FROM tenant_retained_usage WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert_eq!(charged - after, request.value.len() as i64);
    assert!(
        provider
            .create(&intent.reference, &request.value)
            .await
            .is_err()
    );
    assert!(provider.read(&intent.reference).await.unwrap().is_none());
    assert!(
        secrets
            .reconcile_request(&w.admin, ns, request.request_id, &ctx())
            .await
            .is_err()
    );
    // A crash before any provider receipt also retires the absent path before releasing its quota.
    let absent = secrets
        .prepare(&w.admin, ns, &write("absent-intent"), &ctx())
        .await
        .unwrap();
    age_operations(&w.db.pool).await;
    let failed = SecretCleanupService::new(w.store.clone(), Arc::new(Down(provider.clone())))
        .run_batch(32)
        .await
        .unwrap();
    assert_eq!((failed.retired, failed.deferred), (0, 1));
    let retained: i64 =
        sqlx::query_scalar("SELECT count(*) FROM secret_provider_operations WHERE tenant_id=$1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    assert_eq!(retained, 1);
    let cleanup = SecretCleanupService::new(w.store.clone(), provider.clone());
    assert_eq!(cleanup.run_batch(32).await.unwrap().retired, 0);
    expire_claim(&w.db.pool, &absent.reference).await;
    assert_eq!(cleanup.run_batch(32).await.unwrap().retired, 1);
    assert!(
        provider
            .create(&absent.reference, &request.value)
            .await
            .is_err()
    );
    assert!(cleanup.run_batch(0).await.is_err());
    assert!(cleanup.run_batch(129).await.is_err());
    assert!(
        w.store
            .provider_cleanup_candidates(32)
            .await
            .unwrap()
            .is_empty()
    );
}

pub async fn runtime_store(w: &super::support::engine::World) -> sagat_infra_postgres::PgRunStore {
    let pool = sqlx::PgPool::connect(&w.db.url_for(
        super::support::RUNTIME_ROLE,
        super::support::RUNTIME_ROLE_PASSWORD,
    ))
    .await
    .unwrap();
    sagat_infra_postgres::PgIdentityStore::new(pool.clone())
        .verify_least_privilege()
        .await
        .unwrap();
    sagat_infra_postgres::PgRunStore::new(pool, w.cache.clone())
}
