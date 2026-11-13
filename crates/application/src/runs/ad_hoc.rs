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
    /// What went wrong, for a reader that knows no checks: the step's own code, or for an Iceberg probe the code of the first failed check.
    pub failure_code: Option<String>,
    /// Whether the probe is the probe of an Iceberg connection (its step is on the probe activity), which is the only kind with checks.
    pub lake: bool,
    /// What the store kept as the output of an Iceberg probe: the server's own writing of its result. Read by the service and not passed
    /// on; `None` once it has been.
    pub output: Option<Vec<u8>>,
    /// The checks of an Iceberg probe whose result could be read; `None` for any other probe and for one whose result cannot be.
    pub result: Option<sagat_domain::database::ProbeResult>,
}

/// What the store gave, for [`probe_view`].
#[derive(Debug, Clone)]
pub struct ProbeViewInput<'a> {
    pub lake: bool,
    /// Whether the step of the probe succeeded (a failed check is the answer and the step has succeeded).
    pub succeeded: bool,
    /// The code the step failed with, already one of the closed codes.
    pub stored_failure: Option<String>,
    pub output: Option<&'a [u8]>,
    /// The connection the probe was admitted with, which its result must be about.
    pub connection: &'a str,
    pub revision: u32,
}

/// What a reader of the latest probe sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeView {
    pub result: Option<sagat_domain::database::ProbeResult>,
    pub failure_code: Option<String>,
}

