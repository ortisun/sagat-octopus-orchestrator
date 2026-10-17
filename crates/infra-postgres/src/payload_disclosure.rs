//! Transactional PostgreSQL selection and auditing for classified payload disclosure.

use sagat_application::blobs::descriptor_json;
use sagat_application::classification::CarrierPointer;
use sagat_application::error::StoreError;
use sagat_application::payload_disclosure::{
    DisclosureEvaluator, DisclosureFailure, DisclosureRead, DisclosureRun, PayloadDisclosureStore,
    PreparedDisclosure, SelectedDisclosure, StoredDefinition,
};
use sagat_application::payloads::{ExternalContentKind, StoredPayload, StoredPayloadBytes};
use sagat_application::runs::RunStore;
use sagat_domain::audit::AuditRecord;
use sagat_domain::blobs::{BlobDescriptor, BlobId, ContentType};
use sagat_domain::classification::ClassificationRules;
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, Role, TenantId};
use sagat_domain::workflow::{Digest, NodeId};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{best_effort_denial_audit, col, corrupt, insert_audit, store_error};
use crate::pool::BeginSafely;
use crate::reveal_grants::has_effective_payload_reveal_grant;

impl PayloadDisclosureStore for PgRunStore {
    async fn disclosure_run(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<DisclosureRun>, StoreError> {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT namespace_id FROM runs WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map(|namespace| {
            namespace.map(|id| DisclosureRun {
                namespace_id: NamespaceId::new(id),
            })
        })
        .map_err(store_error)
    }

    async fn run_exists_outside(&self, tenant: TenantId, run: RunId) -> Result<bool, StoreError> {
        RunStore::run_exists_outside(self, tenant, run).await
    }

    async fn record_disclosure_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        RunStore::record_denial(self, audit).await
    }

