//! Exact-path PostgreSQL mutation transaction.

use super::*;

impl PathExecutionTx for PgPathExecutionTx {
    async fn logical_output_bytes(&mut self) -> Result<u64, StoreError> {
        Ok(crate::collection_outputs::inspect(
            &mut self.tx,
            &self.snapshot.info,
            &self.snapshot.definition,
        )
        .await?
        .total())
    }

    fn snapshot(&mut self) -> &mut PathExecutionSnapshot {
        &mut self.snapshot
    }

    fn now(&self) -> OffsetDateTime {
        self.now
    }

    async fn lease_deadlines(
        &mut self,
        instance: &ExecutionInstanceId,
    ) -> Result<Option<LeaseDeadlines>, StoreError> {
        if instance.path() != self.snapshot.state.path() {
            return Ok(None);
        }
        let (nodes, indices) = stored_path_parts(instance.path());
        let row = sqlx::query(
            "SELECT lease_expires_at,attempt_deadline_at FROM steps \
             WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
               AND instance_path_nodes=$4 AND instance_path_indices=$5 \
               AND status IN ('LEASED','RUNNING')",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .bind(instance.node().as_str())
        .bind(nodes)
        .bind(indices)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?;
        row.map(|row| {
            Ok(LeaseDeadlines {
                expires_at: col(&row, "lease_expires_at")?,
                attempt_deadline_at: col(&row, "attempt_deadline_at")?,
            })
        })
        .transpose()
    }

    async fn holds_shard(
        &mut self,
        owner: &str,
        lease: sagat_application::shards::Lease,
    ) -> Result<bool, StoreError> {
        let Some(now) =
            locked_scheduler_time(&mut self.tx, self.snapshot.info.run_id, owner, lease).await?
        else {
            return Ok(false);
        };
        self.now = now;
        Ok(true)
    }

    async fn lease_expiry(
        &mut self,
        instance: &ExecutionInstanceId,
    ) -> Result<Option<OffsetDateTime>, StoreError> {
        self.lease_deadlines(instance)
            .await
            .map(|deadlines| deadlines.map(|lease| lease.expires_at))
    }

    async fn worker_drained(&mut self, subject: &str) -> Result<bool, StoreError> {
        sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
            .bind(crate::operations::drain_lock_key(
                self.snapshot.info.tenant_id,
                self.snapshot.info.namespace_id,
                subject,
            ))
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM worker_drains \
             WHERE tenant_id=$1 AND namespace_id=$2 AND subject=$3)",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.namespace_id.as_uuid())
        .bind(subject)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)
    }

    async fn worker_session_room(
        &mut self,
        session_id: Uuid,
        token_hash: &[u8],
        subject: &str,
    ) -> Result<u32, StoreError> {
        worker_session_room(
            &mut self.tx,
            self.snapshot.info.tenant_id,
            self.snapshot.info.namespace_id,
            session_id,
            token_hash,
            subject,
        )
        .await
    }

    async fn bound_patterns(&mut self, subject: &str) -> Result<Vec<String>, StoreError> {
        crate::bindings::hold_worker_binding(
            &mut self.tx,
            self.snapshot.info.tenant_id,
            self.snapshot.info.namespace_id,
            subject,
        )
        .await?;
        sqlx::query_scalar(
            "SELECT pattern FROM worker_bindings \
             WHERE tenant_id=$1 AND namespace_id=$2 AND subject=$3 ORDER BY pattern",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.namespace_id.as_uuid())
        .bind(subject)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(store_error)
    }

    async fn secret_inputs(
        &mut self,
        names: &[SecretName],
    ) -> Result<sagat_application::secrets::SecretInputs, StoreError> {
        crate::secrets::inputs(
            &mut self.tx,
            self.snapshot.info.tenant_id,
            self.snapshot.info.namespace_id,
            self.snapshot.info.version_id.as_uuid(),
            names,
        )
        .await
    }

    async fn secret_pool_refusal(
        &mut self,
        names: &[SecretName],
        subject: &str,
        check: sagat_application::execution::PoolCheck,
    ) -> Result<Option<SecretName>, StoreError> {
        crate::secrets::pool_refusal(
            &mut self.tx,
            self.snapshot.info.tenant_id,
            self.snapshot.info.namespace_id,
            names,
            subject,
            check == sagat_application::execution::PoolCheck::Delivery,
        )
        .await
    }

    async fn secret_versions(
        &mut self,
        names: &[SecretName],
    ) -> Result<Vec<sagat_application::secrets::StoredSecret>, StoreError> {
        crate::secrets::all_versions(
            &mut self.tx,
            self.snapshot.info.tenant_id,
            self.snapshot.info.namespace_id,
            names,
        )
        .await
    }

    async fn current_worker_authorized(&mut self, subject: &str) -> Result<bool, StoreError> {
        current_worker_authorized(
            &mut self.tx,
            self.snapshot.info.tenant_id,
            self.snapshot.info.namespace_id,
            subject,
        )
        .await
    }

    async fn required_worker_protocol_minor(&mut self) -> Result<u32, StoreError> {
        let needs_blob_protocol: bool = sqlx::query_scalar(
            "SELECT runs.input_blob_id IS NOT NULL OR EXISTS ( \
                 SELECT 1 FROM blobs blob WHERE blob.tenant_id=runs.tenant_id \
                   AND blob.namespace_id=runs.namespace_id AND blob.run_id=runs.id \
                   AND blob.purpose='STEP_OUTPUT' AND blob.state='COMMITTED') \
             FROM runs WHERE tenant_id=$1 AND id=$2",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(if needs_blob_protocol {
            sagat_application::worker::BLOB_PROTOCOL_MINOR
        } else {
            0
        })
    }

    async fn activity_admitted(&mut self, activity: &ActivityName) -> Result<bool, StoreError> {
        crate::quotas::activity_admitted(
            &mut self.tx,
            self.snapshot.info.tenant_id,
            self.snapshot.info.namespace_id,
            activity,
        )
        .await
    }

    async fn compensation_admitted(&mut self, activity: &ActivityName) -> Result<bool, StoreError> {
        crate::breaker::admit(
            &mut self.tx,
            self.breaker,
            self.snapshot.info.tenant_id.as_uuid(),
            self.snapshot.info.namespace_id.as_uuid(),
            activity.as_str(),
        )
        .await
    }

    async fn script_execution_admitted(
        &mut self,
        instance: &ExecutionInstanceId,
        activity: &ActivityName,
    ) -> Result<bool, StoreError> {
        self.admit_path_script(instance, activity).await
    }

    async fn lease_unexpired(
        &mut self,
        instance: &ExecutionInstanceId,
        token_hash: &[u8],
    ) -> Result<bool, StoreError> {
        self.lease_token_matches(instance, token_hash, true).await
    }

    async fn lease_matches(
        &mut self,
        instance: &ExecutionInstanceId,
        token_hash: &[u8],
    ) -> Result<bool, StoreError> {
        self.lease_token_matches(instance, token_hash, false).await
    }

    async fn reported_by(
        &mut self,
        instance: &ExecutionInstanceId,
        token_hash: &[u8],
    ) -> Result<bool, StoreError> {
        let (nodes, indices) = stored_path_parts(instance.path());
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
               AND instance_path_nodes=$4 AND instance_path_indices=$5 \
               AND status NOT IN ('LEASED','RUNNING') AND reported_lease_token_hash=$6)",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .bind(instance.node().as_str())
        .bind(nodes)
        .bind(indices)
        .bind(token_hash)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)
    }

    async fn replay(&mut self, idempotency: &Idempotency) -> Result<Option<String>, StoreError> {
        replay(
            &mut self.tx,
            self.snapshot.info.tenant_id,
            idempotency,
            &hash(&idempotency.fingerprint),
        )
        .await
    }

    async fn persist_addressed(
        &mut self,
        events: &[AddressedRunEvent],
        outputs: &[(ExecutionInstanceId, Vec<u8>)],
        effects: Effects,
    ) -> Result<(), StoreError> {
        self.persist_path_addressed(events, outputs, effects).await
    }

    async fn commit(self) -> Result<(), StoreError> {
        if self.snapshot.state.global().version() != self.persisted {
            return Err(StoreError::Internal {
                detail: "bounded state was not persisted before commit".into(),
            });
        }
        self.tx.commit().await.map_err(store_error)?;
        for signal in self.signals {
            signal.emit();
        }
        Ok(())
    }
}

