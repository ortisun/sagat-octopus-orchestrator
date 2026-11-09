//! The instance-capability registry in PostgreSQL (feature 018, `FR-018-052`; migration
//! `20270113000000_instance_registry`).
//!
//! A process calls three `SECURITY DEFINER` functions (register, heartbeat, deregister) with the token it keeps to itself
//! and reads three tables (the instances without the hash of the token); it writes nothing and locks nothing itself,
//! which is what the runtime role can do (it takes no row lock on a table it cannot UPDATE). The operator side
//! (`activate`, `remove`) calls functions the runtime role cannot execute: it runs with the schema-owning role, as
//! `provision-tenant` does, and the feature's own activation runs in the same transaction through the closure it passes.
//!
//! Locks, in the order the functions take them: the requirement epoch row (`FOR SHARE` to register, `FOR UPDATE` to
//! change a requirement), the advisory lock that counts the table, then rows.
//!
//! **A feature's activation step takes no lock that a transaction reading the epoch may hold: it reads, it does not
//! lock.** The step runs while the epoch is held `FOR UPDATE`, and every process that registers waits for the epoch
//! `FOR SHARE` until the transaction ends. The transaction is therefore bounded by [`OperatorBounds`]: a lock wait, a
//! statement and the whole step each have a limit that is shorter than the statement timeout of the processes that wait
//! for it, so that a slow step is cut off and rolled back instead of making every restarting process time out.
//!
//! A connection that fails (or a pool that cannot give one) is [`StoreError::DatabaseUnavailable`]; any other failure
//! keeps the mapping of the rest of the adapter. The distinction is the caller's: only a plain connection failure leaves a
//! process serving after 30 s of failed heartbeats.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::instances::{
    Activated, Activation, Capability, ChangeRefused, Heartbeat, InstanceStore, InstanceToken,
    Registered, Registration, Removed, Requirement, Requirements,
};
use sagat_domain::identity::Subject;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::identity::{col, corrupt, store_error};
use crate::pool::BeginSafely;

/// A step of a feature's own, run inside the transaction of an activation or a removal: the handover row of 018 is
/// written by one, and a removal verifies in one that nothing depends on the requirement. An error rolls the whole
/// transaction back, the requirement with it.
pub type FeatureStep<'c> = Pin<Box<dyn Future<Output = Result<(), StoreError>> + Send + 'c>>;

/// How long the operator's transaction may hold the requirement epoch. The processes that wait for it run with a
/// statement timeout of 5 s by default, so the defaults are shorter than that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperatorBounds {
    /// The longest the transaction waits for a lock (the epoch, while a registration finishes).
    pub lock_timeout: Duration,
    /// The longest any one statement of the transaction, the feature's included, may run.
    pub statement_timeout: Duration,
    /// The longest the feature's step may take in all.
    pub step: Duration,
}

impl Default for OperatorBounds {
    fn default() -> Self {
        Self {
            lock_timeout: Duration::from_secs(2),
            statement_timeout: Duration::from_secs(3),
            step: Duration::from_secs(3),
        }
    }
}

#[derive(Clone)]
pub struct PgInstanceStore {
    pool: PgPool,
    bounds: OperatorBounds,
}

/// A failure of the connection: the database cannot be reached, which is not the registry's fault.
fn registry_error(error: sqlx::Error) -> StoreError {
    use sqlx::Error;
    let connection = match &error {
        Error::Io(_)
        | Error::Tls(_)
        | Error::PoolTimedOut
        | Error::PoolClosed
        | Error::WorkerCrashed => true,
        // connection exceptions (08) and the server shutting down or restarting (57P01 to 57P03)
        Error::Database(db) => db.code().is_some_and(|code| {
            code.starts_with("08") || matches!(&*code, "57P01" | "57P02" | "57P03")
        }),
        _ => false,
    };
    if connection {
        tracing::warn!(%error, "the database could not be reached by the instance registry");
        StoreError::DatabaseUnavailable
    } else {
        store_error(error)
    }
}

fn millis(duration: Duration) -> String {
    duration.as_millis().to_string()
}

