//! Complete exact-path reads, shared by read-only preparation and current metadata commands.
use super::*;

impl PgRunStore {
    pub(super) async fn read_path_snapshot(
        &self,
        tx: &mut PgConnection,
        tenant: TenantId,
        run: RunId,
        path: InstancePath,
    ) -> Result<Option<PathExecutionSnapshot>, StoreError> {
        let row = sqlx::query(
            "SELECT id,tenant_id,namespace_id,workflow_id,workflow_version_id,status,run_version, \
                    stop_reason,stop_code,paused,correlation_id,started_by,trace_id,deadline_at, \
                    created_at,updated_at,finished_at,parent_run_id,root_run_id,parent_node, \
                    cancel_policy,depth,trace_parent_span,rerun_of,origin \
             FROM runs WHERE tenant_id=$1 AND id=$2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let record = run_record(&row)?;
        let stop_code = col::<Option<String>>(&row, "stop_code")?
            .map(|code| FailureCode::parse(&code))
            .transpose()
            .map_err(|_| corrupt("stop code"))?;
        let (path_nodes, path_indices) = stored_path_parts(&path);
        let rows = sqlx::query(concat!(
            "SELECT node,status,attempt,generation,cancel_requested,routed,condition,switch_case_id, \
                    timed_out,failure_code,next_attempt_at,waiting_kind,due_at,correlation_key,event_type,event_since, \
                    child_run_id,approval_request_id,s.namespace_id,s.step_id,s.node_kind AS stored_node_kind, (",
            scoped_control_metadata_empty!(),
            ") AS control_metadata_empty, (",scoped_timer_metadata_valid!(),") AS timer_metadata_valid, (",scoped_event_metadata_valid!(),") AS event_metadata_valid \
             FROM steps s WHERE tenant_id=$1 AND run_id=$2 \
               AND instance_path_nodes=$3 AND instance_path_indices=$4 ORDER BY node",
        ))
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        if rows.is_empty() {
            return Ok(None);
        }
        let mut steps = BTreeMap::new();
        for row in &rows {
            let node = NodeId::parse_stored(&col::<String>(row, "node")?)
                .map_err(|_| corrupt("step node"))?;
            steps.insert(node, step_state(row)?);
        }

        let collection_rows = sqlx::query(
            "SELECT owner_node,kind,lifecycle,snapshot_digest,item_count,max_items, \
                    max_concurrency,next_admission_index,admitted_count,completed_count, \
                    active_count,result_digest,drain_kind,drain_iteration,drain_failure_code, \
                    drain_sequence,drain_outbox_id,finish_sequence,finish_outbox_id \
             FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2 \
               AND owner_path_nodes=$3 AND owner_path_indices=$4 ORDER BY owner_node",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let mut collections = BTreeMap::new();
        for row in &collection_rows {
            if col::<String>(row, "kind")? != "COLLECTION" {
                return Err(corrupt("bounded collection kind"));
            }
            let node = NodeId::parse_stored(&col::<String>(row, "owner_node")?)
                .map_err(|_| corrupt("bounded collection owner"))?;
            if !steps.contains_key(&node) {
                return Err(corrupt("bounded collection owner"));
            }
            let phase = match col::<String>(row, "lifecycle")?.as_str() {
                "OPEN" => CollectionPhase::Open,
                "DRAINING" => CollectionPhase::Draining,
                "SUCCEEDED" => CollectionPhase::Succeeded,
                "FAILED" => CollectionPhase::Failed,
                "CANCELLED" => CollectionPhase::Cancelled,
                _ => return Err(corrupt("bounded collection lifecycle")),
            };
            let result_digest = col::<Option<Vec<u8>>>(row, "result_digest")?
                .map(|digest| stored_digest(digest, "bounded collection result"))
                .transpose()?;
            let drain_sequence = col::<Option<i64>>(row, "drain_sequence")?;
            let drain_outbox_id = col::<Option<Uuid>>(row, "drain_outbox_id")?;
            let finish_sequence = col::<Option<i64>>(row, "finish_sequence")?;
            let finish_outbox_id = col::<Option<Uuid>>(row, "finish_outbox_id")?;
            let drain_reason = match (
                col::<Option<String>>(row, "drain_kind")?.as_deref(),
                col::<Option<i32>>(row, "drain_iteration")?,
                col::<Option<String>>(row, "drain_failure_code")?,
            ) {
                (None, None, None) => None,
                (Some("FAILED"), Some(iteration), Some(code)) => {
                    Some(CollectionDrainReason::Failed {
                        iteration: u32::try_from(iteration)
                            .map_err(|_| corrupt("bounded collection drain iteration"))?,
                        code: FailureCode::parse(&code)
                            .map_err(|_| corrupt("bounded collection drain code"))?,
                    })
                }
                (Some("RUN_DATA_LIMIT"), None, Some(code))
                    if code == FailureCode::run_data_limit().as_str()
                        && drain_sequence.is_some_and(|sequence| sequence > 0)
                        && drain_outbox_id.is_some() =>
                {
                    Some(CollectionDrainReason::RunDataLimit)
                }
                (Some("STORAGE_QUOTA"), None, Some(code))
                    if code == FailureCode::storage_quota_exceeded().as_str() =>
                {
                    Some(CollectionDrainReason::StorageQuotaExceeded)
                }
                (Some("CANCELLED"), None, None) => Some(CollectionDrainReason::Cancelled),
                _ => return Err(corrupt("bounded collection drain reason")),
            };
            if matches!(&drain_reason, Some(CollectionDrainReason::RunDataLimit)) {
                let run_data_limit = FailureCode::run_data_limit();
                let owner = steps
                    .get(&node)
                    .ok_or_else(|| corrupt("bounded collection owner"))?;
                let owner_matches = match phase {
                    CollectionPhase::Draining => {
                        result_digest.is_none()
                            && finish_sequence.is_none()
                            && finish_outbox_id.is_none()
                            && owner.status() == StepStatus::Waiting
                            && matches!(owner.waiting_on(), Some(WaitingOn::Collection))
                    }
                    CollectionPhase::Failed => {
                        result_digest.is_none()
                            && finish_sequence.is_some_and(|sequence| sequence > 0)
                            && finish_outbox_id.is_some()
                            && owner.status() == StepStatus::Failed
                            && owner.waiting_on().is_none()
                            && owner.failure() == Some(&run_data_limit)
                    }
                    _ => false,
                };
                if !owner_matches {
                    return Err(corrupt("bounded collection data-limit projection"));
                }
            }
            let next_admission = u32::try_from(col::<i32>(row, "next_admission_index")?)
                .map_err(|_| corrupt("bounded collection next admission"))?;
            if col::<i32>(row, "admitted_count")?
                != i32::try_from(next_admission).unwrap_or(i32::MAX)
            {
                return Err(corrupt("bounded collection admitted count"));
            }
            let scope = PathCollectionState::restore(
                phase,
                u32::try_from(col::<i32>(row, "item_count")?)
                    .map_err(|_| corrupt("bounded collection item count"))?,
                u32::try_from(col::<i32>(row, "max_items")?)
                    .map_err(|_| corrupt("bounded collection item limit"))?,
                u16::try_from(col::<i32>(row, "max_concurrency")?)
                    .map_err(|_| corrupt("bounded collection concurrency"))?,
                stored_digest(col(row, "snapshot_digest")?, "bounded collection snapshot")?,
                next_admission,
                u32::try_from(col::<i32>(row, "completed_count")?)
                    .map_err(|_| corrupt("bounded collection completed count"))?,
                u16::try_from(col::<i32>(row, "active_count")?)
                    .map_err(|_| corrupt("bounded collection active count"))?,
                result_digest,
                drain_reason,
            )
            .map_err(|_| corrupt("bounded collection scalar progress"))?;
            if collections.insert(node, scope).is_some() {
                return Err(corrupt("bounded collection owner"));
            }
        }

        let loop_rows = sqlx::query(
            "SELECT owner_node,kind,lifecycle,max_iterations,created_at,deadline_at, \
                    initial_carry_digest,current_carry_digest,next_iteration,active_iteration, \
                    delay_due_at,drain_kind,drain_iteration,drain_failure_code \
             FROM loop_scopes WHERE tenant_id=$1 AND run_id=$2 \
               AND owner_path_nodes=$3 AND owner_path_indices=$4 ORDER BY owner_node",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let mut loops = BTreeMap::new();
        for row in &loop_rows {
            let node = NodeId::parse_stored(&col::<String>(row, "owner_node")?)
                .map_err(|_| corrupt("bounded loop owner"))?;
            let owner = steps
                .get(&node)
                .ok_or_else(|| corrupt("bounded loop owner"))?;
            let kind = match col::<String>(row, "kind")?.as_str() {
                "WHILE" => LoopKind::While,
                "REPEAT_UNTIL" => LoopKind::RepeatUntil,
                _ => return Err(corrupt("bounded loop kind")),
            };
            let phase = match col::<String>(row, "lifecycle")?.as_str() {
                "READY" => LoopPhase::Ready,
                "RUNNING" => LoopPhase::Running,
                "DELAYING" => LoopPhase::Delaying,
                "DRAINING" => LoopPhase::Draining,
                "SUCCEEDED" => LoopPhase::Succeeded,
                "FAILED" => LoopPhase::Failed,
                "CANCELLED" => LoopPhase::Cancelled,
                _ => return Err(corrupt("bounded loop lifecycle")),
            };
            let drain_reason = match (
                col::<Option<String>>(row, "drain_kind")?.as_deref(),
                col::<Option<i32>>(row, "drain_iteration")?,
                col::<Option<String>>(row, "drain_failure_code")?,
            ) {
                (None, None, None) => None,
                (Some("FAILED"), Some(iteration), Some(code)) => Some(LoopDrainReason::Failed {
                    iteration: u32::try_from(iteration)
                        .map_err(|_| corrupt("bounded loop drain iteration"))?,
                    code: FailureCode::parse(&code)
                        .map_err(|_| corrupt("bounded loop drain code"))?,
                }),
                (Some("TIMED_OUT"), None, Some(code)) if code == "RUN_TIMED_OUT" => {
                    Some(LoopDrainReason::TimedOut)
                }
                (Some("CANCELLED"), None, None) => Some(LoopDrainReason::Cancelled),
                _ => return Err(corrupt("bounded loop drain reason")),
            };
            let scope = PathLoopState::restore(
                kind,
                phase,
                u32::try_from(col::<i32>(row, "max_iterations")?)
                    .map_err(|_| corrupt("bounded loop iteration limit"))?,
                col(row, "created_at")?,
                col(row, "deadline_at")?,
                stored_digest(
                    col(row, "initial_carry_digest")?,
                    "bounded loop initial carry",
                )?,
                stored_digest(
                    col(row, "current_carry_digest")?,
                    "bounded loop current carry",
                )?,
                u32::try_from(col::<i32>(row, "next_iteration")?)
                    .map_err(|_| corrupt("bounded loop next iteration"))?,
                col::<Option<i32>>(row, "active_iteration")?
                    .map(|index| {
                        u32::try_from(index).map_err(|_| corrupt("bounded loop active iteration"))
                    })
                    .transpose()?,
                col(row, "delay_due_at")?,
                drain_reason,
                owner.failure().cloned(),
            )
            .map_err(|_| corrupt("bounded loop scalar progress"))?;
            if loops.insert(node, scope).is_some() {
                return Err(corrupt("bounded loop owner"));
            }
        }

        let definition = self
            .definition(&mut *tx, tenant, record.info.version_id)
            .await?;
        let mut ancestors = Vec::with_capacity(path.depth());
        let mut owner_nodes = Vec::<String>::new();
        let mut owner_indices = Vec::<i32>::new();
        for segment in path.segments() {
            let index =
                i32::try_from(segment.iteration_index()).map_err(|_| corrupt("iteration index"))?;
            let mut scopes = sqlx::query(
                "SELECT 'COLLECTION' AS family,scope.lifecycle,member.slot_held AS active, \
                        scope.result_digest,scope.drain_kind,scope.drain_iteration, \
                        scope.drain_failure_code,scope.drain_sequence,scope.drain_outbox_id, \
                        scope.finish_sequence,scope.finish_outbox_id,owner.status AS owner_status, \
                        owner.waiting_kind AS owner_waiting_kind, \
                        owner.failure_code AS owner_failure_code,member.entry_node, \
                        member.entry_dispatch,member.activity,member.outbox_id, \
                        admission.detail->>'entry_dispatch' AS journal_dispatch \
                 FROM iteration_scopes scope JOIN collection_iterations member \
                   ON member.tenant_id=scope.tenant_id AND member.run_id=scope.run_id \
                  AND member.scope_id=scope.scope_id AND member.iteration_index=$6 \
                 LEFT JOIN run_events admission ON admission.tenant_id=member.tenant_id AND admission.run_id=member.run_id \
                  AND admission.seq=member.admitted_sequence AND admission.event_type='collection.iteration_admitted' \
                 LEFT JOIN steps owner \
                   ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
                  AND owner.instance_path_nodes=scope.owner_path_nodes \
                  AND owner.instance_path_indices=scope.owner_path_indices \
                  AND owner.node=scope.owner_node \
                 WHERE scope.tenant_id=$1 AND scope.run_id=$2 \
                   AND scope.owner_path_nodes=$3 AND scope.owner_path_indices=$4 \
                   AND scope.owner_node=$5 \
                 UNION ALL \
                 SELECT 'LOOP',scope.lifecycle,coalesce(scope.active_iteration=$6,false), \
                        NULL::bytea,NULL::text,NULL::integer,NULL::text,NULL::bigint,NULL::uuid, \
                        NULL::bigint,NULL::uuid,NULL::text,NULL::text,NULL::text, \
                        NULL::text,NULL::text,NULL::text,NULL::uuid,NULL::text \
                 FROM loop_scopes scope WHERE scope.tenant_id=$1 AND scope.run_id=$2 \
                   AND scope.owner_path_nodes=$3 AND scope.owner_path_indices=$4 \
                   AND scope.owner_node=$5",
            )
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .bind(&owner_nodes)
            .bind(&owner_indices)
            .bind(segment.scope_node().as_str())
            .bind(index)
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?;
            if scopes.len() != 1 {
                return Err(corrupt("path ancestor scope"));
            }
            let scope = scopes.remove(0);
            let owner = ExecutionInstanceId::in_path(
                instance_path(owner_nodes.clone(), owner_indices.clone(), "ancestor path")?,
                segment.scope_node().clone(),
            );
            let lifecycle = col::<String>(&scope, "lifecycle")?;
            let active = col::<bool>(&scope, "active")?;
            ancestors.push(match col::<String>(&scope, "family")?.as_str() {
                "COLLECTION" => {
                    let entry = ExecutionInstanceId::in_path(
                        owner
                            .path()
                            .child(owner.node().clone(), segment.iteration_index())
                            .map_err(|_| corrupt("collection entry path"))?,
                        NodeId::parse_stored(&col::<String>(&scope, "entry_node")?)
                            .map_err(|_| corrupt("collection entry node"))?,
                    );
                    validate_collection_entry_dispatch(&definition, &entry, &scope)?;
                    let phase = match lifecycle.as_str() {
                        "OPEN" => CollectionPhase::Open,
                        "DRAINING" => CollectionPhase::Draining,
                        "SUCCEEDED" => CollectionPhase::Succeeded,
                        "FAILED" => CollectionPhase::Failed,
                        "CANCELLED" => CollectionPhase::Cancelled,
                        _ => return Err(corrupt("collection lifecycle")),
                    };
                    if col::<Option<String>>(&scope, "drain_kind")?.as_deref()
                        == Some("RUN_DATA_LIMIT")
                    {
                        let run_data_limit = FailureCode::run_data_limit();
                        let drain_matches = col::<Option<i32>>(&scope, "drain_iteration")?
                            .is_none()
                            && col::<Option<String>>(&scope, "drain_failure_code")?.as_deref()
                                == Some(run_data_limit.as_str())
                            && col::<Option<i64>>(&scope, "drain_sequence")?
                                .is_some_and(|sequence| sequence > 0)
                            && col::<Option<Uuid>>(&scope, "drain_outbox_id")?.is_some()
                            && col::<Option<Vec<u8>>>(&scope, "result_digest")?.is_none();
                        let lifecycle_matches = match phase {
                            CollectionPhase::Draining => {
                                col::<Option<i64>>(&scope, "finish_sequence")?.is_none()
                                    && col::<Option<Uuid>>(&scope, "finish_outbox_id")?.is_none()
                                    && col::<Option<String>>(&scope, "owner_status")?.as_deref()
                                        == Some("WAITING")
                                    && col::<Option<String>>(&scope, "owner_waiting_kind")?
                                        .as_deref()
                                        == Some("COLLECTION")
                            }
                            CollectionPhase::Failed => {
                                col::<Option<i64>>(&scope, "finish_sequence")?
                                    .is_some_and(|sequence| sequence > 0)
                                    && col::<Option<Uuid>>(&scope, "finish_outbox_id")?.is_some()
                                    && col::<Option<String>>(&scope, "owner_status")?.as_deref()
                                        == Some("FAILED")
                                    && col::<Option<String>>(&scope, "owner_waiting_kind")?
                                        .is_none()
                                    && col::<Option<String>>(&scope, "owner_failure_code")?
                                        .as_deref()
                                        == Some(run_data_limit.as_str())
                            }
                            _ => false,
                        };
                        if !drain_matches || !lifecycle_matches {
                            return Err(corrupt("collection data-limit ancestor projection"));
                        }
                    }
                    AncestorExecutionFence::Collection {
                        owner,
                        iteration_index: segment.iteration_index(),
                        phase,
                        active,
                    }
                }
                "LOOP" => AncestorExecutionFence::Loop {
                    owner,
                    iteration_index: segment.iteration_index(),
                    phase: match lifecycle.as_str() {
                        "READY" => LoopPhase::Ready,
                        "RUNNING" => LoopPhase::Running,
                        "DELAYING" => LoopPhase::Delaying,
                        "DRAINING" => LoopPhase::Draining,
                        "SUCCEEDED" => LoopPhase::Succeeded,
                        "FAILED" => LoopPhase::Failed,
                        "CANCELLED" => LoopPhase::Cancelled,
                        _ => return Err(corrupt("loop lifecycle")),
                    },
                    active,
                },
                _ => return Err(corrupt("ancestor scope kind")),
            });
            owner_nodes.push(segment.scope_node().as_str().to_owned());
            owner_indices.push(index);
        }

        // A scoped compensation is authorized by its owning ancestor saga. Load only entries for
        // this exact member path; sibling iteration plans remain in PostgreSQL. Local saga headers
        // are included for bounded control progression, without hydrating their member history.
        let mut saga_owners = BTreeMap::<ExecutionInstanceId, Option<u32>>::new();
        for ancestor in &ancestors {
            saga_owners.insert(ancestor.owner().clone(), Some(ancestor.iteration_index()));
        }
        for node in collections.keys().chain(loops.keys()) {
            saga_owners
                .entry(ExecutionInstanceId::in_path(path.clone(), node.clone()))
                .or_insert(None);
        }
        let mut scope_sagas = BTreeMap::new();
        for (owner, relevant_iteration) in saga_owners {
            let (owner_path_nodes, owner_path_indices) = stored_path_parts(owner.path());
            let Some(saga_row) = sqlx::query(
                "SELECT phase,cause_code,outcome,during_stop FROM scope_sagas \
                 WHERE tenant_id=$1 AND run_id=$2 AND owner_path_nodes=$3 \
                   AND owner_path_indices=$4 AND owner_node=$5",
            )
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .bind(&owner_path_nodes)
            .bind(&owner_path_indices)
            .bind(owner.node().as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?
            else {
                continue;
            };
            let mut entries = BTreeMap::new();
            let mut irreversible = Vec::new();
            if let Some(iteration) = relevant_iteration {
                let iteration =
                    i32::try_from(iteration).map_err(|_| corrupt("scope saga iteration"))?;
                let entry_rows = sqlx::query(
                    "SELECT node,kind,after_iterations,after_nodes FROM scope_saga_entries \
                     WHERE tenant_id=$1 AND run_id=$2 AND owner_path_nodes=$3 \
                       AND owner_path_indices=$4 AND owner_node=$5 AND iteration_index=$6 \
                     ORDER BY node",
                )
                .bind(tenant.as_uuid())
                .bind(run.as_uuid())
                .bind(&owner_path_nodes)
                .bind(&owner_path_indices)
                .bind(owner.node().as_str())
                .bind(iteration)
                .fetch_all(&mut *tx)
                .await
                .map_err(store_error)?;
                let member =
                    |iteration: i32, node: String| -> Result<ExecutionInstanceId, StoreError> {
                        let index = u32::try_from(iteration)
                            .map_err(|_| corrupt("bounded scope saga dependency"))?;
                        let member_path = owner
                            .path()
                            .child(owner.node().clone(), index)
                            .map_err(|_| corrupt("bounded scope saga member path"))?;
                        let node = NodeId::parse_stored(&node)
                            .map_err(|_| corrupt("bounded scope saga member"))?;
                        Ok(ExecutionInstanceId::in_path(member_path, node))
                    };
                for entry in &entry_rows {
                    let node = NodeId::parse_stored(&col::<String>(entry, "node")?)
                        .map_err(|_| corrupt("bounded scope saga member"))?;
                    match col::<String>(entry, "kind")?.as_str() {
                        "COMPENSATE" => {
                            let after_iterations = col::<Vec<i32>>(entry, "after_iterations")?;
                            let after_nodes = col::<Vec<String>>(entry, "after_nodes")?;
                            if after_iterations.len() != after_nodes.len() {
                                return Err(corrupt("bounded scope saga dependencies"));
                            }
                            let after = after_iterations
                                .into_iter()
                                .zip(after_nodes)
                                .map(|(iteration, node)| member(iteration, node))
                                .collect::<Result<Vec<_>, _>>()?;
                            entries.insert(node, after);
                        }
                        "IRREVERSIBLE" => irreversible.push(node),
                        _ => return Err(corrupt("bounded scope saga entry kind")),
                    }
                }
            }
            let saga = PathScopeSagaState::restore(
                owner.clone(),
                col::<String>(&saga_row, "phase")?
                    .parse::<SagaPhase>()
                    .map_err(|_| corrupt("bounded scope saga phase"))?,
                FailureCode::parse(&col::<String>(&saga_row, "cause_code")?)
                    .map_err(|_| corrupt("bounded scope saga cause"))?,
                entries,
                irreversible,
                col::<Option<String>>(&saga_row, "outcome")?
                    .map(|outcome| outcome.parse::<SagaOutcome>())
                    .transpose()
                    .map_err(|()| corrupt("bounded scope saga outcome"))?,
                col(&saga_row, "during_stop")?,
            );
            scope_sagas.insert(owner, saga);
        }
        let input_row = sqlx::query(
            "SELECT payload.content,runs.input_blob_id FROM runs JOIN payloads payload \
               ON payload.tenant_id=runs.tenant_id AND payload.digest=runs.input_digest \
             WHERE runs.tenant_id=$1 AND runs.id=$2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let output_rows = sqlx::query(
            "SELECT step.node,payload.content FROM steps step JOIN payloads payload \
               ON payload.tenant_id=step.tenant_id AND payload.digest=step.output_digest \
             WHERE step.tenant_id=$1 AND step.run_id=$2 \
               AND step.instance_path_nodes=$3 AND step.instance_path_indices=$4 \
             ORDER BY step.node",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let mut outputs = BTreeMap::new();
        for row in &output_rows {
            let node = NodeId::parse_stored(&col::<String>(row, "node")?)
                .map_err(|_| corrupt("bounded output node"))?;
            outputs.insert(
                ExecutionInstanceId::in_path(path.clone(), node),
                col::<Vec<u8>>(row, "content")?,
            );
        }
        let state_row = sqlx::query(
            "SELECT payload.content,state.state_revision \
             FROM workflow_operation_states state JOIN payloads payload \
               ON payload.tenant_id=state.tenant_id AND payload.digest=state.state_digest \
             WHERE state.tenant_id=$1 AND state.run_id=$2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let mut scope_rows = sqlx::query(
            "SELECT 'COLLECTION' AS family,member.iteration_index,payload.content \
             FROM collection_iterations member JOIN payloads payload \
               ON payload.tenant_id=member.tenant_id AND payload.digest=member.item_digest \
             WHERE member.tenant_id=$1 AND member.run_id=$2 \
               AND member.entry_path_nodes=$3 AND member.entry_path_indices=$4 \
             UNION ALL \
             SELECT 'LOOP',member.iteration_index,payload.content \
             FROM loop_iterations member JOIN loop_scopes scope \
               ON scope.tenant_id=member.tenant_id AND scope.run_id=member.run_id \
              AND scope.owner_instance_id=member.owner_instance_id \
             JOIN payloads payload ON payload.tenant_id=scope.tenant_id \
              AND payload.digest=CASE WHEN member.iteration_index=0 \
                   THEN scope.initial_carry_digest ELSE ( \
                       SELECT predecessor.carry_digest FROM loop_iterations predecessor \
                        WHERE predecessor.tenant_id=member.tenant_id \
                          AND predecessor.run_id=member.run_id \
                          AND predecessor.owner_instance_id=member.owner_instance_id \
                          AND predecessor.iteration_index=member.iteration_index-1) END \
             WHERE member.tenant_id=$1 AND member.run_id=$2 \
               AND member.entry_path_nodes=$3 AND member.entry_path_indices=$4",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        if scope_rows.len() != 1 {
            return Err(corrupt("bounded scope input"));
        }
        let scope_row = scope_rows.remove(0);
        let scope_index = u32::try_from(col::<i32>(&scope_row, "iteration_index")?)
            .map_err(|_| corrupt("bounded scope input index"))?;
        let scope_input = match col::<String>(&scope_row, "family")?.as_str() {
            "COLLECTION" => ScopeInput::Collection {
                index: scope_index,
                item: col(&scope_row, "content")?,
            },
            "LOOP" => ScopeInput::Loop {
                iteration: scope_index,
                carry: col(&scope_row, "content")?,
            },
            _ => return Err(corrupt("bounded scope input kind")),
        };
        let mut scope_inputs = BTreeMap::new();
        scope_inputs.insert(path.clone(), scope_input);
        let operation_state = state_row
            .as_ref()
            .map(|row| col::<Vec<u8>>(row, "content"))
            .transpose()?;
        let state_revision = state_row
            .as_ref()
            .map(|row| {
                u64::try_from(col::<i64>(row, "state_revision")?)
                    .map_err(|_| corrupt("bounded operation state revision"))
            })
            .transpose()?;
        let data = RunData {
            run_id: Some(run),
            input: col(&input_row, "content")?,
            input_blob: col::<Option<Uuid>>(&input_row, "input_blob_id")?
                .map(sagat_domain::blobs::BlobId::new),
            state: operation_state,
            state_revision,
            outputs,
            scope_inputs,
        };
        for row in &rows {
            let node = NodeId::parse_stored(&col::<String>(row, "node")?)
                .map_err(|_| corrupt("step node"))?;
            let instance = ExecutionInstanceId::in_path(path.clone(), node.clone());
            let step = steps
                .get(&node)
                .ok_or_else(|| corrupt("bounded step identity"))?;
            validate_scoped_control_row(definition.node_for_instance(&instance), step, row)?;
            validate_scoped_event_identity(
                definition.node_for_instance(&instance),
                record.info.namespace_id,
                instance_step_id(record.info.version_id, run, &instance),
                row,
            )?;
        }
        for node in collections.keys() {
            let owner = ExecutionInstanceId::in_path(path.clone(), node.clone());
            if !matches!(
                definition.node_for_instance(&owner).map(Node::kind),
                Some(NodeKind::ForEach { .. })
            ) {
                return Err(corrupt("bounded collection definition"));
            }
        }
        for node in loops.keys() {
            let owner = ExecutionInstanceId::in_path(path.clone(), node.clone());
            if !matches!(
                definition.node_for_instance(&owner).map(Node::kind),
                Some(NodeKind::While { .. } | NodeKind::RepeatUntil { .. })
            ) {
                return Err(corrupt("bounded loop definition"));
            }
        }
        // Existential facts about work outside this exact path. Partial indexes contain only
        // open/live/review work, so completed sibling histories are never part of these probes.
        // Reads share either the read-only snapshot or the already-held run mutation lock.
        let (outside_nodes, outside_indices) = stored_path_parts(&path);
        let outside = sqlx::query(
            "SELECT \
             EXISTS(SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND status IN ('PENDING','READY','LEASED','RUNNING','RETRY_WAIT') \
               AND NOT(instance_path_nodes=$3 AND instance_path_indices=$4)) AS active, \
             EXISTS(SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND status IN ('READY','LEASED','RUNNING','RETRY_WAIT') \
               AND NOT(instance_path_nodes=$3 AND instance_path_indices=$4)) AS live, \
             EXISTS(SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 AND status='WAITING' \
               AND NOT(instance_path_nodes=$3 AND instance_path_indices=$4)) AS waiting, \
             EXISTS(SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 \
               AND status='WAITING' AND waiting_kind='OPERATOR' \
               AND NOT(instance_path_nodes=$3 AND instance_path_indices=$4)) AS reviewing, \
             (EXISTS(SELECT 1 FROM run_sagas WHERE tenant_id=$1 AND run_id=$2 \
               AND phase IN ('ABORTING','COMPENSATING')) OR \
              EXISTS(SELECT 1 FROM scope_sagas WHERE tenant_id=$1 AND run_id=$2 \
               AND phase='COMPENSATING')) AS compensating",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(outside_nodes)
        .bind(outside_indices)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let outside_work = RunWorkSummary::restore(
            col(&outside, "active")?,
            col(&outside, "live")?,
            col(&outside, "waiting")?,
            col(&outside, "reviewing")?,
        )
        .map_err(|_| corrupt("bounded global work summary"))?;
        let state = PathExecutionState::restore(
            GlobalExecutionFence::restore(
                run,
                record.status,
                record.stop_reason.map(|reason| (reason, stop_code)),
                record.paused,
                record.version,
                outside_work,
                col(&outside, "compensating")?,
            ),
            path,
            ancestors,
            steps,
            collections,
            loops,
            scope_sagas,
        )
        .map_err(|error| StoreError::Internal {
            detail: error.to_string(),
        })?;
        Ok(Some(PathExecutionSnapshot {
            info: record.info,
            definition,
            state,
            data,
        }))
    }
}