    async fn disclose_and_audit(
        &self,
        prepared: &PreparedDisclosure,
        evaluator: &dyn DisclosureEvaluator,
        audits: &sagat_application::payload_disclosure::DisclosureAudits,
    ) -> Result<DisclosureRead, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let Some(run) = sqlx::query(
            "SELECT r.namespace_id, r.run_version, r.input_digest, r.input_blob_id, \
                    r.result_digest, r.workflow_version_id, r.origin, \
                    v.version_number, v.document, v.classification_rules \
             FROM runs r JOIN workflow_versions v \
               ON v.tenant_id = r.tenant_id AND v.id = r.workflow_version_id \
             WHERE r.tenant_id = $1 AND r.id = $2 FOR SHARE OF r",
        )
        .bind(prepared.tenant_id().as_uuid())
        .bind(prepared.run_id().as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        else {
            return Ok(DisclosureRead::Missing);
        };

        let namespace = NamespaceId::new(col::<Uuid>(&run, "namespace_id")?);
        if namespace != prepared.namespace_id() {
            return Err(corrupt("prepared disclosure namespace"));
        }
        lock_namespace(&mut tx, prepared.tenant_id(), namespace).await?;
        let Some(current_role) =
            current_role(&mut tx, prepared.tenant_id(), prepared.subject().as_str()).await?
        else {
            return Ok(DisclosureRead::Refused(
                DisclosureFailure::RunReadNotGranted,
            ));
        };
        let reveal_granted = has_effective_payload_reveal_grant(
            &mut tx,
            prepared.tenant_id(),
            namespace,
            prepared.subject(),
        )
        .await?;

        // The durable origin is deliberately read from `runs`, never from caller input or metadata.
        // It does not grant, deny or choose a carrier: copied values follow the receiving carrier.
        let _origin: Option<String> = col(&run, "origin")?;
        let run_version =
            u64::try_from(col::<i64>(&run, "run_version")?).map_err(|_| corrupt("run version"))?;
        let workflow_version = WorkflowVersionId::new(col(&run, "workflow_version_id")?);
        let version_number = u32::try_from(col::<i32>(&run, "version_number")?)
            .map_err(|_| corrupt("workflow version number"))?;
        let rules = u8::try_from(col::<i16>(&run, "classification_rules")?)
            .ok()
            .and_then(|value| ClassificationRules::try_from(value).ok())
            .ok_or_else(|| corrupt("classification rules"))?;
        let document: Vec<u8> = col(&run, "document")?;

        let (carrier, node_id, attempt, payload) = match prepared.selector() {
            sagat_application::payload_disclosure::DisclosureSelector::Input => {
                let digest: Vec<u8> = col(&run, "input_digest")?;
                let content = payload_content(&mut tx, prepared.tenant_id(), &digest).await?;
                let payload = match col::<Option<Uuid>>(&run, "input_blob_id")? {
                    Some(id) => {
                        let blob = load_input_blob(&mut tx, prepared.tenant_id(), namespace, id)
                            .await?
                            .ok_or_else(|| corrupt("committed run input blob"))?;
                        if descriptor_json(&blob.descriptor) != content {
                            return Err(corrupt("run input blob descriptor"));
                        }
                        StoredPayload::External {
                            size_bytes: blob.descriptor.size(),
                            content_kind: content_kind(blob.descriptor.content_type()),
                        }
                    }
                    None => StoredPayload::Inline(StoredPayloadBytes::new(content)),
                };
                (Some(CarrierPointer::Input), None, None, payload)
            }
            sagat_application::payload_disclosure::DisclosureSelector::Result => {
                let payload = match col::<Option<Vec<u8>>>(&run, "result_digest")? {
                    Some(digest) => StoredPayload::Inline(StoredPayloadBytes::new(
                        payload_content(&mut tx, prepared.tenant_id(), &digest).await?,
                    )),
                    None => StoredPayload::NoRetainedPayload,
                };
                (Some(CarrierPointer::Result), None, None, payload)
            }
            sagat_application::payload_disclosure::DisclosureSelector::Journal { sequence } => {
                let Ok(sequence) = i64::try_from(sequence) else {
                    return Ok(DisclosureRead::Missing);
                };
                let Some(event) = sqlx::query(
                    "SELECT e.event_type, e.node, e.attempt, e.payload_digest, \
                            e.instance_id, e.instance_path_nodes, e.instance_path_indices, p.content \
                     FROM run_events e LEFT JOIN payloads p \
                       ON p.tenant_id = e.tenant_id AND p.digest = e.payload_digest \
                     WHERE e.tenant_id = $1 AND e.run_id = $2 AND e.seq = $3",
                )
                .bind(prepared.tenant_id().as_uuid())
                .bind(prepared.run_id().as_uuid())
                .bind(sequence)
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_error)?
                else {
                    return Ok(DisclosureRead::Missing);
                };
                let event_type: String = col(&event, "event_type")?;
                let node: Option<String> = col(&event, "node")?;
                let attempt_i32: Option<i32> = col(&event, "attempt")?;
                let path_nodes: Option<Vec<String>> = col(&event, "instance_path_nodes")?;
                let path_indices: Option<Vec<i32>> = col(&event, "instance_path_indices")?;
                let carrier = node.as_deref().and_then(|node| {
                    journal_carrier(
                        &document,
                        &event_type,
                        node,
                        path_nodes.as_deref(),
                        path_indices.as_deref(),
                    )
                });
                let digest: Option<Vec<u8>> = col(&event, "payload_digest")?;
                let payload = match digest {
                    None => StoredPayload::NoRetainedPayload,
                    Some(_) => {
                        let content: Vec<u8> = col::<Option<Vec<u8>>>(&event, "content")?
                            .ok_or_else(|| corrupt("journal payload"))?;
                        let instance: Option<Uuid> = col(&event, "instance_id")?;
                        match (node.as_deref(), attempt_i32) {
                            (Some(node), Some(attempt)) => {
                                let blobs = load_step_blobs(
                                    &mut tx,
                                    prepared.tenant_id(),
                                    namespace,
                                    prepared.run_id(),
                                    instance,
                                    path_nodes.as_deref(),
                                    path_indices.as_deref(),
                                    node,
                                    attempt,
                                )
                                .await?;
                                match blobs
                                    .into_iter()
                                    .find(|blob| descriptor_json(&blob.descriptor) == content)
                                {
                                    Some(blob) => StoredPayload::External {
                                        size_bytes: blob.descriptor.size(),
                                        content_kind: content_kind(blob.descriptor.content_type()),
                                    },
                                    None => StoredPayload::Inline(StoredPayloadBytes::new(content)),
                                }
                            }
                            _ => StoredPayload::Inline(StoredPayloadBytes::new(content)),
                        }
                    }
                };
                let attempt = attempt_i32
                    .map(u32::try_from)
                    .transpose()
                    .map_err(|_| corrupt("journal attempt"))?;
                (carrier, node, attempt, payload)
            }
        };

