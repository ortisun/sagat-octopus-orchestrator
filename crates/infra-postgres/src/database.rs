//! Connection heads, immutable revisions and receipts. No customer database I/O.
use crate::{
    execution::PgRunStore,
    identity::{col, corrupt, insert_audit, store_error},
    pool::BeginSafely,
    quotas::lock_secrets,
};
use sagat_application::{
    database::{ConnectionRecord, ConnectionStore, ConnectionWrite},
    error::StoreError,
};
use sagat_domain::{
    audit::AuditRecord,
    database::{
        ConnectionConfig, ConnectionId, ConnectionStatus, DEFAULT_CONNECTIONS, DatabaseLimits,
        TlsPolicy,
    },
    identity::{NamespaceId, Permission, Principal, Role, Slug},
    secrets::SecretName,
};
use serde_json::{Value, json};
use sqlx::{PgConnection, postgres::PgRow};
use uuid::Uuid;

pub(crate) async fn current_permission(
    conn: &mut PgConnection,
    principal: &Principal,
    permission: Permission,
) -> Result<Role, StoreError> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM memberships WHERE tenant_id=$1 AND subject=$2 FOR SHARE",
    )
    .bind(principal.tenant_id().as_uuid())
    .bind(principal.subject().as_str())
    .fetch_optional(conn)
    .await
    .map_err(store_error)?;
    let role = role
        .and_then(|s| s.parse::<Role>().ok())
        .filter(|r| r.grants(permission))
        .ok_or(StoreError::MissingReference)?;
    Ok(role)
}
fn config_json(config: &ConnectionConfig) -> Value {
    let ConnectionConfig::Postgres {
        host,
        port,
        database,
        tls,
        secret,
        limits,
    } = config;
    let tls = match tls {
        TlsPolicy::VerifyFull { ca_pem } => json!({"mode":"verify-full", "caPem":ca_pem}),
        TlsPolicy::DevelopmentPlaintext => json!({"mode":"development-plaintext"}),
    };
    json!({"driver":"postgresql", "host":host,"port":port,"database":database,"tls":tls,"secretName":secret.as_str(),"limits":{
        "maxStatementBytes":limits.max_statement_bytes,"maxParameters":limits.max_parameters,"maxParameterBytes":limits.max_parameter_bytes,
        "maxRows":limits.max_rows,"maxResultBytes":limits.max_result_bytes,"timeoutMs":limits.timeout_ms,
        "connectTimeoutMs":limits.connect_timeout_ms,"poolSize":limits.pool_size,"idleTimeoutMs":limits.idle_timeout_ms
    }})
}
fn config_from(value: &Value) -> Result<ConnectionConfig, StoreError> {
    let text = |name: &str| {
        value[name]
            .as_str()
            .ok_or_else(|| corrupt("database connection settings"))
    };
    if text("driver")? != "postgresql" {
        return Err(corrupt("database driver"));
    }
    let limit = |name: &str| {
        value["limits"][name]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| corrupt("database limits"))
    };
    let limits = DatabaseLimits {
        max_statement_bytes: limit("maxStatementBytes")?,
        max_parameters: limit("maxParameters")?,
        max_parameter_bytes: limit("maxParameterBytes")?,
        max_rows: limit("maxRows")?,
        max_result_bytes: limit("maxResultBytes")?,
        timeout_ms: limit("timeoutMs")?,
        connect_timeout_ms: limit("connectTimeoutMs")?,
        pool_size: limit("poolSize")?,
        idle_timeout_ms: limit("idleTimeoutMs")?,
    };
    let tls = match value["tls"]["mode"].as_str() {
        Some("verify-full") => TlsPolicy::VerifyFull {
            ca_pem: match &value["tls"]["caPem"] {
                Value::Null => None,
                Value::String(pem) => Some(pem.clone()),
                _ => return Err(corrupt("database trust")),
            },
        },
        Some("development-plaintext") => TlsPolicy::DevelopmentPlaintext,
        _ => return Err(corrupt("database TLS")),
    };
    ConnectionConfig::postgres(
        text("host")?,
        value["port"]
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .ok_or_else(|| corrupt("database port"))?,
        text("database")?,
        tls,
        SecretName::parse(text("secretName")?).map_err(|_| corrupt("database secret reference"))?,
        limits,
    )
    .map_err(|_| corrupt("database connection settings"))
}
fn from_row(row: &PgRow) -> Result<ConnectionRecord, StoreError> {
    Ok(ConnectionRecord {
        id: ConnectionId::new(col(row, "id")?),
        tenant_id: sagat_domain::identity::TenantId::new(col(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col(row, "namespace_id")?),
        name: Slug::parse(&col::<String>(row, "name")?).map_err(|_| corrupt("connection name"))?,
        revision: u32::try_from(col::<i32>(row, "revision")?)
            .map_err(|_| corrupt("connection revision"))?,
        status: match col::<String>(row, "status")?.as_str() {
            "ACTIVE" => ConnectionStatus::Active,
            "PAUSED" => ConnectionStatus::Paused,
            "DELETED" => ConnectionStatus::Deleted,
            _ => return Err(corrupt("connection status")),
        },
        config: config_from(&col::<Value>(row, "config")?)?,
        created_at: col(row, "created_at")?,
        updated_at: col(row, "updated_at")?,
    })
}
macro_rules! select_connections { ($tail:literal) => { concat!("SELECT c.id,c.tenant_id,c.namespace_id,c.name,c.status,c.created_at,c.updated_at,r.revision,r.config FROM database_connections c JOIN database_connection_revisions r ON r.tenant_id=c.tenant_id AND r.namespace_id=c.namespace_id AND r.connection_id=c.id", $tail) }; }

async fn load(
    conn: &mut PgConnection,
    principal: &Principal,
    namespace: NamespaceId,
    name: &Slug,
    revision: Option<u32>,
) -> Result<ConnectionRecord, StoreError> {
    let row = sqlx::query(select_connections!(" WHERE c.tenant_id=$1 AND c.namespace_id=$2 AND c.name=$3 AND r.revision=COALESCE($4,c.current_revision) AND c.status<>'DELETED'"))
        .bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).bind(name.as_str()).bind(revision.map(i64::from))
        .fetch_optional(conn).await.map_err(store_error)?.ok_or(StoreError::MissingReference)?;
    from_row(&row)
}

