//! Retained managed executions. Customer SQL is only dispatched as normal leased work.
use super::*;
use crate::authz::require_namespace;
use sagat_domain::{
    database::ConnectionRef, identity::Slug, quota::QuotaResource, workflow::Digest,
};

pub const DEFAULT_AD_HOC_TIMEOUT_MS: u32 = 300_000;
pub const MAX_AD_HOC_TIMEOUT_MS: u32 = 3_600_000;

/// Provenance is set by the admission use case, never by an IR document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOrigin {
    AdHocStep,
    AdHocWorkflow,
    ConnectionProbe,
}
impl RunOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AdHocStep => "AD_HOC_STEP",
            Self::AdHocWorkflow => "AD_HOC_WORKFLOW",
            Self::ConnectionProbe => "CONNECTION_PROBE",
        }
    }
}
impl std::str::FromStr for RunOrigin {
    type Err = ();
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "AD_HOC_STEP" => Ok(Self::AdHocStep),
            "AD_HOC_WORKFLOW" => Ok(Self::AdHocWorkflow),
            "CONNECTION_PROBE" => Ok(Self::ConnectionProbe),
            _ => Err(()),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdHocMode {
    Step,
    Workflow,
}
impl AdHocMode {
    const fn origin(self) -> RunOrigin {
        match self {
            Self::Step => RunOrigin::AdHocStep,
            Self::Workflow => RunOrigin::AdHocWorkflow,
        }
    }
}
#[derive(Clone)]
pub struct StartDatabaseExecution<'a> {
    pub namespace: NamespaceId,
    pub mode: AdHocMode,
    pub document: &'a [u8],
    pub input: &'a [u8],
    pub timeout_ms: Option<u32>,
    pub key: &'a str,
}
impl std::fmt::Debug for StartDatabaseExecution<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartDatabaseExecution")
            .field("namespace", &self.namespace)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}
/// Only validated application admission can construct this private definition request.
#[derive(Clone)]
pub struct RetainedDefinition {
    canonical: Vec<u8>,
    digest: Digest,
    origin: RunOrigin,
    workflow_limit: u64,
}
impl std::fmt::Debug for RetainedDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetainedDefinition")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}
impl RetainedDefinition {
    pub fn document(&self) -> &[u8] {
        &self.canonical
    }
    pub const fn digest(&self) -> Digest {
        self.digest
    }
    pub const fn origin(&self) -> RunOrigin {
        self.origin
    }
    pub const fn workflow_limit(&self) -> u64 {
        self.workflow_limit
    }
}

/// Safe connection health evidence; the result cells remain behind their separate permission.
#[derive(Debug, Clone)]
pub struct ConnectionProbe {
    pub run: RunRecord,
    pub revision: u32,
    pub failure_code: Option<String>,
}

impl<S: RunStore + DefinitionStore, C: Clock, E: ExpressionEvaluator> RunService<S, C, E> {
    #[tracing::instrument(skip_all, name = "database.ad_hoc.start", fields(tenant_id = %principal.tenant_id(), namespace_id = %request.namespace, run_id = tracing::field::Empty))]
    pub async fn start_database(
        &self,
        principal: &Principal,
        request: &StartDatabaseExecution<'_>,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        self.start_private(principal, request, request.mode.origin(), None, ctx)
            .await
    }

    #[tracing::instrument(skip_all, name = "database.probe.start", fields(tenant_id = %principal.tenant_id(), namespace_id = %namespace, run_id = tracing::field::Empty))]
    pub async fn probe_connection(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        connection: &ConnectionRef,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        let document = probe_document(connection);
        self.start_private(
            principal,
            &StartDatabaseExecution {
                namespace,
                mode: AdHocMode::Step,
                document: &document,
                input: b"{}",
                timeout_ms: Some(30_000),
                key,
            },
            RunOrigin::ConnectionProbe,
            None,
            ctx,
        )
        .await
    }

