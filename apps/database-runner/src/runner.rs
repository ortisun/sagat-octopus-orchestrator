//! Fair bounded polling with one session per namespace and finite graceful shutdown.
use crate::{
    RunnerError, attempt,
    config::Config,
    lake::shell::{Launch, Production},
    rpc::Rpc,
};
use sagat_api_grpc::pb;
use sagat_domain::{
    database::{ICEBERG_PROBE_ACTIVITY, ICEBERG_WRITE_ACTIVITY, MANAGED_DATABASE_ACTIVITY},
    identity::NamespaceId,
};
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
    /// Present when the configuration has a `lake` section: the runner is then a lake runner and offers no SQL.
    lake: Option<Arc<dyn Launch>>,
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
        // A lake section is read whole, once, and decides what the runner is: it is not left off when it is wrong.
        let lake: Option<Arc<dyn Launch>> = match &config.lake {
            Some(section) => {
                let settings = section
                    .validated(config.development_loopback)
                    .map_err(|_| RunnerError::Configuration)?;
                Some(Arc::new(Production::new(
                    settings,
                    config.development_loopback,
                )?))
            }
            None => None,
        };
        Self::connect(path, config, lake).await
    }
    async fn connect(
        path: PathBuf,
        config: Config,
        lake: Option<Arc<dyn Launch>>,
    ) -> Result<Self, RunnerError> {
        let rpc = Rpc::connect(&config).await?;
        Ok(Self {
            config,
            path: path.into(),
            rpc,
            lake,
        })
    }
    /// A runner whose lake runtime is `lake`, for the tests of the shell.
    #[cfg(test)]
    pub(crate) async fn start_with(
        path: PathBuf,
        lake: Arc<dyn Launch>,
    ) -> Result<Self, RunnerError> {
        let config = Config::read(&path).await?;
        Self::connect(path, config, Some(lake)).await
    }
    pub async fn serve(self, stop: CancellationToken) {
        match self.lake.clone() {
            Some(lake) => self.serve_lake(stop, lake).await,
            None => self.serve_sql(stop).await,
        }
    }
    async fn serve_sql(self, stop: CancellationToken) {
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
                    match self
                        .register(
                            index,
                            MANAGED_DATABASE_ACTIVITY,
                            i32::from(self.config.max_concurrent),
                        )
                        .await
                    {
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
    /// One session per namespace, for the one activity of the profile and as many attempts at once as it can hold.
    async fn register(
        &self,
        index: usize,
        activity: &str,
        max_inflight: i32,
    ) -> Result<Session, RunnerError> {
        self.register_activities(index, &[activity], max_inflight)
            .await
            .map_err(|refused| refused.error)
    }

    /// One session per namespace for these activities, and the status the service refused with when it did.
    async fn register_activities(
        &self,
        index: usize,
        activities: &[&str],
        max_inflight: i32,
    ) -> Result<Session, Refused> {
        let namespace = &self.config.namespaces[index];
        let request = self
            .rpc
            .request(pb::RegisterWorkerRequest {
                worker_name: format!("{}/{}", self.config.name, namespace.slug),
                namespace: namespace.slug.clone(),
                capabilities: Some(pb::WorkerCapabilities {
                    activities: activities
                        .iter()
                        .map(|activity| (*activity).into())
                        .collect(),
                    languages: vec![],
                    pools: vec![],
                    max_inflight,
                    protocol_version: "1.6".into(),
                }),
            })
            .await
            .map_err(|_| Refused::of(RunnerError::Credentials))?;
        let response = self
            .rpc
            .client
            .clone()
            .register_worker(request)
            .await
            .map_err(|status| Refused {
                error: RunnerError::Transport,
                status: Some(status.code()),
            })?
            .into_inner();
        if response.accepted_protocol_version != "1.6"
            || response.worker_id.parse::<uuid::Uuid>().is_err()
            || response.session_token.is_empty()
            || response.session_token.len() > 256
        {
            return Err(Refused::of(RunnerError::Capability));
        }
        Ok(Session {
            id: response.worker_id,
            token: Zeroizing::new(response.session_token),
        })
    }

    /// The session of the lake profile: its two activities, the writes and the probe of a connection (D035-150), in **one**
    /// session. A service that refuses with `PERMISSION_DENIED` (`NOT_BOUND`) is saying that the worker's binding does not cover
    /// an activity it offered, and the binding may be older than the runner (the order of a rollout is the server, the
    /// bindings, then the runners): the writes are what the runner was for and must not stop for it, so it offers them alone.
    /// Any other refusal is not a binding, and is not a reason to offer less.
    async fn register_lake(&self, index: usize, max_inflight: i32) -> Result<Session, RunnerError> {
        let both = [ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY];
        match self.register_activities(index, &both, max_inflight).await {
            Ok(session) => {
                metrics::counter!("database_runner_lake_probe_offer_total","outcome"=>"offered")
                    .increment(1);
                // a gauge too: a counter that went up once, at registration, cannot say for days that a runner still serves
                // the writes alone, and it is a state that stays until the runner registers again
                metrics::gauge!("database_runner_lake_probe_offered").set(1.0);
                Ok(session)
            }
            Err(Refused {
                status: Some(tonic::Code::PermissionDenied),
                ..
            }) => {
                // seen whether or not the fallback works: a binding that covers neither is refused twice, and the second refusal
                // says only that a registration was refused
                metrics::counter!("database_runner_lake_probe_offer_total","outcome"=>"not_bound")
                    .increment(1);
                let session = self
                    .register_activities(index, &[ICEBERG_WRITE_ACTIVITY], max_inflight)
                    .await
                    .map_err(|refused| refused.error)?;
                // the runner serves the writes and not the probes: that is worth seeing, because it is silent otherwise, and
                // it stays so until the runner registers again (a widened binding does not close a session that is in force)
                metrics::counter!("database_runner_lake_probe_offer_total","outcome"=>"write_only")
                    .increment(1);
                metrics::gauge!("database_runner_lake_probe_offered").set(0.0);
                Ok(session)
            }
            Err(refused) => Err(refused.error),
        }
    }
}

/// Why a registration did not make a session: what the runner makes of it, and the status of the service when it said no.
struct Refused {
    error: RunnerError,
    status: Option<tonic::Code>,
}

impl Refused {
    const fn of(error: RunnerError) -> Self {
        Self {
            error,
            status: None,
        }
    }
}

impl Runner {
    /// The lake profile's loop (feature 035, D035-143). It is the SQL loop with three differences. **How much it asks for** is what
    /// it can hold: the free slots, not more attempts than `maxConcurrent`, and no poll at all when there is none. **What it
    /// offers** is the lake activity and nothing else. **How it stops** is not the SQL way: it stops polling and lets the attempts
    /// that are running finish, for a grace period; after that their leases are given up as lost, never cancelled, because a write
    /// that was under way and is cancelled is reported as unknown, and a rollout would make every one of them so.
    async fn serve_lake(self, stop: CancellationToken, lake: Arc<dyn Launch>) {
        let capacity = usize::from(self.config.max_concurrent).min(lake.slots());
        let mut sessions: Vec<Option<Session>> =
            (0..self.config.namespaces.len()).map(|_| None).collect();
        let active: Vec<_> = (0..sessions.len())
            .map(|_| Arc::new(AtomicUsize::new(0)))
            .collect();
        let mut tasks = tokio::task::JoinSet::<()>::new();
        let drain = CancellationToken::new();
        // The attempt directories no attempt owns any more (a container that outlived its runner, a directory that could not be
        // removed) are swept for as long as the runner serves.
        let sweeping = lake.work_root().map(|root| {
            tokio::spawn(crate::lake::sweep::keep_sweeping(
                root,
                lake.sweep_every(),
                crate::lake::sweep::SWEEP_AFTER,
                stop.clone(),
                |pass| match pass {
                    Ok(swept) => {
                        metrics::counter!("database_runner_lake_directories_total","outcome"=>"removed")
                            .increment(swept.removed as u64);
                        metrics::counter!("database_runner_lake_directories_total","outcome"=>"failed")
                            .increment(swept.failed as u64);
                    }
                    Err(_) => {
                        metrics::counter!("database_runner_lake_sweeps_total","outcome"=>"unreadable")
                            .increment(1);
                    }
                },
            ))
        });
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
                // the slots nobody holds, counting the ones held back, and not more attempts than the runner is meant to hold
                let room = lake.free().min(capacity.saturating_sub(tasks.len())).min(4);
                if room == 0 {
                    break;
                }
                next = (index + 1) % sessions.len();
                if sessions[index].is_none() {
                    if active[index].load(Ordering::Relaxed) > 0 {
                        continue;
                    }
                    let max_inflight = i32::try_from(capacity).unwrap_or(i32::MAX);
                    match self.register_lake(index, max_inflight).await {
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
                let poll = async {
                    let request = self
                        .rpc
                        .request(pb::PollTasksRequest {
                            worker_id: session.id.clone(),
                            session_token: session.token.to_string(),
                            max_tasks: i32::try_from(room).unwrap_or(4),
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
                for lease in response.tasks.into_iter().take(room) {
                    let Ok(namespace_id) = namespace.id.parse::<NamespaceId>() else {
                        continue;
                    };
                    let trace =
                        sagat_telemetry::TraceContext::from_traceparent(Some(&lease.traceparent));
                    // Only validated identifiers belong in spans, never arbitrary lease content.
                    let run_id = lease
                        .run_id
                        .parse::<uuid::Uuid>()
                        .map(|id| id.to_string())
                        .unwrap_or_default();
                    let span = tracing::info_span!("lake.runner.attempt",
                        tenant_id = tracing::field::Empty,
                        trace_id = %trace.trace_id(),
                        parent_span_id = %trace.parent_span_id().map(|id|id.to_string()).unwrap_or_default(),
                        namespace_id = %namespace_id, run_id = %run_id, attempt = lease.attempt);
                    let task = crate::lake::attempt::Task {
                        lease,
                        namespace: namespace_id,
                        worker_id: session.id.clone(),
                    };
                    active[index].fetch_add(1, Ordering::Relaxed);
                    let held = Active(active[index].clone());
                    let launched = lake.launch(self.rpc.clone(), task, drain.clone());
                    tasks.spawn(
                        async move {
                            let _held = held;
                            let started = Instant::now();
                            let result = launched.await;
                            metrics::histogram!("database_runner_attempt_seconds")
                                .record(started.elapsed().as_secs_f64());
                            metrics::counter!("database_runner_attempts_total","outcome"=>match result { Ok(crate::lake::attempt::Disposition::Reported) => "reported", Ok(crate::lake::attempt::Disposition::Abandoned) => "abandoned", Err(_) => "unacknowledged" }).increment(1);
                        }
                        .instrument(span),
                    );
                }
            }
            tokio::select! {
                ()=stop.cancelled()=>break,
                Some(result)=tasks.join_next(), if !tasks.is_empty()=> { if result.is_err() { metrics::counter!("database_runner_task_panics_total").increment(1); } },
                ()=tokio::time::sleep(Duration::from_millis(u64::from(self.config.poll_interval_ms)))=>{},
            }
        }
        // Stopped: no more polls. What is running gets its grace; then the leases are given up as lost, and what is left is aborted.
        let deadline = Instant::now() + lake.drain_grace();
        while !tasks.is_empty() {
            if tokio::time::timeout_at(deadline, tasks.join_next())
                .await
                .is_err()
            {
                break;
            }
        }
        if !tasks.is_empty() {
            drain.cancel();
            let settled = Instant::now() + SETTLE;
            while !tasks.is_empty() {
                if tokio::time::timeout_at(settled, tasks.join_next())
                    .await
                    .is_err()
                {
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    break;
                }
            }
        }
        // it stops with `stop`, which has been cancelled: this waits for the pass that may be under way
        if let Some(sweeping) = sweeping {
            let _ = sweeping.await;
        }
    }
}

/// After the leases are given up, how long the attempts have to kill their containers and let go.
const SETTLE: Duration = Duration::from_secs(30);

struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Fake, NAMESPACE_SLUG, home, serve_fake};
    use std::time::Duration;

    /// The runner ends, within a bound: a runner that cannot be stopped fails its test and does not hang the suite.
    async fn ended(serving: tokio::task::JoinHandle<()>) {
        tokio::time::timeout(Duration::from_secs(20), serving)
            .await
            .expect("the runner ends when it is stopped")
            .unwrap();
    }

    /// The SQL shell as it is, pinned before the lake shell is built beside it (D035-143): what it offers when it registers, the
    /// session it polls with, and how much it asks for.
    #[tokio::test]
    async fn the_sql_runner_registers_for_the_sql_activity_and_polls_with_its_session() {
        let fake = Fake::default();
        let server = serve_fake(fake.clone()).await;
        let home = home(&server, None);
        let runner = Runner::start(home.config.clone()).await.unwrap();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(runner.serve(stop.clone()));
        fake.until("a poll", |script| !script.polled.is_empty())
            .await;
        stop.cancel();
        ended(serving).await;
        fake.with(|script| {
            let registration = &script.registered[0];
            assert_eq!(registration.worker_name, format!("runner/{NAMESPACE_SLUG}"));
            assert_eq!(registration.namespace, NAMESPACE_SLUG);
            let capabilities = registration.capabilities.as_ref().unwrap();
            assert_eq!(capabilities.activities, [MANAGED_DATABASE_ACTIVITY]);
            assert_eq!(capabilities.max_inflight, 4);
            assert_eq!(capabilities.protocol_version, "1.6");
            let poll = &script.polled[0];
            assert_eq!(poll.worker_id, uuid::Uuid::from_u128(7).to_string());
            assert_eq!(poll.session_token, "session-token");
            assert_eq!(poll.max_tasks, 4);
        });
    }

    // -- the lake profile (feature 035, D035-143) ----------------------------------------------------------------------

    use crate::lake::{
        doubles::{Script, TestLaunch, answer, receipt_line, world},
        fixtures::{input_json, resolved_database},
    };
    use crate::testing::{NAMESPACE_ID, epoch_ms_in};
    use sagat_domain::database::{ICEBERG_PROBE_ACTIVITY, ICEBERG_WRITE_ACTIVITY};

    fn lake_lease(task: &str) -> pb::TaskLease {
        pb::TaskLease {
            task_id: task.into(),
            run_id: uuid::Uuid::from_u128(9).to_string(),
            step_id: uuid::Uuid::from_u128(10).to_string(),
            attempt: 1,
            generation: 3,
            lease_token: "lease-token".into(),
            expires_at_epoch_ms: epoch_ms_in(Duration::from_secs(60)),
            activity_name: ICEBERG_WRITE_ACTIVITY.into(),
            input_json: String::from_utf8(input_json(|_| {})).unwrap(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_lake_runner_offers_both_lake_activities_in_one_session_and_no_more_attempts_than_it_has_slots()
     {
        let fake = Fake::default();
        let server = serve_fake(fake.clone()).await;
        let home = home(&server, None);
        let world = world();
        // two slots, and a configuration that would allow four
        let runner = Runner::start_with(
            home.config.clone(),
            TestLaunch::new(&world, Script::NeverConnects, 2),
        )
        .await
        .unwrap();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(runner.serve(stop.clone()));
        fake.until("a poll", |script| !script.polled.is_empty())
            .await;
        stop.cancel();
        ended(serving).await;
        fake.with(|script| {
            assert_eq!(
                script.registered.len(),
                1,
                "one session for both activities, not one each"
            );
            let capabilities = script.registered[0].capabilities.as_ref().unwrap();
            assert_eq!(
                capabilities.activities,
                [ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY]
            );
            assert_eq!(capabilities.max_inflight, 2, "the slots, not maxConcurrent");
            assert_eq!(capabilities.protocol_version, "1.6");
            assert_eq!(script.polled[0].max_tasks, 2);
        });
    }

    /// A lake runner whose service refuses the first registrations with `statuses`, until it has polled: what it offered each
    /// time, and the worker the first poll was made as (each registration the service accepts is the next worker, from 7).
    async fn lake_registrations_refused_with(
        statuses: Vec<tonic::Status>,
    ) -> (Vec<Vec<String>>, String) {
        let fake = Fake::default();
        fake.with(|script| script.register_errors = statuses.into());
        let server = serve_fake(fake.clone()).await;
        let home = home(&server, None);
        let world = world();
        let runner = Runner::start_with(
            home.config.clone(),
            TestLaunch::new(&world, Script::NeverConnects, 2),
        )
        .await
        .unwrap();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(runner.serve(stop.clone()));
        fake.until("a poll", |script| !script.polled.is_empty())
            .await;
        stop.cancel();
        ended(serving).await;
        fake.with(|script| {
            let offered = script
                .registered
                .iter()
                .map(|registration| {
                    registration
                        .capabilities
                        .as_ref()
                        .unwrap()
                        .activities
                        .clone()
                })
                .collect();
            (offered, script.polled[0].worker_id.clone())
        })
    }

    fn worker(number: u128) -> String {
        uuid::Uuid::from_u128(number).to_string()
    }

    #[tokio::test]
    async fn a_binding_that_does_not_cover_the_probe_leaves_a_runner_that_serves_the_writes() {
        // the server refuses an activity the worker is not bound to with `PERMISSION_DENIED` (`NOT_BOUND`): the binding of the
        // runner may be older than the runner (the rollout order is the server, the binding, then the runners), and the
        // writes must not stop for it
        let (offered, polled_as) =
            lake_registrations_refused_with(vec![tonic::Status::permission_denied("NOT_BOUND")])
                .await;

        assert_eq!(
            offered,
            [
                vec![ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY],
                vec![ICEBERG_WRITE_ACTIVITY],
            ]
        );
        // the session that polls is the one of the registration that was accepted: the second, and not the refused first
        assert_eq!(polled_as, worker(8));
    }

    #[tokio::test]
    async fn a_service_that_cannot_answer_is_not_a_reason_to_stop_offering_the_probe() {
        // an outage is not a binding: the same offer is made again, and the runner does not fall back to the writes alone
        let (offered, polled_as) =
            lake_registrations_refused_with(vec![tonic::Status::unavailable("down")]).await;

        assert_eq!(
            offered,
            [
                vec![ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY],
                vec![ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY],
            ]
        );
        assert_eq!(polled_as, worker(8));
    }

    #[tokio::test]
    async fn a_binding_that_covers_neither_activity_is_refused_for_both_and_is_not_a_session() {
        // the fallback is one more try with the writes alone, and a refusal of that too is a refusal, not a session: the runner
        // goes on as it always did after one (it tries again, with the offer it has), and does not stay on the writes alone
        let (offered, polled_as) = lake_registrations_refused_with(vec![
            tonic::Status::permission_denied("NOT_BOUND"),
            tonic::Status::permission_denied("NOT_BOUND"),
        ])
        .await;

        assert_eq!(
            offered,
            [
                vec![ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY],
                vec![ICEBERG_WRITE_ACTIVITY],
                vec![ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY],
            ]
        );
        assert_eq!(polled_as, worker(9));
    }

    /// What the metrics saw while a lake runner registered and polled, its service refusing the first registrations with
    /// `statuses`. The registration is async and the reader of the metrics is a thread-local one: the runtime is a current-thread
    /// runtime run inside it, so that every task of the runner is polled on the thread the reader is on.
    fn metrics_of_lake_registrations_refused_with(
        statuses: Vec<tonic::Status>,
    ) -> crate::testing::Recorded {
        crate::testing::recorded(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(lake_registrations_refused_with(statuses));
        })
    }

    /// How many times the offer of the probe ended as `outcome` (`offered`, `not_bound` or `write_only`).
    fn offers(recorded: &crate::testing::Recorded, outcome: &str) -> u64 {
        recorded
            .counters
            .iter()
            .filter(|(name, labels, _)| {
                name == "database_runner_lake_probe_offer_total"
                    && labels.contains(&("outcome".to_owned(), outcome.to_owned()))
            })
            .map(|(_, _, amount)| amount)
            .sum()
    }

    /// Every value the gauge of the probe was set to, in order.
    fn offered_gauge(recorded: &crate::testing::Recorded) -> Vec<f64> {
        recorded
            .gauges
            .iter()
            .filter(|(name, _)| name == "database_runner_lake_probe_offered")
            .map(|(_, value)| *value)
            .collect()
    }

    #[test]
    fn a_runner_that_is_offering_the_probe_says_so_in_a_counter_and_a_gauge() {
        let recorded = metrics_of_lake_registrations_refused_with(vec![]);

        assert_eq!(offers(&recorded, "offered"), 1);
        assert_eq!(offers(&recorded, "not_bound"), 0);
        assert_eq!(offers(&recorded, "write_only"), 0);
        assert_eq!(offered_gauge(&recorded), [1.0]);
    }

    #[test]
    fn a_runner_left_with_the_writes_alone_says_so_in_the_counters_and_the_gauge_stays_down() {
        // the gauge is what an alert reads for days after the one registration that counted: it is down while the runner serves
        // the writes alone, and it is not set up by the offer that was refused
        let recorded =
            metrics_of_lake_registrations_refused_with(vec![tonic::Status::permission_denied(
                "NOT_BOUND",
            )]);

        assert_eq!(offers(&recorded, "not_bound"), 1);
        assert_eq!(offers(&recorded, "write_only"), 1);
        assert_eq!(offers(&recorded, "offered"), 0);
        assert_eq!(offered_gauge(&recorded), [0.0]);
    }

    #[test]
    fn a_refusal_that_is_not_a_binding_is_not_counted_as_one_and_the_gauge_is_not_set_down() {
        let recorded =
            metrics_of_lake_registrations_refused_with(vec![tonic::Status::unavailable("down")]);

        assert_eq!(offers(&recorded, "not_bound"), 0);
        assert_eq!(offers(&recorded, "write_only"), 0);
        assert_eq!(offers(&recorded, "offered"), 1);
        assert_eq!(offered_gauge(&recorded), [1.0]);
    }

    #[test]
    fn a_binding_that_covers_neither_activity_counts_the_refusal_and_is_not_a_runner_that_serves_the_writes()
     {
        // the writes alone were refused too: nothing was left standing, so it is not `write_only`, and the gauge is not set down
        // for a session that does not exist; the offer that the runner makes again is the one that is accepted
        let recorded = metrics_of_lake_registrations_refused_with(vec![
            tonic::Status::permission_denied("NOT_BOUND"),
            tonic::Status::permission_denied("NOT_BOUND"),
        ]);

        assert_eq!(offers(&recorded, "not_bound"), 1);
        assert_eq!(offers(&recorded, "write_only"), 0);
        assert_eq!(offers(&recorded, "offered"), 1);
        assert_eq!(offered_gauge(&recorded), [1.0]);
    }

    #[tokio::test]
    async fn a_configuration_with_a_lake_section_makes_a_lake_runner_and_one_without_makes_a_sql_runner()
     {
        let fake = Fake::default();
        let server = serve_fake(fake.clone()).await;
        // a short path: the work root is limited, because the attempt directories hold Unix sockets
        let work = tempfile::Builder::new()
            .prefix("dr")
            .tempdir_in("/tmp")
            .unwrap();
        let section = serde_json::json!({
            "workRoot": work.path(), "sandboxSocket": "/run/sagat/lake-sandbox.sock", "sandboxUid": 10002,
            "slots": 2, "relayAllowCidrs": ["0.0.0.0/0"], "relayDenyCidrs": ["10.0.0.0/8"]
        });
        let home = home(&server, Some(section));
        let runner = Runner::start(home.config.clone()).await.unwrap();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(runner.serve(stop.clone()));
        fake.until("a poll", |script| !script.polled.is_empty())
            .await;
        stop.cancel();
        ended(serving).await;
        fake.with(|script| {
            let capabilities = script.registered[0].capabilities.as_ref().unwrap();
            assert_eq!(
                capabilities.activities,
                [ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY]
            );
            assert_eq!(capabilities.max_inflight, 2);
        });
        // and a lake section that is not valid stops the runner: it is not served as a SQL runner
        let bad = serde_json::json!({"workRoot": "relative", "sandboxSocket": "/run/sagat/x.sock", "sandboxUid": 1,
            "slots": 2, "relayAllowCidrs": ["0.0.0.0/0"], "relayDenyCidrs": []});
        let broken = crate::testing::home(&server, Some(bad));
        assert!(Runner::start(broken.config.clone()).await.is_err());
        // and so does a work root that is not there
        let missing = serde_json::json!({"workRoot": "/tmp/dr-no-such-directory", "sandboxSocket": "/run/sagat/x.sock",
            "sandboxUid": 10002, "slots": 2, "relayAllowCidrs": ["0.0.0.0/0"], "relayDenyCidrs": ["10.0.0.0/8"]});
        let absent = crate::testing::home(&server, Some(missing));
        assert!(Runner::start(absent.config.clone()).await.is_err());
    }

    #[tokio::test]
    async fn a_lake_runner_sweeps_the_attempt_directories_nobody_owns_and_the_sweep_ends_with_it() {
        use crate::lake::sweep::SWEEP_AFTER;
        use std::time::SystemTime;
        let fake = Fake::default();
        let server = serve_fake(fake.clone()).await;
        let home = home(&server, None);
        let world = world();
        let stale = world.work.path().join(uuid::Uuid::new_v4().to_string());
        let young = world.work.path().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&stale).unwrap();
        std::fs::create_dir(&young).unwrap();
        std::fs::File::open(&stale)
            .unwrap()
            .set_modified(SystemTime::now() - SWEEP_AFTER - Duration::from_secs(60))
            .unwrap();
        let launch = TestLaunch::new(&world, Script::NeverConnects, 2);
        let runner = Runner::start_with(home.config.clone(), launch)
            .await
            .unwrap();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(runner.serve(stop.clone()));
        crate::testing::eventually("the sweep of the stale directory", || !stale.exists()).await;
        assert!(
            young.exists(),
            "a directory that may be an attempt's is left"
        );
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(20), serving)
            .await
            .expect("the runner ends, so the sweep did")
            .unwrap();
    }

    #[tokio::test]
    async fn a_lake_step_goes_from_the_poll_to_the_receipt_through_the_shell() {
        let fake = Fake::default();
        fake.with(|script| {
            script.database = Some(resolved_database(NAMESPACE_ID));
            script.tasks.push_back(lake_lease("task-1"));
        });
        let server = serve_fake(fake.clone()).await;
        let home = home(&server, None);
        let world = world();
        let launch = TestLaunch::new(&world, answer(receipt_line()), 2);
        let runner = Runner::start_with(home.config.clone(), launch.clone())
            .await
            .unwrap();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(runner.serve(stop.clone()));
        fake.until("the receipt", |script| !script.completed.is_empty())
            .await;
        stop.cancel();
        ended(serving).await;
        fake.with(|script| {
            assert_eq!(script.completed.len(), 1);
            assert_eq!(script.completed[0].task_id, "task-1");
            assert!(script.failed.is_empty());
            assert!(
                !format!("{:?}{:?}", script.registered, script.completed).contains("canary"),
                "no secret of the connection reaches the control plane"
            );
        });
        assert_eq!(launch.slots.in_use(), 0, "and the slot is back");
    }

    #[tokio::test]
    async fn a_lake_runner_does_not_poll_when_it_has_no_free_slot() {
        let fake = Fake::default();
        let server = serve_fake(fake.clone()).await;
        let home = home(&server, None);
        let world = world();
        let launch = TestLaunch::new(&world, Script::NeverConnects, 1);
        let held = launch.slots.take().unwrap();
        let runner = Runner::start_with(home.config.clone(), launch.clone())
            .await
            .unwrap();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(runner.serve(stop.clone()));
        // the loop goes round three times with the only slot held, then it is freed
        tokio::time::timeout(
            Duration::from_secs(20),
            launch.looked.subscribe().wait_for(|rounds| *rounds >= 3),
        )
        .await
        .expect("the loop looked at its free slots three times")
        .unwrap();
        assert!(
            fake.with(|script| script.polled.is_empty() && script.registered.is_empty()),
            "with no slot it neither registers nor polls"
        );
        drop(held);
        fake.until("a poll", |script| !script.polled.is_empty())
            .await;
        stop.cancel();
        ended(serving).await;
        fake.with(|script| assert!(script.polled.iter().all(|poll| poll.max_tasks == 1)));
    }

    #[tokio::test]
    async fn a_drain_gives_the_leases_up_as_lost_and_reports_nothing_for_a_write_that_was_under_way()
     {
        let fake = Fake::default();
        fake.with(|script| {
            script.database = Some(resolved_database(NAMESPACE_ID));
            script.tasks.push_back(lake_lease("task-1"));
        });
        let server = serve_fake(fake.clone()).await;
        let home = home(&server, None);
        let world = world();
        // the host has the request and never answers: the write is under way when the runner is stopped
        let launch = TestLaunch::new(&world, Script::Silent, 2);
        let runner = Runner::start_with(home.config.clone(), launch.clone())
            .await
            .unwrap();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(runner.serve(stop.clone()));
        fake.until("the connection asked for", |script| {
            !script.resolved.is_empty()
        })
        .await;
        // the host has been sent the request: only then would a cancel be an unknown outcome
        tokio::time::timeout(Duration::from_secs(20), launch.sandbox.requested.notified())
            .await
            .expect("the host was sent the request");
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(20), serving)
            .await
            .expect("the drain ends")
            .unwrap();
        fake.with(|script| {
            assert!(
                script.completed.is_empty() && script.failed.is_empty(),
                "a drain is not a cancel: nothing is reported, and the lease expires on its own"
            );
        });
        assert_eq!(
            launch.slots.in_use(),
            0,
            "the container was killed and the slot is back"
        );
        assert_eq!(
            launch
                .sandbox
                .kills
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }
}
