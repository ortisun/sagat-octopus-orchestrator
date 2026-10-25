//! Lease-bound managed database capabilities. Provider I/O separates two authority transactions.
use super::*;

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    /// Resolves one managed capability in two transactions. Provider I/O holds no run, catalog or
    /// membership lock. Every current fence is repeated and the resolution audit commits first.
    #[tracing::instrument(name = "database.resolve", skip_all)]
    pub async fn resolve_database(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        trace_id: Option<&str>,
    ) -> Result<crate::database::ResolvedDatabase, AppError> {
        principal.authorize(Permission::DatabaseExecute)?;
        let started = std::time::Instant::now();
        let result = self.resolve_database_inner(principal, task, trace_id).await;
        let outcome = if result.is_ok() {
            "resolved"
        } else {
            "refused"
        };
        metrics::counter!("database_capabilities_total", "outcome" => outcome).increment(1);
        metrics::histogram!("database_capability_seconds", "outcome" => outcome)
            .record(started.elapsed().as_secs_f64());
        if result.is_err() {
            let audit = AuditRecord::by(
                principal,
                AuditAction::DatabaseDenied,
                ResourceType::Run,
                task.run_id.to_string(),
                trace_id.map(str::to_owned),
                self.clock.now(),
            )
            .with_reason("managed database capability refused");
            RunStore::record_denial(&self.store, &audit).await?;
        }
        result
    }

    async fn resolve_database_inner(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        trace_id: Option<&str>,
    ) -> Result<crate::database::ResolvedDatabase, AppError> {
        if !task.instance.path().is_root() {
            return Err(AppError::StaleAttempt);
        }
        let reader = self.vault.as_ref().ok_or(AppError::Unavailable)?;
        let before = {
            let mut tx = self.open(principal, task, Lock::Wait).await?;
            self.live_capability_fence(principal, &mut tx, task).await?;
            let snapshot = tx
                .database_input(task.instance.node(), principal.subject().as_str())
                .await?;
            let snapshot = self.database_parameters(&mut tx, snapshot).await?;
            tx.commit().await?;
            snapshot
        };
        let value = reader.resolve(&before.secret).await?;
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        self.live_capability_fence(principal, &mut tx, task).await?;
        let after = tx
            .database_input(task.instance.node(), principal.subject().as_str())
            .await?;
        let after = self.database_parameters(&mut tx, after).await?;
        if before != after {
            return Err(AppError::Unavailable);
        }
        let expires_at = tx
            .lease_expiry_instance(task.instance)
            .await?
            .ok_or(AppError::StaleAttempt)?;
        tx.pin_database_credential(task.instance.node(), &after.secret)
            .await?;
        let mut effects = Effects::by(
            Actor::Worker(principal.subject().as_str().to_owned()),
            trace_id.map(str::to_owned),
        );
        effects.audit = Some(
            AuditRecord::by(
                principal,
                AuditAction::SecretResolved,
                ResourceType::Secret,
                format!("{}/run {}", tx.info().namespace_id, task.run_id),
                trace_id.map(str::to_owned),
                self.clock.now(),
            )
            .with_reason(format!(
                "database node {} attempt {} generation {}: {} v{}",
                task.instance.node(),
                task.fence.attempt,
                task.fence.generation,
                after.secret.meta.name,
                after.secret.meta.version
            )),
        );
        tx.persist(&[], effects).await?;
        tx.commit().await?;
        Ok(crate::database::ResolvedDatabase {
            connection: after.connection,
            operation: after.operation,
            expires_at,
            credential: ResolvedSecret {
                name: after.secret.meta.name,
                version: after.secret.meta.version,
                value,
            },
        })
    }

    async fn database_parameters<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        mut input: crate::database::DatabaseInput,
    ) -> Result<crate::database::DatabaseInput, AppError> {
        use sagat_domain::database::{DatabaseOperation, Parameter};
        let data = tx.data().await?;
        let parameters = input
            .operation
            .parameters()
            .iter()
            .map(|p| {
                self.engine
                    .evaluator()
                    .database_parameter(p, data)
                    .map(Parameter::Literal)
                    .map_err(|_| AppError::NotAllowed("invalid database parameter"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        input.operation = DatabaseOperation::new(
            input.operation.connection.clone(),
            input.operation.kind,
            input.operation.statement(),
            parameters,
            input.operation.limits,
        )
        .map_err(|_| AppError::NotAllowed("invalid database parameters or limits"))?;
        Ok(input)
    }

    #[tracing::instrument(name = "database.output.scan", skip_all)]
    pub(super) async fn scan_database_output(
        &self,
        secret: &crate::secrets::StoredSecret,
        limits: sagat_domain::database::DatabaseLimits,
        output: &[u8],
    ) -> Result<bool, AppError> {
        let started = std::time::Instant::now();
        let result = async {
            let validator = self.database_results.as_ref().ok_or(AppError::NotAllowed(
                "managed database results require content scanning",
            ))?;
            validator.validate(output, limits).map_err(|_| {
                AppError::NotAllowed("invalid or oversized managed database result")
            })?;
            let reader = self.vault.as_ref().ok_or(AppError::Unavailable)?;
            let codec = self
                .database_credentials
                .as_ref()
                .ok_or(AppError::Unavailable)?;
            let value = reader.resolve(secret).await?;
            codec
                .contains_credential(&value, output)
                .map_err(|_| AppError::NotAllowed("invalid managed database content"))
        }
        .await;
        let outcome = match &result {
            Ok(true) => "detected",
            Ok(false) => "clean",
            Err(_) => "refused",
        };
        metrics::counter!("database_output_scans_total","outcome"=>outcome).increment(1);
        metrics::histogram!("database_output_scan_seconds","outcome"=>outcome)
            .record(started.elapsed().as_secs_f64());
        result
    }

    pub(super) async fn database_output_snapshot<Tx: RunTx>(
        &self,
        principal: &Principal,
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Result<
        (
            crate::secrets::StoredSecret,
            sagat_domain::database::DatabaseLimits,
        ),
        AppError,
    > {
        self.live_capability_fence(principal, tx, task).await?;
        let secret = tx
            .database_credential(task.instance.node())
            .await?
            .ok_or(AppError::Unavailable)?;
        let limits = tx.database_result_limits(task.instance.node()).await?;
        Ok((secret, limits))
    }

    /// Returns the frozen bound only for a live managed attempt with a retained credential pin.
    /// Ordinary uploads continue through their existing streaming admission. `None` does not
    /// authorize an upload: the caller must still obtain its exact-path blob capability.
    pub async fn database_blob_limit(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
    ) -> Result<Option<usize>, AppError> {
        principal.authorize(Permission::WorkerExecute)?;
        // Managed database execution is root-only. Do not restore unrelated scoped history merely
        // to discover that this is ordinary work; check_blob_upload enforces its live capability.
        if !task.instance.path().is_root() {
            return Ok(None);
        }
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        self.verify(&mut tx, task).await?;
        self.ensure_bound(principal, &mut tx, task).await?;
        if !Self::managed_database(&mut tx, task) {
            return Ok(None);
        }
        let (_, limits) = self
            .database_output_snapshot(principal, &mut tx, task)
            .await?;
        tx.commit().await?;
        Ok(Some(limits.max_result_bytes as usize))
    }

    /// Scans managed content before any object reservation/write. Rechecks the capability after
    /// provider I/O and durably rejects credential-bearing content without retaining the bytes.
    #[tracing::instrument(name = "database.blob.scan", skip_all)]
    pub async fn check_database_blob_upload(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        content: &[u8],
    ) -> Result<BlobScope, AppError> {
        principal.authorize(Permission::DatabaseExecute)?;
        let before = {
            let mut tx = self.open(principal, task, Lock::Wait).await?;
            let snapshot = self
                .database_output_snapshot(principal, &mut tx, task)
                .await?;
            tx.commit().await?;
            snapshot
        };
        let contains = self
            .scan_database_output(&before.0, before.1, content)
            .await?;
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        let after = self
            .database_output_snapshot(principal, &mut tx, task)
            .await?;
        if before != after {
            return Err(AppError::StaleAttempt);
        }
        if contains {
            let idempotency =
                completion_idempotency(task, "database-blob-output-refused", content, false);
            self.reject_secret_output(principal, task, tx, before.0.meta.name, idempotency)
                .await?;
            return Err(AppError::NotAllowed(
                "credential in managed database result",
            ));
        }
        let scope = Self::scope_of(&tx, task);
        tx.commit().await?;
        Ok(scope)
    }
}