impl PgPathExecutionTx {
    async fn persist_path_addressed(
        &mut self,
        events: &[AddressedRunEvent],
        outputs: &[(ExecutionInstanceId, Vec<u8>)],
        effects: Effects,
    ) -> Result<(), StoreError> {
        let path = self.snapshot.state.path();
        if events.iter().any(|event| event.path() != path)
            || outputs.iter().any(|(instance, _)| instance.path() != path)
            || effects
                .lease
                .as_ref()
                .is_some_and(|grant| grant.instance.path() != path)
            || effects
                .renewal
                .as_ref()
                .is_some_and(|renewal| renewal.instance.path() != path)
            || effects
                .reported
                .as_ref()
                .is_some_and(|reported| reported.instance.path() != path)
            || effects
                .rehint_instances
                .iter()
                .any(|instance| instance.path() != path)
        {
            return Err(StoreError::Internal {
                detail: "a bounded write crossed its exact path".into(),
            });
        }
        if !effects.outputs.is_empty() || !effects.rehint.is_empty() || !effects.children.is_empty()
        {
            return Err(StoreError::Internal {
                detail: "a bounded write contains root-only effects".into(),
            });
        }
        if events.iter().any(|event| {
            !matches!(
                event.event(),
                RunEvent::StepLeased { .. }
                    | RunEvent::StepStarted { .. }
                    | RunEvent::StepCompleted { .. }
                    | RunEvent::StepFailed { .. }
                    | RunEvent::StepRouted { .. }
                    | RunEvent::StepRetryScheduled { .. }
                    | RunEvent::StepRetryDue { .. }
                    | RunEvent::StepReleased { .. }
                    | RunEvent::StepCancelRequested { .. }
                    | RunEvent::StepCancelled { .. }
                    | RunEvent::ReconciliationRequested { .. }
                    | RunEvent::ReconciliationConcluded { .. }
                    | RunEvent::StepNeedsReview { .. }
                    | RunEvent::StepRetryRequested { .. }
                    | RunEvent::StepSkipped { .. }
                    | RunEvent::StepResolved { .. }
            )
        }) {
            return Err(StoreError::Internal {
                detail: "the bounded event is not supported by path storage".into(),
            });
        }

        let written = u64::try_from(events.len()).map_err(|_| corrupt("event count"))?;
        let version = self.snapshot.state.global().version();
        if version != self.persisted + written {
            return Err(StoreError::Internal {
                detail: "the bounded state and events to write diverged".into(),
            });
        }
        let completed = events
            .iter()
            .filter_map(|event| {
                matches!(
                    event.event(),
                    RunEvent::StepCompleted { .. } | RunEvent::ReconciliationConcluded { .. }
                )
                .then(|| event.instance())
                .flatten()
            })
            .collect::<BTreeSet<_>>();
        let mut charged = BTreeSet::new();
        for (instance, _) in outputs {
            if !completed.contains(instance) || !charged.insert(instance.clone()) {
                return Err(StoreError::Internal {
                    detail: "a bounded output has no unique completion event".into(),
                });
            }
        }
        for blob in &effects.blobs {
            let info = &self.snapshot.info;
            let valid = effects.blobs.len() == 1
                && effects.reported.as_ref().is_some_and(|reported| {
                    let instance = &reported.instance;
                    completed.contains(instance)
                        && outputs.iter().any(|(output, bytes)| {
                            output == instance && blob_output_matches(bytes, blob)
                        })
                        && !instance.node().is_derived()
                        && blob.scope.access.tenant_id == info.tenant_id
                        && blob.scope.access.namespace_id == info.namespace_id
                        && blob.scope.run_id == info.run_id
                        && blob.scope.step_id
                            == instance_step_id(info.version_id, info.run_id, instance)
                        && self
                            .snapshot
                            .state
                            .local_instance(instance.node())
                            .is_some_and(|step| {
                                step.attempt() == blob.scope.attempt
                                    && step.generation() == blob.scope.generation
                            })
                });
            if !valid {
                return Err(StoreError::Internal {
                    detail: "a bounded blob commit has no matching forward completion".into(),
                });
            }
        }
        self.charge_path_output_bytes(outputs).await?;

        let tenant = self.snapshot.info.tenant_id;
        let run = self.snapshot.info.run_id;
        let mut output_digest = BTreeMap::new();
        for (instance, bytes) in outputs {
            let digest = Sha256::digest(bytes).to_vec();
            sqlx::query(
                "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(tenant.as_uuid())
            .bind(&digest)
            .bind(bytes)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
            output_digest.insert(instance.clone(), digest);
            self.snapshot
                .data
                .insert_output(instance.clone(), bytes.clone());
        }
        for blob in &effects.blobs {
            crate::blobs::commit(&mut self.tx, tenant, blob).await?;
        }

        let mut touched = BTreeSet::new();
        let mut hinted = BTreeSet::new();
        let mut rehinted = BTreeSet::new();
        for (offset, addressed) in events.iter().enumerate() {
            let instance = addressed
                .instance()
                .ok_or_else(|| corrupt("bounded event instance"))?;
            let event = addressed.event();
            let mut encoded = encode(event);
            if matches!(event, RunEvent::StepLeased { .. })
                && let Some(pins) = self.script_retention.get(&instance)
            {
                let detail =
                    encoded
                        .detail
                        .as_object_mut()
                        .ok_or_else(|| StoreError::Internal {
                            detail: "a bounded lease event did not encode as an object".into(),
                        })?;
                detail.insert("scriptRetentionPolicyVersion".into(), serde_json::json!(1));
                for (name, value) in [
                    ("scriptRetentionDays", pins.script),
                    ("compensationScriptRetentionDays", pins.compensation),
                    ("reconciliationScriptRetentionDays", pins.reconciliation),
                ] {
                    if let Some(days) = value {
                        detail.insert(name.into(), serde_json::json!(days));
                    }
                }
            }
            let seq = i64::try_from(self.persisted + 1 + offset as u64)
                .map_err(|_| corrupt("sequence"))?;
            let (path_nodes, path_indices) = stored_path_parts(instance.path());
            sqlx::query(
                "INSERT INTO run_events \
                 (tenant_id,run_id,seq,event_type,node,attempt,detail,payload_digest,actor, \
                  trace_id,occurred_at,instance_id,instance_path_nodes,instance_path_indices) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
            )
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .bind(seq)
            .bind(encoded.event_type)
            .bind(encoded.node.as_deref())
            .bind(encoded.attempt)
            .bind(&encoded.detail)
            .bind(
                matches!(
                    event,
                    RunEvent::StepCompleted { .. } | RunEvent::ReconciliationConcluded { .. }
                )
                .then(|| output_digest.get(&instance).map(Vec::as_slice))
                .flatten(),
            )
            .bind(effects.actor.label())
            .bind(
                effects
                    .trace_id
                    .as_deref()
                    .or(self.snapshot.info.trace_id.as_deref()),
            )
            .bind(self.now)
            .bind(instance_step_id(
                self.snapshot.info.version_id,
                run,
                &instance,
            ))
            .bind(path_nodes)
            .bind(path_indices)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
            touched.extend(addressed.changed_instances());
            if matches!(
                event,
                RunEvent::StepReleased { .. }
                    | RunEvent::StepRetryDue { .. }
                    | RunEvent::StepRetryRequested { .. }
            ) {
                hinted.insert(instance.clone());
            }
            self.record_path_breaker(&instance, event).await?;
            self.signals.extend(of_event(event));
        }
        for instance in effects
            .lease
            .iter()
            .map(|grant| &grant.instance)
            .chain(effects.renewal.iter().map(|renewal| &renewal.instance))
            .chain(effects.reported.iter().map(|reported| &reported.instance))
        {
            touched.insert(instance.clone());
        }
        for instance in &effects.rehint_instances {
            if self
                .snapshot
                .state
                .local_instance(instance.node())
                .is_some_and(|step| step.status() == StepStatus::Ready)
            {
                touched.insert(instance.clone());
                hinted.insert(instance.clone());
                rehinted.insert(instance.clone());
            }
        }

        if written > 0 {
            let head = i64::try_from(version).map_err(|_| corrupt("sequence"))?;
            sqlx::query(
                "INSERT INTO telemetry_pending (tenant_id,run_id,head_seq,applied_seq,behind_since) \
                 VALUES ($1,$2,$3,0,clock_timestamp()) \
                 ON CONFLICT (tenant_id,run_id) DO UPDATE SET \
                   behind_since=CASE WHEN telemetry_pending.applied_seq>=telemetry_pending.head_seq \
                     THEN clock_timestamp() ELSE telemetry_pending.behind_since END, \
                   head_seq=GREATEST(telemetry_pending.head_seq,EXCLUDED.head_seq)",
            )
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .bind(head)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
        }
        for instance in &touched {
            self.upsert_path_instance(
                instance,
                hinted.contains(instance),
                rehinted.contains(instance),
                output_digest.get(instance),
                &effects,
            )
            .await?;
        }
        for instance in &hinted {
            self.outbox_path_task_ready(instance).await?;
        }

        if written > 0 {
            let updated = sqlx::query(
                "UPDATE runs SET status=$4,run_version=$5,updated_at=$6 \
                 WHERE tenant_id=$1 AND id=$2 AND run_version=$3",
            )
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .bind(i64::try_from(self.persisted).map_err(|_| corrupt("run version"))?)
            .bind(self.snapshot.state.global().status().as_str())
            .bind(i64::try_from(version).map_err(|_| corrupt("run version"))?)
            .bind(self.now)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?
            .rows_affected();
            if updated != 1 {
                return Err(corrupt("bounded run revision"));
            }
        }
        self.persisted = version;
        if let Some(audit) = &effects.audit {
            insert_audit(&mut self.tx, audit).await?;
        }
        if let Some((idempotency, result)) = &effects.idempotency
            && !remember(
                &mut self.tx,
                tenant,
                idempotency,
                &hash(&idempotency.fingerprint),
                result,
            )
            .await?
        {
            return Err(StoreError::Unavailable {
                detail: "an idempotency record for this command appeared concurrently".into(),
            });
        }
        Ok(())
    }

    async fn charge_path_output_bytes(
        &mut self,
        outputs: &[(ExecutionInstanceId, Vec<u8>)],
    ) -> Result<(), StoreError> {
        let mut charge = 0_i64;
        for (instance, bytes) in outputs {
            let (path_nodes, path_indices) = stored_path_parts(instance.path());
            let retained: Option<bool> = sqlx::query_scalar(
                "SELECT output_digest IS NOT NULL FROM steps \
                 WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
                   AND instance_path_indices=$4 AND node=$5 FOR UPDATE",
            )
            .bind(self.snapshot.info.tenant_id.as_uuid())
            .bind(self.snapshot.info.run_id.as_uuid())
            .bind(path_nodes)
            .bind(path_indices)
            .bind(instance.node().as_str())
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(store_error)?;
            match retained {
                Some(true) => {}
                Some(false) => {
                    charge = charge
                        .checked_add(
                            i64::try_from(bytes.len()).map_err(|_| corrupt("output bytes"))?,
                        )
                        .ok_or_else(|| corrupt("output byte ledger"))?;
                }
                None => return Err(corrupt("output step projection")),
            }
        }
        if charge == 0 {
            return Ok(());
        }
        let effective = crate::collection_outputs::inspect(
            &mut self.tx,
            &self.snapshot.info,
            &self.snapshot.definition,
        )
        .await?;
        let charge = u64::try_from(charge).map_err(|_| corrupt("output byte charge"))?;
        let after = effective
            .total()
            .checked_add(charge)
            .ok_or_else(|| corrupt("effective output byte total"))?;
        if after > u64::try_from(MAX_RUN_OUTPUT_BYTES).map_err(|_| corrupt("run output limit"))? {
            return Err(StoreError::QuotaExceeded {
                what: "output retained by one run",
            });
        }
        let updated = sqlx::query(
            "UPDATE run_instance_usage SET logical_output_bytes=logical_output_bytes+$4 \
             WHERE tenant_id=$1 AND run_id=$2 AND logical_output_bytes=$3 \
               AND logical_output_bytes+$4+$5 <= $6",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .bind(i64::try_from(effective.ledger()).map_err(|_| corrupt("output byte ledger"))?)
        .bind(i64::try_from(charge).map_err(|_| corrupt("output byte charge"))?)
        .bind(i64::try_from(effective.legacy_bytes()?).map_err(|_| corrupt("legacy output bytes"))?)
        .bind(i64::try_from(MAX_RUN_OUTPUT_BYTES).map_err(|_| corrupt("run output limit"))?)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        if updated.rows_affected() != 1 {
            return Err(corrupt("run output byte ledger changed under its run lock"));
        }
        Ok(())
    }

    async fn ready_path_script_retention(
        &mut self,
        activity: &ActivityName,
    ) -> Result<Option<u16>, StoreError> {
        let Some(digest) = sagat_domain::scripts::digest_of_activity(activity.as_str()) else {
            return Ok(None);
        };
        let days: Option<i16> = sqlx::query_scalar(
            "SELECT max(retention_days) FROM script_versions \
             WHERE tenant_id=$1 AND artifact_digest=$2 AND status='READY'",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(digest.as_str())
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)?;
        let days = days.ok_or_else(|| StoreError::Internal {
            detail: "a workflow script has no READY version".into(),
        })?;
        u16::try_from(days)
            .ok()
            .filter(|days| (1..=365).contains(days))
            .ok_or_else(|| corrupt("READY script retention"))
            .map(Some)
    }

    async fn current_path_script_retention(
        &mut self,
        activity: &ActivityName,
    ) -> Result<ScriptRetentionDecision, StoreError> {
        if sagat_domain::scripts::digest_of_activity(activity.as_str()).is_none() {
            return Ok(ScriptRetentionDecision::NotScript);
        }
        let cap = match crate::content_usage::script_execution_retention(
            &mut self.tx,
            self.snapshot.info.tenant_id,
            self.snapshot.info.namespace_id,
            365,
        )
        .await
        {
            Ok(days) => days,
            Err(StoreError::QuotaExceeded { what })
                if what == sagat_domain::quota::QuotaResource::ScriptRetentionDays.limit_name() =>
            {
                return Ok(ScriptRetentionDecision::Refused);
            }
            Err(error) => return Err(error),
        };
        let requested = self
            .ready_path_script_retention(activity)
            .await?
            .ok_or_else(|| corrupt("script activity digest"))?;
        Ok(ScriptRetentionDecision::Pinned(requested.min(cap)))
    }

    async fn inherited_path_script_retention(
        &mut self,
        forward: &ExecutionInstanceId,
        field: &'static str,
        activity: &ActivityName,
    ) -> Result<u16, StoreError> {
        let attempt = self
            .snapshot
            .state
            .local_instance(forward.node())
            .map(sagat_domain::execution::StepState::attempt)
            .ok_or_else(|| corrupt("bounded derived script forward step"))?;
        let (path_nodes, path_indices) = stored_path_parts(forward.path());
        let detail: serde_json::Value = sqlx::query_scalar(
            "SELECT detail FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
               AND instance_path_nodes=$4 AND instance_path_indices=$5 \
               AND event_type='step.leased' AND attempt=$6 ORDER BY seq DESC LIMIT 1",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .bind(forward.node().as_str())
        .bind(path_nodes)
        .bind(path_indices)
        .bind(i32::try_from(attempt).map_err(|_| corrupt("attempt"))?)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?
        .ok_or_else(|| corrupt("bounded derived script lease"))?;
        match versioned_script_retention(&detail, field)? {
            Some(days) => Ok(days),
            None => self
                .ready_path_script_retention(activity)
                .await?
                .ok_or_else(|| corrupt("legacy bounded script version")),
        }
    }

    async fn admit_path_script(
        &mut self,
        instance: &ExecutionInstanceId,
        activity: &ActivityName,
    ) -> Result<bool, StoreError> {
        let origin = self
            .snapshot
            .definition
            .node_for_instance(instance)
            .map(Node::origin)
            .cloned()
            .ok_or_else(|| corrupt("bounded leased node"))?;
        match origin {
            NodeOrigin::Compensation(forward) | NodeOrigin::Reconciliation(forward) => {
                if sagat_domain::scripts::digest_of_activity(activity.as_str()).is_none() {
                    return Ok(true);
                }
                let field = if instance.node().is_compensation() {
                    "compensationScriptRetentionDays"
                } else {
                    "reconciliationScriptRetentionDays"
                };
                let forward = ExecutionInstanceId::in_path(instance.path().clone(), forward);
                let days = self
                    .inherited_path_script_retention(&forward, field, activity)
                    .await?;
                self.script_retention.insert(
                    instance.clone(),
                    ScriptRetentionPins {
                        script: Some(days),
                        ..ScriptRetentionPins::default()
                    },
                );
                Ok(true)
            }
            NodeOrigin::Declared => {
                let companion_activity = |node: NodeId| {
                    let candidate = ExecutionInstanceId::in_path(instance.path().clone(), node);
                    self.snapshot
                        .definition
                        .node_for_instance(&candidate)
                        .and_then(|node| match node.kind() {
                            NodeKind::Activity { activity, .. } => Some(activity.clone()),
                            _ => None,
                        })
                };
                let compensation = companion_activity(instance.node().compensation());
                let reconciliation = companion_activity(instance.node().reconciliation());
                let mut pins = ScriptRetentionPins::default();
                for (role, candidate) in [
                    (0_u8, Some(activity.clone())),
                    (1_u8, compensation),
                    (2_u8, reconciliation),
                ] {
                    let Some(candidate) = candidate else {
                        continue;
                    };
                    match self.current_path_script_retention(&candidate).await? {
                        ScriptRetentionDecision::NotScript => {}
                        ScriptRetentionDecision::Refused => return Ok(false),
                        ScriptRetentionDecision::Pinned(days) => match role {
                            0 => pins.script = Some(days),
                            1 => pins.compensation = Some(days),
                            _ => pins.reconciliation = Some(days),
                        },
                    }
                }
                if pins.script.is_some()
                    || pins.compensation.is_some()
                    || pins.reconciliation.is_some()
                {
                    self.script_retention.insert(instance.clone(), pins);
                }
                Ok(true)
            }
        }
    }

    async fn record_path_breaker(
        &mut self,
        instance: &ExecutionInstanceId,
        event: &RunEvent,
    ) -> Result<(), StoreError> {
        if !instance.node().is_compensation() {
            return Ok(());
        }
        let succeeded = match event {
            RunEvent::StepCompleted { .. } => Some(true),
            RunEvent::StepRetryScheduled { .. }
            | RunEvent::StepFailed { fence: Some(_), .. }
            | RunEvent::StepNeedsReview { fence: Some(_), .. } => Some(false),
            _ => None,
        };
        let Some(succeeded) = succeeded else {
            return Ok(());
        };
        let Some(NodeKind::Activity { activity, .. }) = self
            .snapshot
            .definition
            .node_for_instance(instance)
            .map(Node::kind)
        else {
            return Ok(());
        };
        let tenant = self.snapshot.info.tenant_id.as_uuid();
        let namespace = self.snapshot.info.namespace_id.as_uuid();
        if succeeded {
            crate::breaker::record_success(
                &mut self.tx,
                self.breaker,
                tenant,
                namespace,
                activity.as_str(),
            )
            .await?;
        } else if crate::breaker::record_failure(
            &mut self.tx,
            self.breaker,
            tenant,
            namespace,
            activity.as_str(),
        )
        .await?
        {
            self.signals.push(Signal::BreakerOpened);
        }
        Ok(())
    }

    async fn ensure_path_node_kind(
        &mut self,
        instance: &ExecutionInstanceId,
        node_kind: &str,
    ) -> Result<(), StoreError> {
        let (path_nodes, path_indices) = stored_path_parts(instance.path());
        let updated = sqlx::query(
            "UPDATE steps SET node_kind=$6 WHERE tenant_id=$1 AND run_id=$2 \
               AND instance_path_nodes=$3 AND instance_path_indices=$4 AND node=$5 \
               AND node_kind IN ('legacyControl',$6)",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .bind(path_nodes)
        .bind(path_indices)
        .bind(instance.node().as_str())
        .bind(node_kind)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        if updated.rows_affected() != 1 {
            return Err(corrupt("immutable step node kind"));
        }
        Ok(())
    }

    async fn consume_path_reservation(
        &mut self,
        instance: &ExecutionInstanceId,
    ) -> Result<(), StoreError> {
        let (path_nodes, path_indices) = stored_path_parts(instance.path());
        let collection = sqlx::query(
            "UPDATE collection_iterations SET remaining_reservations=remaining_reservations-1, \
                    updated_at=$5 WHERE tenant_id=$1 AND run_id=$2 \
               AND entry_path_nodes=$3 AND entry_path_indices=$4 \
               AND lifecycle IN ('ADMITTED','CHECKPOINTED','DRAINED') \
               AND remaining_reservations>0",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .bind(self.now)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        let loop_iteration = sqlx::query(
            "UPDATE loop_iterations SET remaining_reservations=remaining_reservations-1, \
                    updated_at=$5 WHERE tenant_id=$1 AND run_id=$2 \
               AND entry_path_nodes=$3 AND entry_path_indices=$4 \
               AND lifecycle IN ('ADMITTED','CHECKPOINTED','DRAINED') \
               AND remaining_reservations>0",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .bind(path_nodes)
        .bind(path_indices)
        .bind(self.now)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if collection + loop_iteration != 1 {
            return Err(corrupt("scoped instance reservation"));
        }
        let usage = sqlx::query(
            "UPDATE run_instance_usage SET admitted_instances=admitted_instances+1, \
                    reserved_instances=reserved_instances-1 \
             WHERE tenant_id=$1 AND run_id=$2 AND reserved_instances>0",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        if usage.rows_affected() != 1 {
            return Err(corrupt("scoped instance usage"));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn upsert_path_instance(
        &mut self,
        instance: &ExecutionInstanceId,
        hinted: bool,
        rehinted: bool,
        output: Option<&Vec<u8>>,
        effects: &Effects,
    ) -> Result<(), StoreError> {
        let step = self
            .snapshot
            .state
            .local_instance(instance.node())
            .cloned()
            .ok_or_else(|| corrupt("bounded step projection"))?;
        let waiting_kind = match step.waiting_on() {
            None => None,
            Some(WaitingOn::Reconciliation) => Some("RECONCILIATION"),
            Some(WaitingOn::Operator { .. }) => Some("OPERATOR"),
            Some(_) => return Err(corrupt("bounded step waiting kind")),
        };
        if step.condition().is_some() || step.switch_selection().is_some() {
            return Err(corrupt("bounded control projection"));
        }
        let declared = self.snapshot.definition.node_for_instance(instance);
        let kind = declared.map_or("FORWARD", |node| node.origin().label());
        let node_kind = declared.map_or("activity", |node| node.kind().label());
        let status = step.status().as_str();
        let attempt = i32::try_from(step.attempt()).map_err(|_| corrupt("attempt"))?;
        let generation = i64::try_from(step.generation()).map_err(|_| corrupt("generation"))?;
        let grant = effects
            .lease
            .as_ref()
            .filter(|grant| &grant.instance == instance);
        let renewal = effects
            .renewal
            .as_ref()
            .filter(|renewal| &renewal.instance == instance);
        let reported = effects
            .reported
            .as_ref()
            .filter(|reported| &reported.instance == instance);
        let leased = matches!(step.status(), StepStatus::Leased | StepStatus::Running);
        let fresh_grant = grant.filter(|_| leased);
        let (path_nodes, path_indices) = stored_path_parts(instance.path());
        let step_uuid = instance_step_id(
            self.snapshot.info.version_id,
            self.snapshot.info.run_id,
            instance,
        );
        if !self.stored.contains(instance) {
            if !matches!(step.status(), StepStatus::Pending | StepStatus::Ready) {
                return Err(corrupt("bounded step created after start"));
            }
            let activity = declared.and_then(|node| match node.kind() {
                NodeKind::Activity { activity, .. } => Some(activity.as_str().to_owned()),
                _ => None,
            });
            if step.status() == StepStatus::Ready && activity.is_none() {
                return Err(corrupt("bounded ready step activity"));
            }
            let secrets = declared
                .map(|node| {
                    node.secrets()
                        .iter()
                        .map(|name| name.as_str().to_owned())
                        .collect::<Vec<_>>()
                })
                .filter(|names| !names.is_empty());
            if !matches!(declared.map(Node::kind), Some(NodeKind::Return { .. })) {
                self.consume_path_reservation(instance).await?;
            }
            sqlx::query(
                "INSERT INTO steps \
                 (tenant_id,run_id,node,step_id,namespace_id,status,attempt,generation, \
                  cancel_requested,routed,timed_out,failure_code,next_attempt_at,activity, \
                  ready_at,last_hinted_at,output_digest,updated_at,kind,instance_path_nodes, \
                  instance_path_indices,waiting_kind,secrets,node_kind) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14, \
                   CASE WHEN $6='READY' THEN $15 END,CASE WHEN $16 THEN $15 END,$17,$15,$20, \
                   $18,$19,$21,$22,$23)",
            )
            .bind(self.snapshot.info.tenant_id.as_uuid())
            .bind(self.snapshot.info.run_id.as_uuid())
            .bind(instance.node().as_str())
            .bind(step_uuid)
            .bind(self.snapshot.info.namespace_id.as_uuid())
            .bind(status)
            .bind(attempt)
            .bind(generation)
            .bind(step.cancel_requested())
            .bind(step.routed())
            .bind(step.timed_out())
            .bind(step.failure().map(FailureCode::as_str))
            .bind(step.next_attempt_at())
            .bind(activity.as_deref())
            .bind(self.now)
            .bind(hinted)
            .bind(output.cloned())
            .bind(path_nodes)
            .bind(path_indices)
            .bind(kind)
            .bind(waiting_kind)
            .bind(secrets)
            .bind(node_kind)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
            self.stored.insert(instance.clone());
            return Ok(());
        }

        self.ensure_path_node_kind(instance, node_kind).await?;
        let updated = sqlx::query(
            "UPDATE steps SET status=$6,attempt=$7,generation=$8,cancel_requested=$9,routed=$10, \
                    timed_out=$11,failure_code=$12,next_attempt_at=$13, \
                    ready_at=CASE WHEN $6='READY' \
                      THEN COALESCE(CASE WHEN status='READY' THEN ready_at END,$14) ELSE ready_at END, \
                    last_hinted_at=CASE WHEN $15 THEN $14 ELSE last_hinted_at END, \
                    hint_count=CASE WHEN $24 THEN hint_count+1 WHEN $15 THEN 0 ELSE hint_count END, \
                    lease_token_hash=CASE WHEN $6 IN ('LEASED','RUNNING') \
                      THEN COALESCE($19,lease_token_hash) END, \
                    lease_expires_at=CASE WHEN $6 NOT IN ('LEASED','RUNNING') THEN NULL \
                      WHEN $19::bytea IS NOT NULL THEN $14+$20*interval '1 microsecond' \
                      WHEN $23::bigint IS NOT NULL THEN LEAST($14+$23*interval '1 microsecond', \
                        COALESCE(attempt_deadline_at,'infinity')) ELSE lease_expires_at END, \
                    attempt_deadline_at=CASE WHEN $6 NOT IN ('LEASED','RUNNING') THEN NULL \
                      WHEN $19::bytea IS NOT NULL THEN CASE WHEN $21::bigint IS NOT NULL \
                        THEN $14+$21*interval '1 microsecond' END ELSE attempt_deadline_at END, \
                    leased_by=CASE WHEN $6 NOT IN ('LEASED','RUNNING') THEN NULL \
                      WHEN $19::bytea IS NOT NULL THEN $22 ELSE leased_by END, \
                    reported_lease_token_hash=CASE WHEN $6 IN ('LEASED','RUNNING') THEN NULL \
                      WHEN $25::bytea IS NOT NULL THEN $25 ELSE reported_lease_token_hash END, \
                    output_digest=COALESCE($16,output_digest),waiting_kind=$26,updated_at=$14 \
             WHERE tenant_id=$1 AND run_id=$2 AND node=$3 AND step_id=$4 AND namespace_id=$5 \
               AND instance_path_nodes=$17 AND instance_path_indices=$18",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .bind(instance.node().as_str())
        .bind(step_uuid)
        .bind(self.snapshot.info.namespace_id.as_uuid())
        .bind(status)
        .bind(attempt)
        .bind(generation)
        .bind(step.cancel_requested())
        .bind(step.routed())
        .bind(step.timed_out())
        .bind(step.failure().map(FailureCode::as_str))
        .bind(step.next_attempt_at())
        .bind(self.now)
        .bind(hinted)
        .bind(output.cloned())
        .bind(path_nodes)
        .bind(path_indices)
        .bind(fresh_grant.map(|grant| grant.token_hash.clone()))
        .bind(fresh_grant.map_or(0, |grant| micros(grant.ttl)))
        .bind(fresh_grant.and_then(|grant| grant.attempt_timeout).map(micros))
        .bind(fresh_grant.map(|grant| grant.worker_id.clone()))
        .bind(renewal.map(|renewal| micros(renewal.ttl)))
        .bind(rehinted)
        .bind(reported.map(|reported| reported.token_hash.clone()))
        .bind(waiting_kind)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if updated != 1 {
            return Err(corrupt("bounded step identity"));
        }
        Ok(())
    }

    async fn outbox_path_task_ready(
        &mut self,
        instance: &ExecutionInstanceId,
    ) -> Result<(), StoreError> {
        let Some(NodeKind::Activity { activity, .. }) = self
            .snapshot
            .definition
            .node_for_instance(instance)
            .map(Node::kind)
        else {
            return Err(corrupt("bounded ready step activity"));
        };
        let path = instance
            .path()
            .segments()
            .iter()
            .map(|segment| {
                serde_json::json!({
                    "scope": segment.scope_node().as_str(),
                    "index": segment.iteration_index(),
                })
            })
            .collect::<Vec<_>>();
        sqlx::query(
            "INSERT INTO event_outbox \
             (id,tenant_id,aggregate_id,event_type,routing_key,payload) \
             VALUES ($1,$2,$3,'task.ready',$4,$5)",
        )
        .bind(Uuid::now_v7())
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .bind(activity.as_str())
        .bind(serde_json::json!({
            "tenantId": self.snapshot.info.tenant_id.to_string(),
            "namespaceId": self.snapshot.info.namespace_id.to_string(),
            "runId": self.snapshot.info.run_id.to_string(),
            "node": instance.node().as_str(),
            "instancePath": path,
            "activity": activity.as_str(),
            "traceId": self.snapshot.info.trace_id,
        }))
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn lease_token_matches(
        &mut self,
        instance: &ExecutionInstanceId,
        token_hash: &[u8],
        require_unexpired: bool,
    ) -> Result<bool, StoreError> {
        let (nodes, indices) = stored_path_parts(instance.path());
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node=$3 \
               AND instance_path_nodes=$4 AND instance_path_indices=$5 \
               AND status IN ('LEASED','RUNNING') AND lease_token_hash=$6 \
               AND (NOT $7 OR lease_expires_at > clock_timestamp()))",
        )
        .bind(self.snapshot.info.tenant_id.as_uuid())
        .bind(self.snapshot.info.run_id.as_uuid())
        .bind(instance.node().as_str())
        .bind(nodes)
        .bind(indices)
        .bind(token_hash)
        .bind(require_unexpired)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)
    }
}

/// Keep the catalog transition and the payload reference inseparable at the storage boundary.
fn blob_output_matches(bytes: &[u8], blob: &sagat_application::blobs::BlobCommit) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    let Some(wrapper) = value.as_object().filter(|object| object.len() == 1) else {
        return false;
    };
    let Some(descriptor) = wrapper.get("$blob").and_then(serde_json::Value::as_object) else {
        return false;
    };
    descriptor.len() == 4
        && descriptor.get("id").and_then(serde_json::Value::as_str)
            == Some(blob.id.to_string().as_str())
        && descriptor.get("size").and_then(serde_json::Value::as_u64) == Some(blob.size)
        && descriptor.get("sha256").and_then(serde_json::Value::as_str)
            == Some(blob.digest.to_hex().as_str())
        && descriptor
            .get("contentType")
            .and_then(serde_json::Value::as_str)
            == Some(blob.content_type.as_str())
}