/// What a reader of a connection's latest probe sees (D035-150). The probe of an Iceberg connection **succeeds** when a check failed, because a
/// failed check is the answer and not a fault of the platform, and every reader of the probe that existed reads `failureCode` and nothing of
/// any check. So: the step's own code is kept and wins; otherwise the code of the first failed check is the failure code; and a connection is
/// never read as healthy for lack of a result: an Iceberg probe that succeeded and whose result cannot be read (purged, not the probe's, or no
/// codec to read it with) says `DATABASE_RESULT_UNAVAILABLE`. A probe that has not ended says nothing yet. Any other probe is read as it always was.
#[must_use]
pub fn probe_view(
    input: &ProbeViewInput<'_>,
    codec: Option<&dyn crate::database::LakeProbeCodec>,
) -> ProbeView {
    if !input.lake {
        return ProbeView {
            result: None,
            failure_code: input.stored_failure.clone(),
        };
    }
    let result = input
        .output
        .zip(codec)
        .and_then(|(bytes, codec)| {
            codec
                .rebuild(
                    bytes,
                    &crate::database::ExpectedProbe {
                        connection: input.connection,
                        revision: input.revision,
                    },
                )
                .ok()
        })
        .map(|rebuilt| rebuilt.result);
    let failure_code = input
        .stored_failure
        .clone()
        .or_else(|| {
            result
                .as_ref()
                .and_then(sagat_domain::database::ProbeResult::first_failure)
                .map(|code| code.code().to_owned())
        })
        .or_else(|| {
            (input.succeeded && result.is_none()).then(|| {
                sagat_domain::database::DatabaseFailure::ResultUnavailable
                    .code()
                    .to_owned()
            })
        });
    ProbeView {
        result,
        failure_code,
    }
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
    /// The probe of a connection, **chosen by the kind of the connection** (D035-150): the probe of an Iceberg connection is
    /// [`Self::start_lake_probe`], and anything else, a SQL connection or one that is not there, is the fixed `SELECT 1`, which admission
    /// refuses for a connection that is not a SQL one with the answer it has always given. The kind is read **before** the permissions are
    /// looked at, for the tenant of the caller only, and says nothing to the caller: it chooses a document, and the start that follows is
    /// authorised and admitted as any other.
    ///
    /// # Errors
    /// The refusals of any ad hoc start, and for an Iceberg connection also `StateConflict` while a probe of the same connection and
    /// revision is in flight.
    pub async fn probe_connection(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        connection: &ConnectionRef,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        if self
            .store
            .is_lake_connection(
                principal.tenant_id(),
                namespace,
                &connection.name,
                connection.revision,
            )
            .await?
        {
            return self
                .start_lake_probe(principal, namespace, connection, key, ctx)
                .await;
        }
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

    /// The probe of an Iceberg connection (D035-150, ADR-0080): a private run whose one node the platform writes. [`Self::probe_connection`]
    /// calls it for an Iceberg connection, which is how the REST probe reaches it.
    ///
    /// # Errors
    /// `NotFound` for a connection that is not an Iceberg connection of the namespace, whatever the reason (missing, deleted, a SQL one),
    /// `StateConflict` while a probe of the same connection and revision is in flight (a replay of the key that started it is that
    /// run, not a conflict), and the refusals of any ad hoc start: permissions, a connection that cannot be admitted, the lake being off.
    #[tracing::instrument(skip_all, name = "database.lake_probe.start", fields(tenant_id = %principal.tenant_id(), namespace_id = %namespace, run_id = tracing::field::Empty))]
    pub async fn start_lake_probe(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        connection: &ConnectionRef,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        let document = lake_probe_document(connection);
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
        let mut probe = self
            .store
            .latest_connection_probe(principal.tenant_id(), namespace, name)
            .await?;
        if let Some(found) = &mut probe {
            let output = found.output.take();
            let view = probe_view(
                &ProbeViewInput {
                    lake: found.lake,
                    succeeded: found.run.status == sagat_domain::execution::RunStatus::Succeeded,
                    stored_failure: found.failure_code.take(),
                    output: output.as_deref(),
                    connection: name.as_str(),
                    revision: found.revision,
                },
                self.lake_probes.as_deref(),
            );
            found.result = view.result;
            found.failure_code = view.failure_code;
        }
        Ok(probe)
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
        // The platform's own probe documents are the only ones that may carry a private node (ADR-0080): they are parsed by the private
        // entry, whose grammar is the public one plus the private node types. Every other origin is parsed by the public one.
        let parsed = if origin == RunOrigin::ConnectionProbe {
            self.cache.parse_private_document(request.document)
        } else {
            self.cache.parse_document(request.document)
        }
        .map_err(|e| AppError::InvalidDefinition(Box::new(e)))?;
        validate_definition(&parsed.definition, origin)?;
        if origin == RunOrigin::ConnectionProbe {
            let node = parsed.definition.nodes().next().ok_or(AppError::Internal)?;
            let fixed_document = match (node.database_operation(), node.lake_probe_target()) {
                (Some(operation), None) => probe_document(&operation.connection),
                (None, Some(probe)) => lake_probe_document(probe.connection()),
                _ => return Err(AppError::Internal),
            };
            let fixed = self
                .cache
                .parse_private_document(&fixed_document)
                .map_err(|_| AppError::Internal)?;
            if parsed.canonical != fixed.canonical || request.input != b"{}" || timeout_ms != 30_000
            {
                return Err(AppError::NotAllowed("invalid connection probe"));
            }
            // What makes the connection not an Iceberg connection of this namespace (missing, deleted, another tenant's, a SQL one) is
            // refused by admission, which answers all of them alike and records the denial as it does for a SQL probe.
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
            business_id: None,
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
            schedule: None,
            business: None,
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
            reset_sequence: None,
            pending_reset: None,
        };
        let result = match self.store.create(new).await {
            Ok(created) => self
                .finish_start(
                    principal, run_id, created, false, false, None, None, None, ctx,
                )
                .await
                .map(|started| started.run),
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
    // The probe of an Iceberg connection is the platform's own node: only a connection probe is one, and it is one node (ADR-0080).
    let is_probe = |node: &&sagat_domain::workflow::Node| node.lake_probe_target().is_some();
    if !def.triggers().is_empty()
        || nodes.is_empty()
        || nodes.len() > 64
        || (origin != RunOrigin::ConnectionProbe && nodes.iter().any(is_probe))
        || !nodes
            .iter()
            .any(|n| n.database_operation().is_some() || is_probe(n))
        || (origin != RunOrigin::AdHocWorkflow
            && (nodes.len() != 1
                || (nodes[0].database_operation().is_none() && !is_probe(&nodes[0]))))
        || nodes.iter().any(|n| {
            n.database_operation().is_none()
                && !is_probe(n)
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

/// The platform's own definition of the probe of an Iceberg connection (D035-150, ADR-0080): one private node that tests the connection
/// and nothing else. A caller never supplies it, and the start path compares what it is handed with it, as it does for the SQL probe.
/// The attempt's own budget is 25 s inside the run's 30 s, so that a typed answer arrives before the deadline.
///
/// The only interpolation is a validated slug and a nonzero revision; neither can introduce JSON syntax.
#[must_use]
pub fn lake_probe_document(connection: &ConnectionRef) -> Vec<u8> {
    let revision = connection
        .revision
        .map_or_else(String::new, |r| format!(",\"revision\":{r}"));
    format!(r#"{{"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow","metadata":{{"name":"lake-probe"}},"entrypoint":"probe","nodes":[{{"id":"probe","type":"icebergProbe","connectionRef":{{"name":"{}"{revision}}},"timeoutMs":25000}}]}}"#, connection.name.as_str()).into_bytes()
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
