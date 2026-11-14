//! The instance-capability registry as the suites use it (`T018-080`, `FR-018-052`): a migrated database, a store for
//! the processes (as the runtime role in the sweep) and one for the operator (always the owner), and the hooks that
//! age a heartbeat by writing `last_seen`, never by waiting.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use sagat_application::instances::{
    Activation, Capability, InstanceKind, InstanceStore, InstanceToken, Registered, Registration,
};
use sagat_domain::identity::Subject;
use sagat_infra_postgres::{FeatureStep, OperatorBounds, PgInstanceStore};
use sqlx::PgPool;
use uuid::Uuid;

use super::{TestDb, as_runtime_role, migrated_db};

/// The bounds of the operator's transaction in the suites that hold it open.
pub const WIDE: OperatorBounds = OperatorBounds {
    lock_timeout: std::time::Duration::from_secs(120),
    statement_timeout: std::time::Duration::from_secs(120),
    step: std::time::Duration::from_secs(120),
};

pub struct Registry {
    pub db: TestDb,
    /// What a control plane or an engine connects as: the runtime role when the sweep is on.
    pub process_pool: PgPool,
    pub process: PgInstanceStore,
    /// The operator's store: always the owner, because the activation functions are not the runtime role's.
    pub operator: PgInstanceStore,
}

pub async fn registry() -> Registry {
    registry_on(migrated_db().await).await
}

pub async fn registry_on(db: TestDb) -> Registry {
    let process_pool = if as_runtime_role() {
        db.runtime_pool_with(6).await
    } else {
        db.pool.clone()
    };
    Registry {
        process: PgInstanceStore::new(process_pool.clone()),
        process_pool,
        // the suites hold the operator's transaction open on purpose (a barrier, a lock watched in pg_stat_activity), so
        // its bounds are wide here; the bounds themselves are tested with their own
        operator: PgInstanceStore::new(db.pool.clone()).with_bounds(WIDE),
        db,
    }
}

/// The token of every process the suites make unless a test says otherwise (a test about ownership makes its own).
pub static TEST_TOKEN: InstanceToken = InstanceToken::from_bytes([9; 32]);

/// A registration with the version the suites use and the token of the suites.
pub fn registration(
    instance: Uuid,
    kind: InstanceKind,
    capabilities: &[Capability],
) -> Registration<'_> {
    registration_with(instance, &TEST_TOKEN, kind, capabilities)
}

/// A registration under a token of the test's own.
pub fn registration_with<'a>(
    instance: Uuid,
    token: &'a InstanceToken,
    kind: InstanceKind,
    capabilities: &'a [Capability],
) -> Registration<'a> {
    Registration {
        instance,
        token,
        kind,
        scheduler_owner: None,
        binary_version: "0.3.0-test",
        capabilities,
    }
}

/// A feature step that does nothing: the activation or the removal alone.
pub fn nothing(_: &mut sqlx::PgConnection) -> FeatureStep<'_> {
    Box::pin(async { Ok(()) })
}

impl Registry {
    /// Registers a new instance and returns its id; panics unless it is accepted.
    pub async fn serving(&self, kind: InstanceKind, capabilities: &[Capability]) -> Uuid {
        let instance = Uuid::new_v4();
        let got = self
            .process
            .register(&registration(instance, kind, capabilities))
            .await
            .expect("register");
        assert!(matches!(got, Registered::Serving { .. }), "{got:?}");
        instance
    }

    /// Makes the instance's last heartbeat `seconds` old, on the database's clock: the test hook that stands in for
    /// waiting.
    pub async fn age(&self, instance: Uuid, seconds: i64) {
        sqlx::query(
            "UPDATE serving_instances SET last_seen = clock_timestamp() - make_interval(secs => $2::float8) \
             WHERE instance_id = $1",
        )
        .bind(instance)
        .bind(seconds)
        .execute(&self.db.pool)
        .await
        .expect("age the heartbeat");
    }

    /// `n` rows written by the owner, as if as many processes had registered, each last seen `age` seconds ago.
    pub async fn seed(&self, n: i32, kind: InstanceKind, age: i64) {
        sqlx::query(
            "INSERT INTO serving_instances (instance_id, kind, scheduler_owner, binary_version, capabilities, started_at, last_seen, token_hash) \
             SELECT gen_random_uuid(), $2, NULL, 'seed', '{}', \
                    clock_timestamp() - make_interval(secs => $3::float8), \
                    clock_timestamp() - make_interval(secs => $3::float8), \
                    sha256(decode(repeat('09', 32), 'hex')) \
               FROM generate_series(1, $1)",
        )
        .bind(n)
        .bind(kind.as_str())
        .bind(age)
        .execute(&self.db.pool)
        .await
        .expect("seed rows");
    }

    pub async fn rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM serving_instances")
            .fetch_one(&self.db.pool)
            .await
            .expect("count")
    }

    pub async fn row_exists(&self, instance: Uuid) -> bool {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM serving_instances WHERE instance_id = $1)")
            .bind(instance)
            .fetch_one(&self.db.pool)
            .await
            .expect("exists")
    }

    pub async fn history(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM platform_requirement_history")
            .fetch_one(&self.db.pool)
            .await
            .expect("count")
    }

    pub async fn epoch_now(&self) -> i64 {
        sqlx::query_scalar("SELECT epoch FROM platform_requirement_epoch")
            .fetch_one(&self.db.pool)
            .await
            .expect("epoch")
    }

    /// Records the requirement of `capability` for `kinds` with no feature step of its own.
    pub async fn require(
        &self,
        capability: Capability,
        kinds: &[InstanceKind],
    ) -> Result<sagat_application::instances::Activated, sagat_application::instances::ChangeRefused>
    {
        let actor = Subject::operator();
        self.operator
            .activate(
                Activation {
                    capability,
                    kinds,
                    actor: &actor,
                },
                nothing,
            )
            .await
    }

    /// Waits until `n` backends wait on a lock (`pg_stat_activity`, not a time), for at most two minutes.
    pub async fn wait_for_lock_waiters(&self, n: i64, stage: &str) {
        super::bounded::within(stage, std::time::Duration::from_secs(120), async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity \
                     WHERE datname = current_database() AND pid <> pg_backend_pid() \
                       AND wait_event_type = 'Lock'",
                )
                .fetch_one(&self.db.pool)
                .await
                .expect("look at the backends");
                if waiting >= n {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await;
    }
}