    pub async fn latest_connection_probe(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &Slug,
        ctx: &RequestContext,
    ) -> Result<Option<ConnectionProbe>, AppError> {
        principal.authorize(Permission::NamespaceRead)?;
        principal.authorize(Permission::RunRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        self.store
            .latest_connection_probe(principal.tenant_id(), namespace, name)
            .await
            .map_err(Into::into)
    }

    pub(super) async fn rerun_private(
        &self,
        principal: &Principal,
        original: &RunRecord,
        source: &RerunSource,
        rerun: Rerun<'_>,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        if original.info.started_by != principal.subject().as_str() {
            return Err(AppError::NotFound);
        }
        let origin = original.info.origin.ok_or(AppError::Internal)?;
        let document = self
            .store
            .private_definition(
                principal.tenant_id(),
                principal.subject().as_str(),
                original.info.run_id,
            )
            .await?
            .ok_or(AppError::NotFound)?;
        let timeout_ms = if origin == RunOrigin::ConnectionProbe {
            Some(30_000)
        } else {
            None
        };
        self.start_private(
            principal,
            &StartDatabaseExecution {
                namespace: original.info.namespace_id,
                mode: if origin == RunOrigin::AdHocWorkflow {
                    AdHocMode::Workflow
                } else {
                    AdHocMode::Step
                },
                document: &document,
                input: &source.input,
                timeout_ms,
                key,
            },
            origin,
            Some(rerun),
            ctx,
        )
        .await
    }

    async fn start_private(
        &self,
        principal: &Principal,
        request: &StartDatabaseExecution<'_>,
        origin: RunOrigin,
        rerun: Option<Rerun<'_>>,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        for permission in [
            Permission::RunStart,
            Permission::DatabaseAdHoc,
            Permission::DatabaseUse,
        ] {
            if let Err(error) = principal.authorize(permission) {
                audit_denied(&self.store, &self.clock, principal, request.namespace, ctx).await;
                return Err(error.into());
            }
        }
        require_namespace(&self.store, &self.clock, principal, request.namespace, ctx).await?;
        let timeout_ms = request.timeout_ms.unwrap_or(DEFAULT_AD_HOC_TIMEOUT_MS);
        if !(1..=MAX_AD_HOC_TIMEOUT_MS).contains(&timeout_ms)
            || request.input.len() > MAX_PAYLOAD_BYTES
            || !self.engine.evaluator().is_json_object(request.input)
        {
            return Err(AppError::NotAllowed(
                "invalid database execution input or timeout",
            ));
        }
        if !(8..=160).contains(&request.key.len())
            || !request.key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(AppError::NotAllowed("invalid Idempotency-Key"));
        }
        let parsed = self
            .cache
            .parse_document(request.document)
            .map_err(|e| AppError::InvalidDefinition(Box::new(e)))?;
        validate_definition(&parsed.definition, origin)?;
        if origin == RunOrigin::ConnectionProbe {
            let reference = &parsed
                .definition
                .nodes()
                .next()
                .and_then(|n| n.database_operation())
                .ok_or(AppError::Internal)?
                .connection;
            let fixed = self
                .cache
                .parse_document(&probe_document(reference))
                .map_err(|_| AppError::Internal)?;
            if parsed.canonical != fixed.canonical || request.input != b"{}" || timeout_ms != 30_000
            {
                return Err(AppError::NotAllowed("invalid connection probe"));
            }
        }
        if parsed
            .definition
            .nodes()
            .filter_map(|n| n.database_operation())
            .any(|op| op.kind == sagat_domain::database::OperationKind::Execute)
            && let Err(error) = principal.authorize(Permission::DatabaseWrite)
        {
            audit_denied(&self.store, &self.clock, principal, request.namespace, ctx).await;
            return Err(error.into());
        }
        let idempotency = Idempotency {
            scope: format!("database.execution:{}", request.namespace),
            key: request.key.to_owned(),
            fingerprint: fingerprint(&[
                principal.subject().as_str().as_bytes(),
                request.namespace.to_string().as_bytes(),
                origin.as_str().as_bytes(),
                &parsed.canonical,
                request.input,
                &timeout_ms.to_be_bytes(),
                rerun
                    .map(|r| r.of.to_string())
                    .unwrap_or_default()
                    .as_bytes(),
                rerun.map_or("", |r| r.reason).as_bytes(),
            ]),
        };
        let now = self.clock.now();
        let run_id = RunId::generate();
        let info = RunInfo {
            run_id,
            tenant_id: principal.tenant_id(),
            namespace_id: request.namespace,
            workflow_id: WorkflowId::generate(),
            version_id: WorkflowVersionId::generate(),
            started_by: principal.subject().as_str().to_owned(),
            correlation_id: None,
            trace_id: ctx.trace_id.clone(),
            parent_span: ctx.span_id.clone(),
            rerun_of: rerun.map(|r| r.of),
            origin: Some(origin),
            deadline_at: Some(now + time::Duration::milliseconds(i64::from(timeout_ms))),
            created_at: now,
            parent: None,
            depth: 0,
            root: run_id,
        };
        let audit = AuditRecord::by(
            principal,
            if rerun.is_some() {
                AuditAction::RunRerun
            } else {
                AuditAction::RunStarted
            },
            ResourceType::Run,
            run_id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(rerun.map_or_else(
            || origin.as_str().to_owned(),
            |r| {
                format!(
                    "{}; started again from {}: {}",
                    origin.as_str(),
                    r.of,
                    r.reason
                )
            },
        ));
        let workflow_limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(request.namespace),
                QuotaResource::Workflows,
            )
            .await?;
        let new = NewRun {
            limits: RunLimits::of(&self.store, principal.tenant_id(), request.namespace).await?,
            info,
            definition: Arc::new(parsed.definition),
            input: request.input.to_vec(),
            input_blob: None,
            audit,
            idempotency,
            retained: Some(RetainedDefinition {
                canonical: parsed.canonical,
                digest: parsed.digest,
                origin,
                workflow_limit,
            }),
        };
        let result = match self.store.create(new).await {
            Ok(created) => self.finish_start(principal, run_id, created, ctx).await,
            Err(error) => {
                if matches!(
                    error,
                    StoreError::DatabaseUnavailable | StoreError::DatabaseLimit
                ) {
                    audit_denied(&self.store, &self.clock, principal, request.namespace, ctx).await;
                }
                Err(error.into())
            }
        };
        if let Ok(run) = &result {
            tracing::Span::current().record("run_id", run.info.run_id.to_string());
        }
        metrics::counter!("database_executions_total", "origin" => origin.as_str(), "outcome" => if result.is_ok() { "admitted" } else { "refused" }).increment(1);
        result
    }
}