        let selected = SelectedDisclosure {
            run_version,
            workflow_version,
            version_number,
            classification_rules: rules,
            carrier,
            node_id,
            attempt,
            definition: StoredDefinition::new(document),
            payload,
        };
        let evaluated = evaluator.evaluate(selected, current_role, reveal_granted);
        let read = match evaluated {
            Ok(decision) => {
                if let Some(kind) = decision.audit {
                    append_audit(&mut tx, audits.for_kind(kind)).await?;
                }
                DisclosureRead::Ready(decision.disclosure)
            }
            Err(DisclosureFailure::PiiRevealNotGranted) => {
                best_effort_denial_audit(tx, &audits.reveal_denied).await;
                return Ok(DisclosureRead::Refused(
                    DisclosureFailure::PiiRevealNotGranted,
                ));
            }
            Err(DisclosureFailure::RunReadNotGranted) => {
                DisclosureRead::Refused(DisclosureFailure::RunReadNotGranted)
            }
        };
        tx.commit().await.map_err(store_error)?;
        Ok(read)
    }
}

async fn lock_namespace(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    let exists: Option<String> =
        sqlx::query_scalar("SELECT environment FROM public.sagat_lock_namespace($1, $2)")
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(store_error)?;
    exists.map(|_| ()).ok_or(StoreError::MissingReference)
}

async fn current_role(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    subject: &str,
) -> Result<Option<Role>, StoreError> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM public.sagat_lock_memberships($1, ARRAY[$2::text])")
            .bind(tenant.as_uuid())
            .bind(subject)
            .fetch_optional(&mut **tx)
            .await
            .map_err(store_error)?;
    role.map(|role| role.parse::<Role>().map_err(|_| corrupt("membership role")))
        .transpose()
}

async fn append_audit(
    tx: &mut Transaction<'_, Postgres>,
    audit: &AuditRecord,
) -> Result<(), StoreError> {
    insert_audit(tx, audit)
        .await
        .map_err(|error| StoreError::Unavailable {
            detail: format!("payload disclosure audit could not be appended: {error}"),
        })
}

async fn payload_content(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    digest: &[u8],
) -> Result<Vec<u8>, StoreError> {
    sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
        .bind(tenant.as_uuid())
        .bind(digest)
        .fetch_optional(&mut **tx)
        .await
        .map_err(store_error)?
        .ok_or_else(|| corrupt("retained payload"))
}

struct CatalogBlob {
    descriptor: BlobDescriptor,
}