impl PgInstanceStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            bounds: OperatorBounds::default(),
        }
    }

    /// The bounds of the operator's transaction (a test that holds it open on purpose widens them).
    #[must_use]
    pub const fn with_bounds(mut self, bounds: OperatorBounds) -> Self {
        self.bounds = bounds;
        self
    }

    /// Starts the operator's transaction under its bounds.
    async fn operator_transaction(
        &self,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        sqlx::query("SELECT set_config('lock_timeout', $1, true), set_config('statement_timeout', $2, true)")
            .bind(millis(self.bounds.lock_timeout))
            .bind(millis(self.bounds.statement_timeout))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        Ok(tx)
    }

    /// Runs the feature's step within its bound; a step that takes longer is cut off and the transaction is rolled back.
    async fn within_the_step<F>(
        &self,
        tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
        step: F,
    ) -> Result<(), StoreError>
    where
        F: for<'c> FnOnce(&'c mut sqlx::PgConnection) -> FeatureStep<'c> + Send,
    {
        match tokio::time::timeout(self.bounds.step, step(&mut *tx)).await {
            Ok(done) => done,
            Err(_) => Err(StoreError::Unavailable {
                detail: format!(
                    "the activation step of the feature took longer than {} ms and was rolled back",
                    self.bounds.step.as_millis()
                ),
            }),
        }
    }

    /// Records that every instance of `activation.kinds` must list `activation.capability`: one transaction that
    /// locks the requirement epoch `FOR UPDATE`, verifies the live instances, inserts the requirement, moves the
    /// epoch and writes the history, then runs `feature` on the same connection and commits. If the same requirement
    /// was recorded before, nothing is done (`feature` does not run: it ran in the transaction that recorded it).
    ///
    /// The functions it calls are not the runtime role's: run it with the schema-owning role. `feature` reads, it does
    /// not lock (see the module documentation) and is cut off after [`OperatorBounds::step`].
    ///
    /// # Errors
    /// [`ChangeRefused::InstancesLackCapability`] when a live instance of the kinds lacks the capability,
    /// [`ChangeRefused::KindsDiffer`] when it is recorded for other kinds, [`ChangeRefused::Store`] when the registry
    /// or `feature` fails. In every case nothing was recorded.
    pub async fn activate<F>(
        &self,
        activation: Activation<'_>,
        feature: F,
    ) -> Result<Activated, ChangeRefused>
    where
        F: for<'c> FnOnce(&'c mut sqlx::PgConnection) -> FeatureStep<'c> + Send,
    {
        if activation.kinds.is_empty() {
            return Err(StoreError::StateConflict("an activation names at least one kind").into());
        }
        let kinds: Vec<&str> = activation.kinds.iter().map(|k| k.as_str()).collect();
        let mut tx = self.operator_transaction().await?;
        let row = sqlx::query(
            "SELECT out_outcome, out_epoch, out_missing, out_examples \
             FROM platform_requirement_activate($1, $2, $3)",
        )
        .bind(activation.capability.as_str())
        .bind(&kinds)
        .bind(activation.actor.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let outcome: String = col(&row, "out_outcome")?;
        match outcome.as_str() {
            "RECORDED" => {
                let epoch: i64 = col(&row, "out_epoch")?;
                self.within_the_step(&mut tx, feature).await?;
                tx.commit().await.map_err(store_error)?;
                Ok(Activated::Recorded { epoch })
            }
            "ALREADY_RECORDED" => Ok(Activated::AlreadyRecorded),
            "KINDS_DIFFER" => Err(ChangeRefused::KindsDiffer),
            "INSTANCES_LACK_CAPABILITY" => {
                let missing: i32 = col(&row, "out_missing")?;
                Err(ChangeRefused::InstancesLackCapability {
                    missing: u32::try_from(missing).map_err(|_| corrupt("missing count"))?,
                    examples: col::<Vec<Uuid>>(&row, "out_examples")?,
                })
            }
            _ => Err(corrupt("activation outcome").into()),
        }
    }

    /// Runs the feature's own activation in the operator's transaction with the requirement epoch held `FOR UPDATE`, without
    /// recording a requirement: for a feature whose requirement was recorded before and whose own activation did not happen
    /// (the handover of the cron triggers, when `schedules-v1` was recorded by hand). The same bounds, the same rule that the
    /// step reads and does not lock what a registering process may hold.
    ///
    /// # Errors
    /// [`ChangeRefused::Store`] when the registry or the step fails; nothing was changed.
    pub async fn run_locked<F>(&self, feature: F) -> Result<(), ChangeRefused>
    where
        F: for<'c> FnOnce(&'c mut sqlx::PgConnection) -> FeatureStep<'c> + Send,
    {
        let mut tx = self.operator_transaction().await?;
        sqlx::query("SELECT platform_requirement_lock()")
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        self.within_the_step(&mut tx, feature).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(())
    }

    /// The same command in reverse. Takes the epoch lock first, then runs `verify` (the feature checks that nothing
    /// depends on the requirement; an error leaves the requirement where it is), then removes the requirement and
    /// moves the epoch. A requirement that is not recorded is `Removed::NotRecorded` and `verify` does not run.
    ///
    /// # Errors
    /// [`ChangeRefused::Store`] when the registry or `verify` fails; nothing was removed.
    pub async fn remove<F>(
        &self,
        capability: Capability,
        actor: &Subject,
        verify: F,
    ) -> Result<Removed, ChangeRefused>
    where
        F: for<'c> FnOnce(&'c mut sqlx::PgConnection) -> FeatureStep<'c> + Send,
    {
        let mut tx = self.operator_transaction().await?;
        sqlx::query("SELECT platform_requirement_lock()")
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let recorded: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM platform_requirements WHERE capability = $1)",
        )
        .bind(capability.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if !recorded {
            return Ok(Removed::NotRecorded);
        }
        self.within_the_step(&mut tx, verify).await?;
        let row =
            sqlx::query("SELECT out_outcome, out_epoch FROM platform_requirement_remove($1, $2)")
                .bind(capability.as_str())
                .bind(actor.as_str())
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
        let outcome: String = col(&row, "out_outcome")?;
        if outcome != "REMOVED" {
            return Err(corrupt("removal outcome").into());
        }
        let epoch: i64 = col(&row, "out_epoch")?;
        tx.commit().await.map_err(store_error)?;
        Ok(Removed::Removed { epoch })
    }
}