fn validate_definition(def: &WorkflowDefinition, origin: RunOrigin) -> Result<(), AppError> {
    let nodes: Vec<_> = def.nodes().collect();
    if !def.triggers().is_empty()
        || nodes.is_empty()
        || nodes.len() > 64
        || !nodes.iter().any(|n| n.database_operation().is_some())
        || (origin != RunOrigin::AdHocWorkflow
            && (nodes.len() != 1 || nodes[0].database_operation().is_none()))
        || nodes.iter().any(|n| {
            n.database_operation().is_none()
                && !matches!(
                    n.kind(),
                    NodeKind::Timer { .. }
                        | NodeKind::Condition { .. }
                        | NodeKind::Parallel { .. }
                        | NodeKind::End { .. }
                )
        })
    {
        return Err(AppError::NotAllowed("unsupported ad hoc database workflow"));
    }
    Ok(())
}

// The only interpolation is a validated Slug and a nonzero numeric revision; neither can introduce
// JSON syntax. Keeping the fixed probe here prevents callers from supplying its SQL or provenance.
fn probe_document(connection: &ConnectionRef) -> Vec<u8> {
    let revision = connection
        .revision
        .map_or_else(String::new, |r| format!(",\"revision\":{r}"));
    format!(r#"{{"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow","metadata":{{"name":"database-probe"}},"entrypoint":"probe","nodes":[{{"id":"probe","type":"database","connectionRef":{{"name":"{}"{revision}}},"operation":"query","statement":"SELECT 1::bigint","limits":{{"maxRows":1,"maxResultBytes":4096,"timeoutMs":5000,"connectTimeoutMs":5000}},"timeoutMs":10000}}]}}"#, connection.name.as_str()).into_bytes()
}

/// A bounded namespace denial record, without definition/input/connection/provider content.
pub(crate) async fn audit_denied<D: crate::authz::NamespaceDirectory, C: Clock>(
    store: &D,
    clock: &C,
    principal: &Principal,
    namespace: NamespaceId,
    ctx: &RequestContext,
) {
    metrics::counter!("database_execution_denials_total").increment(1);
    let audit = AuditRecord::by(
        principal,
        AuditAction::DatabaseDenied,
        ResourceType::Namespace,
        namespace.to_string(),
        ctx.trace_id.clone(),
        clock.now(),
    )
    .with_reason("managed database execution refused");
    if let Err(error) = store.record_denial(&audit).await {
        tracing::error!(%error, "failed to record database denial");
    }
}
