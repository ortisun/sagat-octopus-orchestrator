//! Fair bounded polling with one session per namespace and finite graceful shutdown.
use crate::{
    RunnerError, attempt,
    config::Config,
    lake::shell::{Launch, Production},
    rpc::Rpc,
};
use sagat_api_grpc::pb;
use sagat_domain::{
    database::{ICEBERG_WRITE_ACTIVITY, MANAGED_DATABASE_ACTIVITY},
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
                )))
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
        let namespace = &self.config.namespaces[index];
        let request = self
            .rpc
            .request(pb::RegisterWorkerRequest {
                worker_name: format!("{}/{}", self.config.name, namespace.slug),
                namespace: namespace.slug.clone(),
                capabilities: Some(pb::WorkerCapabilities {
                    activities: vec![activity.into()],
                    languages: vec![],
                    pools: vec![],
                    max_inflight,
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
                    match self
                        .register(index, ICEBERG_WRITE_ACTIVITY, max_inflight)
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
                            metrics::counter!("database_runner_attempts_total","outcome"=>if result.is_ok() { "reported" } else { "unacknowledged" }).increment(1);
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
        serving.await.unwrap();
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
    use sagat_domain::database::ICEBERG_WRITE_ACTIVITY;

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
    async fn a_lake_runner_offers_the_lake_activity_alone_and_no_more_attempts_than_it_has_slots() {
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
        serving.await.unwrap();
        fake.with(|script| {
            let capabilities = script.registered[0].capabilities.as_ref().unwrap();
            assert_eq!(capabilities.activities, [ICEBERG_WRITE_ACTIVITY]);
            assert_eq!(capabilities.max_inflight, 2, "the slots, not maxConcurrent");
            assert_eq!(capabilities.protocol_version, "1.6");
            assert_eq!(script.polled[0].max_tasks, 2);
        });
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
        serving.await.unwrap();
        fake.with(|script| {
            let capabilities = script.registered[0].capabilities.as_ref().unwrap();
            assert_eq!(capabilities.activities, [ICEBERG_WRITE_ACTIVITY]);
            assert_eq!(capabilities.max_inflight, 2);
        });
        // and a lake section that is not valid stops the runner: it is not served as a SQL runner
        let bad = serde_json::json!({"workRoot": "relative", "sandboxSocket": "/run/sagat/x.sock", "sandboxUid": 1,
            "slots": 2, "relayAllowCidrs": ["0.0.0.0/0"], "relayDenyCidrs": []});
        let broken = crate::testing::home(&server, Some(bad));
        assert!(Runner::start(broken.config.clone()).await.is_err());
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
        serving.await.unwrap();
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
        serving.await.unwrap();
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