impl InstanceStore for PgInstanceStore {
    async fn register(&self, registration: &Registration<'_>) -> Result<Registered, StoreError> {
        let capabilities: Vec<&str> = registration
            .capabilities
            .iter()
            .map(|c| c.as_str())
            .collect();
        let row = sqlx::query(
            "SELECT out_outcome, out_epoch, out_missing \
             FROM instance_register($1, $2, $3, $4, $5, $6)",
        )
        .bind(registration.instance)
        .bind(registration.token.as_bytes().as_slice())
        .bind(registration.kind.as_str())
        .bind(registration.scheduler_owner)
        .bind(registration.binary_version)
        .bind(&capabilities)
        .fetch_one(&self.pool)
        .await
        .map_err(registry_error)?;
        let outcome: String = col(&row, "out_outcome")?;
        let epoch: i64 = col(&row, "out_epoch")?;
        match outcome.as_str() {
            "REGISTERED" => Ok(Registered::Serving { epoch }),
            "CAPABILITY_MISSING" => Ok(Registered::CapabilityMissing {
                epoch,
                missing: col(&row, "out_missing")?,
            }),
            "FULL" => Ok(Registered::Full { epoch }),
            _ => Err(corrupt("registration outcome")),
        }
    }

    async fn heartbeat(
        &self,
        instance: Uuid,
        token: &InstanceToken,
    ) -> Result<Heartbeat, StoreError> {
        let row = sqlx::query(
            "SELECT out_found, out_epoch, out_live_kinds, out_live_counts \
             FROM instance_heartbeat($1, $2)",
        )
        .bind(instance)
        .bind(token.as_bytes().as_slice())
        .fetch_one(&self.pool)
        .await
        .map_err(registry_error)?;
        let kinds: Vec<String> = col(&row, "out_live_kinds")?;
        let counts: Vec<i32> = col(&row, "out_live_counts")?;
        Ok(Heartbeat {
            found: col(&row, "out_found")?,
            epoch: col(&row, "out_epoch")?,
            live: kinds
                .into_iter()
                .zip(counts)
                .map(|(kind, n)| Ok((kind, u32::try_from(n).map_err(|_| corrupt("live count"))?)))
                .collect::<Result<_, StoreError>>()?,
        })
    }

    async fn deregister(&self, instance: Uuid, token: &InstanceToken) -> Result<bool, StoreError> {
        sqlx::query_scalar("SELECT instance_deregister($1, $2)")
            .bind(instance)
            .bind(token.as_bytes().as_slice())
            .fetch_one(&self.pool)
            .await
            .map_err(registry_error)
    }

    async fn epoch(&self) -> Result<i64, StoreError> {
        sqlx::query_scalar("SELECT epoch FROM platform_requirement_epoch")
            .fetch_one(&self.pool)
            .await
            .map_err(registry_error)
    }

    async fn requirements(&self) -> Result<Requirements, StoreError> {
        // one statement, so one snapshot: the epoch is the one the requirements were read under
        let rows = sqlx::query(
            "SELECT e.epoch, r.capability, r.kinds \
             FROM platform_requirement_epoch e LEFT JOIN platform_requirements r ON true \
             ORDER BY r.capability",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(registry_error)?;
        let mut required = Vec::new();
        let mut epoch = 0;
        for row in &rows {
            epoch = row.try_get("epoch").map_err(store_error)?;
            let capability: Option<String> = row.try_get("capability").map_err(store_error)?;
            let kinds: Option<Vec<String>> = row.try_get("kinds").map_err(store_error)?;
            if let (Some(capability), Some(kinds)) = (capability, kinds) {
                required.push(Requirement { capability, kinds });
            }
        }
        Ok(Requirements { epoch, required })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_of_the_connection_is_a_database_that_cannot_be_reached_and_any_other_is_not() {
        for error in [
            sqlx::Error::PoolTimedOut,
            sqlx::Error::PoolClosed,
            sqlx::Error::WorkerCrashed,
            sqlx::Error::Io(std::io::Error::other("connection reset")),
        ] {
            assert_eq!(
                registry_error(error),
                StoreError::DatabaseUnavailable,
                "a lost connection"
            );
        }
        for error in [
            sqlx::Error::RowNotFound,
            sqlx::Error::Protocol("a reply that is not understood".into()),
            sqlx::Error::ColumnNotFound("x".into()),
        ] {
            assert_ne!(
                registry_error(error),
                StoreError::DatabaseUnavailable,
                "not a lost connection"
            );
        }
    }
}
