use crate::DestinationScope;
use crate::{ExecutionError, NetworkPolicy, transport, validate_statement, values};
use futures_util::{StreamExt, future::poll_fn};
use sagat_application::database::DatabaseCredentials;
use sagat_domain::database::{ConnectionConfig, DatabaseOperation, OperationKind};
use serde_json::json;
use std::{
    fmt,
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::Semaphore,
    time::{Duration, Instant, timeout, timeout_at},
};
use tokio_postgres::{Client, NoTls, types::ToSql};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

/// Serialized bounded retained content. Debug never prints cells or column names.
pub struct DatabaseResult(Zeroizing<Vec<u8>>);
impl DatabaseResult {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl fmt::Debug for DatabaseResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DatabaseResult(<redacted>)")
    }
}
/// Fresh connections per attempt, with a finite process-wide capacity. Distributed per-connection
/// capacity is acquired with the managed lease. No idle credential-bearing sessions are retained.
/// The embedding process MUST use Sagat telemetry, which suppresses the driver's SQL log target.
pub struct PostgresExecutor {
    network: NetworkPolicy,
    capacity: Arc<Semaphore>,
}
impl PostgresExecutor {
    pub fn new(network: NetworkPolicy, capacity: usize) -> Result<Self, ExecutionError> {
        if capacity == 0 || capacity > 256 {
            return Err(ExecutionError::InvalidConfiguration);
        }
        Ok(Self {
            network,
            capacity: Arc::new(Semaphore::new(capacity)),
        })
    }
    #[tracing::instrument(name = "database.execute", skip_all)]
    pub async fn execute(
        &self,
        config: &ConnectionConfig,
        namespace_networks: &DestinationScope,
        credentials: DatabaseCredentials,
        mut operation: DatabaseOperation,
        lease_deadline: Instant,
        cancelled: &CancellationToken,
    ) -> Result<DatabaseResult, ExecutionError> {
        let started = Instant::now();
        config
            .validate()
            .map_err(|_| ExecutionError::InvalidConfiguration)?;
        operation.limits = operation.limits.intersect(config.limits());
        operation
            .validate()
            .map_err(|_| ExecutionError::InvalidConfiguration)?;
        validate_statement(operation.statement(), operation.kind)?;
        values::bindings(operation.parameters())?;
        if cancelled.is_cancelled() {
            return Err(ExecutionError::Cancelled);
        }
        if lease_deadline <= started {
            return Err(ExecutionError::Timeout);
        }
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ExecutionError::Capacity)?;
        let deadline = lease_deadline
            .min(started + Duration::from_millis(u64::from(operation.limits.timeout_ms)));
        let connect_deadline = deadline
            .min(started + Duration::from_millis(u64::from(operation.limits.connect_timeout_ms)));
        let ConnectionConfig::Postgres {
            host,
            port,
            database,
            tls,
            ..
        } = config;
        let exceeded = Arc::new(AtomicBool::new(false));
        let connect = async {
            let address = self
                .network
                .resolve(host, *port, namespace_networks)
                .await?;
            let socket = transport::socket(address, host, tls, self.network.development()).await?;
            let socket =
                transport::bounded(socket, operation.limits.max_result_bytes, exceeded.clone());
            let mut pg = tokio_postgres::Config::new();
            pg.user(std::str::from_utf8(credentials.username.expose()).map_err(|_|ExecutionError::Authentication)?)
                .password(credentials.password.expose()).dbname(database).application_name("sagat-database-runner")
                .ssl_mode(tokio_postgres::config::SslMode::Disable)
                .options(format!("-c statement_timeout={} -c lock_timeout={} -c idle_in_transaction_session_timeout={} -c DateStyle=ISO -c TimeZone=UTC",operation.limits.timeout_ms,operation.limits.timeout_ms,operation.limits.timeout_ms));
            // No config strings, environment, pgpass, implicit user, or second DNS resolution.
            let (client, connection) = pg.connect_raw(socket, NoTls).await.map_err(|e| {
                if e.code().is_some_and(|c| c.code().starts_with("28")) {
                    ExecutionError::Authentication
                } else {
                    ExecutionError::Connection
                }
            })?;
            Ok::<_, ExecutionError>((address, client, connection))
        };
        let connected = tokio::select! {
            biased;
            () = cancelled.cancelled() => Err(ExecutionError::Cancelled),
            result = timeout_at(connect_deadline,connect) => result.unwrap_or(Err(ExecutionError::Timeout)),
        };
        drop(credentials);
        let (address, mut client, mut connection) = connected?;
        // The driver's Connection Future logs untrusted server notices. Poll messages explicitly
        // and discard them without formatting; channel and total-wire budgets remain bounded.
        let mut pump = AbortOnDrop(tokio::spawn(async move {
            let _permit = permit;
            while let Some(message) = poll_fn(|cx| connection.poll_message(cx)).await {
                if message.is_err() {
                    break;
                }
            }
        }));
        let cancel_key = client.cancel_token();
        let dispatched = AtomicBool::new(false);
        let work = execute_transaction(&mut client, &operation, &dispatched);
        let result = tokio::select! {
            biased;
            () = cancelled.cancelled() => Err(ExecutionError::Cancelled),
            result = timeout_at(deadline,work) => result.unwrap_or(Err(ExecutionError::Timeout)),
        };
        if matches!(
            result,
            Err(ExecutionError::Cancelled | ExecutionError::Timeout | ExecutionError::ResultLimit)
        ) {
            // A new TCP/TLS socket to the SAME validated address, never another DNS lookup.
            let _ = timeout(Duration::from_secs(1), async {
                let socket =
                    transport::socket(address, host, tls, self.network.development()).await?;
                cancel_key
                    .cancel_query_raw(socket, NoTls)
                    .await
                    .map_err(|_| ExecutionError::Connection)
            })
            .await;
        }
        drop(client);
        pump.close().await; // capacity is released only after the socket is dropped
        let mut result = if exceeded.load(Ordering::Relaxed) {
            Err(ExecutionError::ResultLimit)
        } else {
            result
        };
        if result.is_err()
            && operation.kind == OperationKind::Execute
            && dispatched.load(Ordering::Relaxed)
        {
            result = Err(ExecutionError::OutcomeUnknown);
        }
        let outcome = if result.is_ok() { "success" } else { "failed" };
        metrics::counter!("database_executions_total","operation"=>operation.kind.as_str(),"outcome"=>outcome).increment(1);
        metrics::histogram!("database_execution_seconds","operation"=>operation.kind.as_str())
            .record(started.elapsed().as_secs_f64());
        result
    }
}
struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl AbortOnDrop {
    async fn close(&mut self) {
        self.0.abort();
        let _ = (&mut self.0).await;
    }
}
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn pg_error(error: tokio_postgres::Error) -> ExecutionError {
    if error.code().is_some_and(|c| c.code() == "57014") {
        ExecutionError::Timeout
    } else {
        ExecutionError::Statement
    }
}
async fn execute_transaction(
    client: &mut Client,
    operation: &DatabaseOperation,
    dispatched: &AtomicBool,
) -> Result<DatabaseResult, ExecutionError> {
    // Privileged login roles and inherited server-file/program rights are never accepted. The
    // customer's grants on SECURITY DEFINER functions/extensions remain an operator responsibility.
    let role = client.query_one("SELECT NOT (rolsuper OR rolcreaterole OR rolcreatedb OR rolreplication OR rolbypassrls) AND current_user = session_user AND NOT pg_has_role(current_user, 'pg_read_server_files', 'MEMBER') AND NOT pg_has_role(current_user, 'pg_write_server_files', 'MEMBER') AND NOT pg_has_role(current_user, 'pg_execute_server_program', 'MEMBER') FROM pg_catalog.pg_roles WHERE rolname = current_user",&[]).await.map_err(pg_error)?;
    if !role
        .try_get::<_, bool>(0)
        .map_err(|_| ExecutionError::PrivilegedRole)?
    {
        return Err(ExecutionError::PrivilegedRole);
    }
    let tx = client
        .build_transaction()
        .read_only(operation.kind == OperationKind::Query)
        .start()
        .await
        .map_err(pg_error)?;
    let bindings = values::bindings(operation.parameters())?;
    let types: Vec<_> = bindings.iter().map(values::Bind::pg_type).collect();
    let stmt = tx
        .prepare_typed(operation.statement(), &types)
        .await
        .map_err(pg_error)?;
    if stmt.params().len() != bindings.len() {
        return Err(ExecutionError::ValueType);
    }
    let mut columns = Vec::with_capacity(stmt.columns().len());
    for column in stmt.columns() {
        columns.push(json!({"name":column.name(),"type":values::type_name(column.type_())?}));
    }
    let mut result = LimitedOutput {
        bytes: Zeroizing::new(Vec::new()),
        max: operation.limits.max_result_bytes as usize,
    };
    result.put(b"{\"columns\":")?;
    result.json(&columns)?;
    result.put(b",\"rows\":[")?;
    dispatched.store(true, Ordering::Relaxed);
    let rows = tx
        .query_raw(&stmt, bindings.iter().map(|b| b as &(dyn ToSql + Sync)))
        .await
        .map_err(pg_error)?;
    tokio::pin!(rows);
    let mut count = 0_u32;
    while let Some(row) = rows.next().await {
        let row = row.map_err(pg_error)?;
        if count >= operation.limits.max_rows {
            return Err(ExecutionError::ResultLimit);
        }
        if count > 0 {
            result.put(b",")?;
        }
        result.put(b"[")?;
        for i in 0..row.len() {
            if i > 0 {
                result.put(b",")?;
            }
            result.json(&values::cell(&row, i)?)?;
        }
        result.put(b"]")?;
        count += 1;
    }
    let affected = rows.rows_affected().ok_or(ExecutionError::Statement)?;
    result.put(b"],\"affectedRows\":")?;
    result.json(&affected.to_string())?;
    result.put(b",\"truncated\":false}")?;
    // Retention-size/type errors are detected before asking PostgreSQL to commit a write.
    tx.commit().await.map_err(pg_error)?;
    Ok(DatabaseResult(result.bytes))
}
struct LimitedOutput {
    bytes: Zeroizing<Vec<u8>>,
    max: usize,
}
impl LimitedOutput {
    fn put(&mut self, bytes: &[u8]) -> Result<(), ExecutionError> {
        self.write_all(bytes)
            .map_err(|_| ExecutionError::ResultLimit)
    }
    fn json(&mut self, value: &impl serde::Serialize) -> Result<(), ExecutionError> {
        serde_json::to_writer(self, value).map_err(|_| ExecutionError::ResultLimit)
    }
}
impl Write for LimitedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("database result limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