async fn load_input_blob(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
    id: Uuid,
) -> Result<Option<CatalogBlob>, StoreError> {
    let row = sqlx::query(
        "SELECT id, size_bytes, sha256, content_type FROM blobs \
         WHERE tenant_id = $1 AND namespace_id = $2 AND state = 'COMMITTED' \
           AND id = $3 AND purpose = 'RUN_INPUT'",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    row.as_ref().map(blob_from_row).transpose()
}

#[allow(clippy::too_many_arguments)]
async fn load_step_blobs(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
    run: RunId,
    instance: Option<Uuid>,
    path_nodes: Option<&[String]>,
    path_indices: Option<&[i32]>,
    node: &str,
    attempt: i32,
) -> Result<Vec<CatalogBlob>, StoreError> {
    let rows = sqlx::query(
        "SELECT b.id, b.size_bytes, b.sha256, b.content_type \
         FROM blobs b JOIN steps s \
           ON s.tenant_id = b.tenant_id AND s.step_id = b.step_id \
         WHERE b.tenant_id = $1 AND b.namespace_id = $2 AND b.state = 'COMMITTED' \
           AND b.purpose = 'STEP_OUTPUT' AND b.run_id = $3 AND b.attempt = $4 \
           AND (($5::uuid IS NOT NULL AND s.step_id = $5) OR \
                ($5::uuid IS NULL AND s.node = $6 \
                 AND s.instance_path_nodes = COALESCE($7::text[], '{}') \
                 AND s.instance_path_indices = COALESCE($8::integer[], '{}'))) \
         ORDER BY b.committed_at DESC LIMIT 64",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(run.as_uuid())
    .bind(attempt)
    .bind(instance)
    .bind(node)
    .bind(path_nodes)
    .bind(path_indices)
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    rows.iter().map(blob_from_row).collect()
}

fn blob_from_row(row: &sqlx::postgres::PgRow) -> Result<CatalogBlob, StoreError> {
    let sha =
        <[u8; 32]>::try_from(col::<Vec<u8>>(row, "sha256")?).map_err(|_| corrupt("blob digest"))?;
    let size = u64::try_from(col::<i64>(row, "size_bytes")?).map_err(|_| corrupt("blob size"))?;
    let content_type = ContentType::parse(&col::<String>(row, "content_type")?)
        .map_err(|_| corrupt("blob content type"))?;
    Ok(CatalogBlob {
        descriptor: BlobDescriptor::new(
            BlobId::new(col(row, "id")?),
            size,
            Digest::new(sha),
            content_type,
        ),
    })
}

fn content_kind(content_type: &ContentType) -> ExternalContentKind {
    let media_type = content_type.as_str();
    if media_type == "application/json" || media_type.ends_with("+json") {
        ExternalContentKind::Json
    } else if media_type.starts_with("text/") {
        ExternalContentKind::Text
    } else if media_type == "application/octet-stream"
        || media_type.starts_with("image/")
        || media_type.starts_with("audio/")
        || media_type.starts_with("video/")
    {
        ExternalContentKind::Binary
    } else {
        ExternalContentKind::Other
    }
}

/// Resolves only the exact event node inside its persisted runtime scope. It never searches another
/// graph for a matching id and never guesses the last path segment as a body name.
fn journal_carrier(
    document: &[u8],
    event_type: &str,
    node: &str,
    path_nodes: Option<&[String]>,
    path_indices: Option<&[i32]>,
) -> Option<CarrierPointer> {
    let path_nodes = path_nodes.unwrap_or_default();
    let path_indices = path_indices.unwrap_or_default();
    if path_nodes.len() != path_indices.len() || path_nodes.len() > 8 {
        return None;
    }
    let document: Value = serde_json::from_slice(document).ok()?;
    let mut nodes = document.get("nodes")?.as_array()?;
    let bodies = document.get("bodies").and_then(Value::as_object);
    let mut current_body = None;
    for (owner, index) in path_nodes.iter().zip(path_indices) {
        let owner = nodes
            .iter()
            .find(|candidate| candidate.get("id").and_then(Value::as_str) == Some(owner))?;
        let kind = owner.get("type")?.as_str()?;
        let maximum = match kind {
            "forEach" => owner.get("maxItems")?.as_u64()?,
            "while" | "repeatUntil" => owner.get("maxIterations")?.as_u64()?,
            _ => return None,
        };
        let index = u64::try_from(*index).ok()?;
        if index >= maximum {
            return None;
        }
        let body = NodeId::parse(owner.get("body")?.as_str()?).ok()?;
        nodes = bodies?.get(body.as_str())?.get("nodes")?.as_array()?;
        current_body = Some(body);
    }
    let selected = nodes
        .iter()
        .find(|candidate| candidate.get("id").and_then(Value::as_str) == Some(node))?;
    if selected.get("database").is_some() {
        return None;
    }
    let source_kind = selected.get("type")?.as_str()?;
    let carries = match event_type {
        "step.completed" => matches!(source_kind, "activity" | "script" | "child"),
        "child.finished" => source_kind == "child",
        "wait.satisfied" => source_kind == "waitEvent",
        "reconciliation.concluded" => matches!(source_kind, "activity" | "script"),
        _ => false,
    };
    if !carries {
        return None;
    }
    let node = NodeId::parse(node).ok()?;
    Some(match current_body {
        Some(body) => CarrierPointer::BodyNodeOutput { body, node },
        None => CarrierPointer::NodeOutput(node),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_finished_requires_the_exact_child_node_in_its_declared_scope() {
        let document = br#"{
          "nodes":[
            {"id":"child","type":"child"},
            {"id":"activity","type":"activity"},
            {"id":"script","type":"script"},
            {"id":"wait","type":"waitEvent"},
            {"id":"db","type":"child","database":{}},
            {"id":"items","type":"forEach","body":"line","maxItems":2}
          ],
          "bodies":{"line":{"nodes":[{"id":"child","type":"child"}]}}
        }"#;
        assert_eq!(
            journal_carrier(document, "child.finished", "child", None, None),
            Some(CarrierPointer::NodeOutput(NodeId::parse("child").unwrap()))
        );
        assert_eq!(
            journal_carrier(
                document,
                "child.finished",
                "child",
                Some(&["items".into()]),
                Some(&[1]),
            ),
            Some(CarrierPointer::BodyNodeOutput {
                body: NodeId::parse("line").unwrap(),
                node: NodeId::parse("child").unwrap(),
            })
        );
        for node in ["activity", "script", "wait", "db", "missing"] {
            assert_eq!(
                journal_carrier(document, "child.finished", node, None, None),
                None
            );
        }
    }

    #[test]
    fn scoped_carriers_walk_the_declared_owner_instead_of_searching_equal_ids() {
        let document = br#"{
          "nodes":[
            {"id":"shared","type":"activity"},
            {"id":"outer","type":"forEach","body":"first","maxItems":2}
          ],
          "bodies":{
            "first":{"nodes":[
              {"id":"shared","type":"script"},
              {"id":"inner","type":"while","body":"second","maxIterations":3}
            ]},
            "second":{"nodes":[{"id":"shared","type":"waitEvent"}]}
          }
        }"#;
        assert_eq!(
            journal_carrier(document, "step.completed", "shared", Some(&[]), Some(&[])),
            Some(CarrierPointer::NodeOutput(NodeId::parse("shared").unwrap()))
        );
        assert_eq!(
            journal_carrier(
                document,
                "step.completed",
                "shared",
                Some(&["outer".into()]),
                Some(&[1]),
            ),
            Some(CarrierPointer::BodyNodeOutput {
                body: NodeId::parse("first").unwrap(),
                node: NodeId::parse("shared").unwrap(),
            })
        );
        assert_eq!(
            journal_carrier(
                document,
                "wait.satisfied",
                "shared",
                Some(&["outer".into(), "inner".into()]),
                Some(&[0, 2]),
            ),
            Some(CarrierPointer::BodyNodeOutput {
                body: NodeId::parse("second").unwrap(),
                node: NodeId::parse("shared").unwrap(),
            })
        );
    }

    #[test]
    fn malformed_paths_event_mismatches_and_database_nodes_fail_closed() {
        let document = br#"{
          "nodes":[
            {"id":"db","type":"database","outputSchema":{}},
            {"id":"wait","type":"waitEvent"},
            {"id":"items","type":"forEach","body":"line","maxItems":1}
          ],
          "bodies":{"line":{"nodes":[{"id":"wait","type":"waitEvent"}]}}
        }"#;
        assert_eq!(
            journal_carrier(document, "step.completed", "db", None, None),
            None
        );
        assert_eq!(
            journal_carrier(document, "step.completed", "wait", None, None),
            None
        );
        assert_eq!(
            journal_carrier(
                document,
                "wait.satisfied",
                "wait",
                Some(&["items".into()]),
                Some(&[]),
            ),
            None
        );
        assert_eq!(
            journal_carrier(
                document,
                "wait.satisfied",
                "wait",
                Some(&["items".into()]),
                Some(&[1]),
            ),
            None
        );
    }
}
