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
        let reader = self.vault.as_ref().ok_or(AppError::Unavailable)?;
        let before = {
            let mut tx = self.open(principal, task, Lock::Wait).await?;
            self.database_fence(principal, &mut tx, task).await?;
            let snapshot = tx
                .database_input(task.node, principal.subject().as_str())
                .await?;
            let snapshot = self.database_parameters(&mut tx, snapshot).await?;
            tx.commit().await?;
            snapshot
        };
        let value = reader.resolve(&before.secret).await?;
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        self.database_fence(principal, &mut tx, task).await?;
        let after = tx
            .database_input(task.node, principal.subject().as_str())
            .await?;
        let after = self.database_parameters(&mut tx, after).await?;
        if before != after {
            return Err(AppError::Unavailable);
        }
        let expires_at = tx
            .lease_expiry(task.node)
            .await?
            .ok_or(AppError::StaleAttempt)?;
        tx.pin_database_credential(task.node, &after.secret).await?;
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
                task.node,
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

    async fn database_fence<Tx: RunTx>(
        &self,
        principal: &Principal,
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Result<(), AppError> {
        self.verify(tx, task).await?;
        self.ensure_bound(principal, tx, task).await?;
        let valid_step = tx.parts().1.step(task.node).is_some_and(|s| {
            s.attempt() == task.fence.attempt
                && s.generation() == task.fence.generation
                && !s.cancel_requested()
        }) && tx.parts().1.stop_reason().is_none()
            && !tx.parts().1.is_finished();
        if !valid_step
            || !tx
                .lease_unexpired(task.node, &Token::hash_of(task.lease_token))
                .await?
        {
            return Err(AppError::StaleAttempt);
        }
        Ok(())
    }
}
