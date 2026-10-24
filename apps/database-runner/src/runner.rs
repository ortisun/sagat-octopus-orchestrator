//! Fair bounded polling with one session per namespace and finite graceful shutdown.
use crate::{RunnerError, attempt, config::Config, rpc::Rpc};
use sagat_api_grpc::pb;
use sagat_domain::{database::MANAGED_DATABASE_ACTIVITY, identity::NamespaceId};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;
use zeroize::Zeroizing;

pub struct Runner {
    config: Config,
    path: Arc<Path>,
    rpc: Rpc,
}
struct Session {
    id: String,
    token: Zeroizing<String>,
}
impl Runner {
    pub async fn start(path: PathBuf) -> Result<Self, RunnerError> {
        let config = Config::read(&path).await?;
        // Stable pod identity avoids replicas invalidating each other's worker sessions.
        let config = match std::env::var("SAGAT_DATABASE_RUNNER_NAME") {
            Ok(name) => config.with_name(name)?,
            Err(std::env::VarError::NotPresent) => config,
            Err(_) => return Err(RunnerError::Configuration),
        };
        let rpc = Rpc::connect(&config).await?;
        Ok(Self {
            config,
            path: path.into(),
            rpc,
        })
    }
    pub async fn serve(self, stop: CancellationToken) {
        let mut sessions: Vec<Option<Session>> =
            (0..self.config.namespaces.len()).map(|_| None).collect();
        let active: Vec<_> = (0..sessions.len())
            .map(|_| Arc::new(AtomicUsize::new(0)))
            .collect();
        let mut tasks = tokio::task::JoinSet::<()>::new();
        let cancel = CancellationToken::new();
        let slots = Arc::new(crate::slots::Slots::default());
        let mut next = 0;
        'serving: loop {
            let start = next;
            for offset in 0..sessions.len() {
                let index = (start + offset) % sessions.len();
                let namespace = &self.config.namespaces[index];
                while let Some(result) = tasks.try_join_next() {
                    if result.is_err() {
                        metrics::counter!("database_runner_task_panics_total").increment(1);
                    }
                }
                if stop.is_cancelled() {
                    break 'serving;
                }
                if tasks.len() >= usize::from(self.config.max_concurrent) {
                    break;
                }
                next = (index + 1) % sessions.len();
                if sessions[index].is_none() {
                    if active[index].load(Ordering::Relaxed) > 0 {
                        continue;
                    }
                    match self.register(index).await {
                        Ok(session) => sessions[index] = Some(session),
                        Err(_) => {
                            metrics::counter!("database_runner_registration_total","outcome"=>"refused").increment(1);
                            continue;
                        }
                    }
                }
                let Some(session) = &sessions[index] else {
                    continue;
                };
                let room = (usize::from(self.config.max_concurrent) - tasks.len()).min(4) as i32;
                let poll = async {
                    let request = self
                        .rpc
                        .request(pb::PollTasksRequest {
                            worker_id: session.id.clone(),
                            session_token: session.token.to_string(),
                            max_tasks: room,
                        })
                        .await?;
                    self.rpc
                        .client
                        .clone()
                        .poll_tasks(request)
                        .await
                        .map(tonic::Response::into_inner)
                        .map_err(crate::rpc::Failure::from)
                };
                let response =
                    tokio::select! { ()=stop.cancelled()=>break 'serving, result=poll=>result };
                let response = match response {
                    Ok(response) => response,
                    Err(error) => {
                        if matches!(
                            error.code,
                            tonic::Code::Unauthenticated
                                | tonic::Code::NotFound
                                | tonic::Code::PermissionDenied
                        ) {
                            sessions[index] = None;
                        }
                        metrics::counter!("database_runner_poll_failures_total").increment(1);
                        continue;
                    }
                };
                for lease in response.tasks.into_iter().take(room as usize) {
                    let namespace_id = match namespace.id.parse::<NamespaceId>() {
                        Ok(id) => id,
                        Err(_) => continue,
                    };
                    let slug = namespace.slug.clone();
                    let path = self.path.clone();
                    let rpc = self.rpc.clone();
                    let worker_id = session.id.clone();
                    let development = self.config.development_loopback;
                    let cancelled = cancel.child_token();
                    let slots = slots.clone();
                    let trace =
                        sagat_telemetry::TraceContext::from_traceparent(Some(&lease.traceparent));
                    // Only validated identifiers belong in spans, never arbitrary lease content.
                    let run_id = lease
                        .run_id
                        .parse::<uuid::Uuid>()
                        .map(|id| id.to_string())
                        .unwrap_or_default();
                    let span = tracing::info_span!("database.runner.attempt",
                        tenant_id = tracing::field::Empty,
                        trace_id = %trace.trace_id(),
                        parent_span_id = %trace.parent_span_id().map(|id|id.to_string()).unwrap_or_default(),
                        namespace_id = %namespace_id, run_id = %run_id, attempt = lease.attempt);
                    active[index].fetch_add(1, Ordering::Relaxed);
                    let active = Active(active[index].clone());
                    let task = attempt::Task {
                        lease,
                        namespace: namespace_id,
                        slug,
                        worker_id,
                    };
                    tasks.spawn(async move {
                        let _active = active;
                        let started = Instant::now();
                        let result=attempt::run(rpc,path,task,development,cancelled,slots).await;
                        metrics::histogram!("database_runner_attempt_seconds").record(started.elapsed().as_secs_f64());
                        metrics::counter!("database_runner_attempts_total","outcome"=>if result.is_ok() { "reported" } else { "unacknowledged" }).increment(1);
                    }.instrument(span));
                }
            }
            tokio::select! {
                ()=stop.cancelled()=>break,
                Some(result)=tasks.join_next(), if !tasks.is_empty()=> { if result.is_err() { metrics::counter!("database_runner_task_panics_total").increment(1); } },
                ()=tokio::time::sleep(Duration::from_millis(u64::from(self.config.poll_interval_ms)))=>{},
            }
        }
        cancel.cancel();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !tasks.is_empty() {
            if tokio::time::timeout_at(deadline, tasks.join_next())
                .await
                .is_err()
            {
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                break;
            }
        }
    }
    async fn register(&self, index: usize) -> Result<Session, RunnerError> {
        let namespace = &self.config.namespaces[index];
        let request = self
            .rpc
            .request(pb::RegisterWorkerRequest {
                worker_name: format!("{}/{}", self.config.name, namespace.slug),
                namespace: namespace.slug.clone(),
                capabilities: Some(pb::WorkerCapabilities {
                    activities: vec![MANAGED_DATABASE_ACTIVITY.into()],
                    languages: vec![],
                    pools: vec![],
                    max_inflight: i32::from(self.config.max_concurrent),
                    protocol_version: "1.6".into(),
                }),
            })
            .await
            .map_err(|_| RunnerError::Credentials)?;
        let response = self
            .rpc
            .client
            .clone()
            .register_worker(request)
            .await
            .map_err(|_| RunnerError::Transport)?
            .into_inner();
        if response.accepted_protocol_version != "1.6"
            || response.worker_id.parse::<uuid::Uuid>().is_err()
            || response.session_token.is_empty()
            || response.session_token.len() > 256
        {
            return Err(RunnerError::Capability);
        }
        Ok(Session {
            id: response.worker_id,
            token: Zeroizing::new(response.session_token),
        })
    }
}

struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
