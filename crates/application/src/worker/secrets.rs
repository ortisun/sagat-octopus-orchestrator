//! Ordinary secret capabilities and report scanning release run locks around provider I/O.
use super::*;
use crate::secrets::StoredSecret;

pub(super) enum SecretReport<Tx> {
    Ready(Tx, Option<SecretName>),
    Replayed(StepAck),
}

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    /// Hands a worker the values of the secrets its step declares, for the length of the attempt it
    /// holds the lease of (FR-009-002, US02).
    ///
    /// The lease is the temporary capability: it must be current, the identity still bound to the
    /// activity, and the *publisher* of the workflow version must still be allowed to use every
    /// secret, now and not only when they published (an access list that changed is honoured by the
    /// next resolution). All or nothing: one unusable secret and the worker gets none. The
    /// resolution is audited after the final authority recheck, with names and never
    /// values, so a value cannot be handed out without a record.
    ///
    /// # Errors
    /// [`AppError::StaleAttempt`], [`AppError::NotBound`], [`AppError::SecretUnavailable`] (a secret
    /// is gone or its publisher may no longer use it; audited), [`AppError::NotAllowed`] when this
    /// instance has no key ring.
    #[tracing::instrument(name = "secret.resolve", skip_all)]
    pub async fn resolve_secrets(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        trace_id: Option<&str>,
    ) -> Result<Vec<ResolvedSecret>, AppError> {
        principal.authorize(Permission::WorkerExecute)?;
        let Some(cipher) = &self.vault else {
            return Err(AppError::NotAllowed(
                "secrets are not configured on this instance",
            ));
        };
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        self.live_capability_fence(principal, &mut tx, task).await?;
        let names = Self::declared_secrets(&mut tx, task);
        if names.is_empty() {
            return Ok(Vec::new());
        }
        // The pools are judged again here, on the grants of this moment: a lease handed out before a
        // secret was restricted, or before the worker's grant was taken away, does not carry the
        // value (G48).
        if let Some(withheld) = tx
            .secret_pool_refusal(&names, principal.subject().as_str(), PoolCheck::Delivery)
            .await?
        {
            return self
                .refuse_for_pools(principal, task, tx, withheld, trace_id)
                .await;
        }
        let info = tx.info().clone();
        let inputs = tx.secret_inputs(&names).await?;
        let states: Vec<_> = inputs
            .items
            .iter()
            .map(|item| {
                let state = if item.stored.is_some() {
                    SecretState::Present {
                        granted: item.granted,
                    }
                } else {
                    SecretState::Missing
                };
                (item.name.clone(), state)
            })
            .collect();
        let audit = |action, reason: String| {
            let mut effects = Effects::by(
                Actor::Worker(principal.subject().as_str().to_owned()),
                trace_id.map(str::to_owned),
            );
            effects.audit = Some(
                AuditRecord::by(
                    principal,
                    action,
                    ResourceType::Secret,
                    format!("{}/run {}", info.namespace_id, info.run_id),
                    trace_id.map(str::to_owned),
                    self.clock.now(),
                )
                .with_reason(reason),
            );
            effects
        };
        if let Err(SecretProblem(name)) = check_usable(inputs.publisher_role, &states) {
            // The denial is written and committed: it is the trail of why a worker got nothing.
            let effects = audit(
                AuditAction::SecretDenied,
                format!(
                    "node {} attempt {}: {name} not usable by its publisher",
                    task.instance.node(),
                    task.fence.attempt
                ),
            );
            tx.persist(&[], effects).await?;
            tx.commit().await?;
            metrics::counter!("secret_resolution_denied_total").increment(1);
            return Err(AppError::SecretUnavailable {
                name: name.to_string(),
            });
        }
        tx.commit().await?;
        let mut resolved = Vec::with_capacity(names.len());
        for name in &names {
            let stored = inputs
                .items
                .iter()
                .find(|item| &item.name == name)
                .and_then(|item| item.stored.as_ref())
                .ok_or(AppError::Internal)?;
            let value = cipher.resolve(stored).await?;
            resolved.push(ResolvedSecret {
                name: name.clone(),
                version: stored.meta.version,
                value,
            });
        }
        // Discard resolved values if lease, cancellation, binding, publisher ACL or rotation changed.
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        self.live_capability_fence(principal, &mut tx, task).await?;
        if let Some(withheld) = tx
            .secret_pool_refusal(&names, principal.subject().as_str(), PoolCheck::Delivery)
            .await?
        {
            return self
                .refuse_for_pools(principal, task, tx, withheld, trace_id)
                .await;
        }
        let current = tx.secret_inputs(&names).await?;
        let states: Vec<_> = current
            .items
            .iter()
            .map(|item| {
                (
                    item.name.clone(),
                    if item.stored.is_some() {
                        SecretState::Present {
                            granted: item.granted,
                        }
                    } else {
                        SecretState::Missing
                    },
                )
            })
            .collect();
        if let Err(SecretProblem(name)) = check_usable(current.publisher_role, &states) {
            tx.persist(
                &[],
                audit(
                    AuditAction::SecretDenied,
                    format!(
                        "node {} attempt {}: access changed during resolution",
                        task.instance.node(),
                        task.fence.attempt
                    ),
                ),
            )
            .await?;
            tx.commit().await?;
            return Err(AppError::SecretUnavailable {
                name: name.to_string(),
            });
        }
        if current != inputs {
            return Err(AppError::Unavailable);
        }
        let listed = resolved
            .iter()
            .map(|r| format!("{} v{}", r.name, r.version))
            .collect::<Vec<_>>()
            .join(", ");
        let effects = audit(
            AuditAction::SecretResolved,
            format!(
                "node {} attempt {}: {listed}",
                task.instance.node(),
                task.fence.attempt
            ),
        );
        tx.persist(&[], effects).await?;
        tx.commit().await?;
        metrics::counter!("secrets_handed_out_total").increment(resolved.len() as u64);
        Ok(resolved)
    }

    /// Audits and commits the refusal of a secret outside the worker's pools, then answers as if the
    /// secret did not exist for this worker.
    async fn refuse_for_pools(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        mut tx: S::Tx,
        withheld: SecretName,
        trace_id: Option<&str>,
    ) -> Result<Vec<ResolvedSecret>, AppError> {
        let effects = self.pool_refusal_effects(
            principal,
            &tx,
            task.instance.node(),
            task.fence.attempt,
            &withheld,
            "resolve",
            trace_id,
        );
        tx.persist(&[], effects).await?;
        tx.commit().await?;
        metrics::counter!("secret_delivery_refused_total", "stage" => "resolve").increment(1);
        Err(AppError::SecretUnavailable {
            name: withheld.to_string(),
        })
    }

    /// The name of a secret the step declares whose value appears in `output`, if one does. Only
    /// steps that declare secrets pay for the check, and only values of at least eight bytes are
    /// looked for (a shorter one would match ordinary text); a value that is not UTF-8 is not
    /// looked for either. Both the value and its JSON-escaped form are searched, for every version
    /// of the secret that is still stored: a value handed out earlier in the attempt may have been
    /// replaced since.
    async fn secret_in_output(
        &self,
        versions: &[StoredSecret],
        output: &[u8],
    ) -> Result<Option<SecretName>, AppError> {
        let Some(cipher) = &self.vault else {
            return Ok(None);
        };
        let Ok(text) = std::str::from_utf8(output) else {
            return Ok(None);
        };
        for stored in versions {
            let value = match cipher.resolve(stored).await {
                Ok(value) => value,
                Err(error) if matches!(stored.value, crate::secrets::StoredValue::OpenBao(_)) => {
                    return Err(error);
                }
                Err(_) => continue,
            };
            if value.len() < MIN_SCANNED_SECRET_BYTES {
                continue;
            }
            let Ok(plain) = std::str::from_utf8(value.expose()) else {
                continue;
            };
            if text.contains(plain)
                || text.contains(&json_escaped(plain, false))
                || text.contains(&json_escaped(plain, true))
            {
                return Ok(Some(stored.meta.name.clone()));
            }
        }
        Ok(None)
    }

    /// Snapshot -> provider scan -> current fence/snapshot, with receipt replay before any new effect.
    #[tracing::instrument(name = "secret.output.scan", skip_all)]
    pub(super) async fn checked_secret_report(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        mut tx: S::Tx,
        idempotency: &Idempotency,
        output: &[u8],
    ) -> Result<SecretReport<S::Tx>, AppError> {
        let names = Self::declared_secrets(&mut tx, task);
        if self.vault.is_none() || names.is_empty() {
            return Ok(SecretReport::Ready(tx, None));
        }
        self.live_capability_fence(principal, &mut tx, task).await?;
        let before = tx.secret_versions(&names).await?;
        tx.commit().await?;
        let scanned = self.secret_in_output(&before, output).await;
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        if let Some(ack) = self.replayed(principal, &mut tx, task, idempotency).await? {
            return Ok(SecretReport::Replayed(ack));
        }
        self.live_capability_fence(principal, &mut tx, task).await?;
        if before != tx.secret_versions(&names).await? {
            return Err(AppError::Unavailable);
        }
        Ok(SecretReport::Ready(tx, scanned?))
    }
}
