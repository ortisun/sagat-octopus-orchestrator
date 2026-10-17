//! Operator-only cleanup with independent provider credentials and a whole-command deadline.
use crate::{
    RunError,
    config::{Config, ConfigError, Environment, read_openbao},
    open_pool,
};
use sagat_application::{execution::DefinitionCache, secret_cleanup::SecretCleanupService};
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::{PgIdentityStore, PgRunStore};
use sagat_infra_secrets::openbao::OpenBaoSettings;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub struct CleanupConfig {
    provider: OpenBaoSettings,
    batch: u32,
}
impl CleanupConfig {
    pub fn from_lookup(
        get: &dyn Fn(&str) -> Option<String>,
        environment: Environment,
    ) -> Result<Self, ConfigError> {
        let batch = get("SAGAT_SECRET_CLEANUP_BATCH")
            .map_or(Ok(32), |raw| raw.parse::<u32>())
            .ok()
            .filter(|value| (1..=128).contains(value))
            .ok_or(ConfigError::Invalid {
                name: "SAGAT_SECRET_CLEANUP_BATCH",
                reason: "must be between 1 and 128".into(),
            })?;
        let provider =
            read_openbao(get, environment)?.ok_or(ConfigError::Missing("SAGAT_OPENBAO_ADDRESS"))?;
        Ok(Self { provider, batch })
    }
}

pub async fn run(config: &Config, cleanup: &CleanupConfig) -> Result<(), RunError> {
    let started = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(60), async {
        let provider = cleanup
            .provider
            .provider()
            .await
            .map_err(|_| RunError::Refused("could not configure OpenBao cleanup"))?;
        let pool = open_pool(&config.database, config.database.statement_timeout).await?;
        let result = async {
            if config.environment != Environment::Dev {
                PgIdentityStore::new(pool.clone())
                    .verify_least_privilege()
                    .await?;
            }
            let store =
                PgRunStore::new(pool.clone(), DefinitionCache::new(Arc::new(JsonIrCodec), 1));
            let report = SecretCleanupService::new(store, provider)
                .run_batch(cleanup.batch)
                .await
                .map_err(|_| RunError::Cleanup)?;
            tracing::info!(
                pins_released = report.pins_released,
                retired = report.retired,
                deferred = report.deferred,
                "secret cleanup batch finished"
            );
            if report.deferred > 0 {
                return Err(RunError::Cleanup);
            }
            Ok(())
        }
        .await;
        pool.close().await;
        result
    })
    .await
    .unwrap_or(Err(RunError::Cleanup));
    metrics::histogram!("secret_cleanup_command_seconds", "outcome" => if result.is_ok() { "success" } else { "retry" }).record(started.elapsed().as_secs_f64());
    result
}