impl ConnectionStore for PgRunStore {
    async fn write_connection(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        write: &ConnectionWrite,
        audit: &AuditRecord,
    ) -> Result<ConnectionRecord, StoreError> {
        write
            .config
            .validate()
            .map_err(|_| StoreError::StateConflict("invalid connection settings"))?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // Shared lock order with secret writes/deletion. A selected secret cannot disappear between
        // its authority check and committing the connection revision.
        lock_secrets(&mut tx, principal.tenant_id(), namespace).await?;
        current_permission(&mut tx, principal, Permission::DatabaseManage).await?;
        let environment: Option<String> = sqlx::query_scalar(
            "SELECT environment FROM namespaces WHERE tenant_id=$1 AND id=$2 FOR SHARE",
        )
        .bind(principal.tenant_id().as_uuid())
        .bind(namespace.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let environment = environment.ok_or(StoreError::MissingReference)?;
        if matches!(
            &write.config,
            ConnectionConfig::Postgres {
                tls: TlsPolicy::DevelopmentPlaintext,
                ..
            }
        ) && environment != "dev"
        {
            return Err(StoreError::StateConflict(
                "plaintext database transport requires a development namespace",
            ));
        }
        let config = config_json(&write.config);
        let replay = sqlx::query("SELECT r.connection_id,r.revision,r.config,r.status,c.name,c.status AS current_status FROM database_connection_revisions r JOIN database_connections c ON c.tenant_id=r.tenant_id AND c.namespace_id=r.namespace_id AND c.id=r.connection_id WHERE r.tenant_id=$1 AND r.namespace_id=$2 AND r.actor=$3 AND r.request_id=$4")
            .bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).bind(principal.subject().as_str()).bind(write.request_id).fetch_optional(&mut *tx).await.map_err(store_error)?;
        if let Some(row) = replay {
            let revision: i32 = col(&row, "revision")?;
            if col::<String>(&row, "name")? != write.name.as_str()
                || col::<Value>(&row, "config")? != config
                || col::<String>(&row, "status")? != write.status.as_str()
                || i64::from(revision) != i64::from(write.expected_revision) + 1
            {
                return Err(StoreError::IdempotencyKeyReused);
            }
            if col::<String>(&row, "current_status")? == "DELETED" {
                return Err(StoreError::MissingReference);
            }
            let result = load(
                &mut tx,
                principal,
                namespace,
                &write.name,
                Some(revision as u32),
            )
            .await?;
            tx.commit().await.map_err(store_error)?;
            return Ok(result);
        }
        let current: Option<(Uuid,i32,String)> = sqlx::query_as("SELECT id,current_revision,status FROM database_connections WHERE tenant_id=$1 AND namespace_id=$2 AND name=$3 FOR UPDATE")
            .bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).bind(write.name.as_str()).fetch_optional(&mut *tx).await.map_err(store_error)?;
        let revision = current.as_ref().map_or(0, |(_, v, _)| *v as u32);
        if current.as_ref().is_some_and(|(_, _, s)| s == "DELETED") {
            return Err(StoreError::MissingReference);
        }
        if revision != write.expected_revision {
            return Err(StoreError::RevisionConflict {
                current: u64::from(revision),
            });
        }
        if revision >= 1000 {
            return Err(StoreError::QuotaExceeded {
                what: "connection revisions",
            });
        }
        if write.status != ConnectionStatus::Deleted {
            let authority: Option<String> = sqlx::query_scalar("SELECT authority FROM secrets WHERE tenant_id=$1 AND namespace_id=$2 AND name=$3 FOR SHARE")
                .bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).bind(write.config.secret().as_str()).fetch_optional(&mut *tx).await.map_err(store_error)?;
            match authority.as_deref() {
                Some("openbao") => (),
                Some("local") => {
                    return Err(StoreError::StateConflict(
                        "migrate the selected secret to OpenBao first",
                    ));
                }
                _ => return Err(StoreError::MissingReference),
            }
        }
        let id = if let Some((id, _, _)) = current {
            id
        } else {
            if write.status == ConnectionStatus::Deleted {
                return Err(StoreError::MissingReference);
            }
            let (active,total): (i64,i64) = sqlx::query_as("SELECT count(*) FILTER (WHERE status<>'DELETED'),count(*) FROM database_connections WHERE tenant_id=$1 AND namespace_id=$2")
                .bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).fetch_one(&mut *tx).await.map_err(store_error)?;
            if active >= i64::from(DEFAULT_CONNECTIONS) || total >= 1024 {
                return Err(StoreError::QuotaExceeded {
                    what: "database connections",
                });
            }
            let id = Uuid::now_v7();
            sqlx::query("INSERT INTO database_connections(id,tenant_id,namespace_id,name,current_revision,status) VALUES($1,$2,$3,$4,1,$5)")
                .bind(id).bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).bind(write.name.as_str()).bind(write.status.as_str()).execute(&mut *tx).await.map_err(store_error)?;
            id
        };
        let next = revision + 1;
        sqlx::query("INSERT INTO database_connection_revisions(tenant_id,namespace_id,connection_id,revision,config,status,actor,request_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).bind(id).bind(i64::from(next)).bind(config).bind(write.status.as_str()).bind(principal.subject().as_str()).bind(write.request_id).execute(&mut *tx).await.map_err(store_error)?;
        if revision != 0 {
            sqlx::query("UPDATE database_connections SET current_revision=$4,status=$5,updated_at=clock_timestamp() WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3")
                .bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).bind(id).bind(i64::from(next)).bind(write.status.as_str()).execute(&mut *tx).await.map_err(store_error)?;
        }
        insert_audit(&mut tx, audit).await?;
        let row = sqlx::query(select_connections!(" WHERE c.tenant_id=$1 AND c.namespace_id=$2 AND c.id=$3 AND r.revision=c.current_revision"))
            .bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).bind(id).fetch_one(&mut *tx).await.map_err(store_error)?;
        let result = from_row(&row)?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }
    async fn connection(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &Slug,
        revision: Option<u32>,
    ) -> Result<ConnectionRecord, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        current_permission(&mut tx, principal, Permission::NamespaceRead).await?;
        let result = load(&mut tx, principal, namespace, name, revision).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }
    async fn connections(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
    ) -> Result<Vec<ConnectionRecord>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        current_permission(&mut tx, principal, Permission::NamespaceRead).await?;
        let rows = sqlx::query(select_connections!(" WHERE c.tenant_id=$1 AND c.namespace_id=$2 AND c.status<>'DELETED' AND r.revision=c.current_revision ORDER BY c.name LIMIT 1000"))
            .bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).fetch_all(&mut *tx).await.map_err(store_error)?;
        let result = rows.iter().map(from_row).collect::<Result<Vec<_>, _>>()?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }
}
