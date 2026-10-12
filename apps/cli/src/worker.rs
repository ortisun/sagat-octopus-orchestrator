//! `sagat worker dev`: a worker that serves the activities of a workflow from a file of canned
//! answers, so that a workflow can be run, and made to fail, before any real worker exists.
//!
//! It is a real worker: it registers a session, polls, and completes or fails the steps it is
//! leased through the same protocol an SDK speaks. What is canned is the answer. The engine that
//! runs the workflow is the platform's own, which is the point: there is no emulator whose idea of
//! a retry, a compensation or a reconciliation could differ from the real one.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::pb::{
    CompleteStepRequest, FailStepRequest, PollTasksRequest, RegisterWorkerRequest, TaskLease,
    WorkerCapabilities,
};
use serde_json::{Value, json};
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

use crate::args::Common;
use crate::mocks::{Answer, Kind, Mocks, activities_by_role, activities_of};
use crate::{EXIT_DONE, Env, Io, credentials, manifest, namespace_of, one_line, resolve, session};

/// What `worker dev` was asked.
pub(crate) struct Ask<'a> {
    pub mocks: &'a Path,
    pub workflow: Option<&'a Path>,
    pub bind: bool,
    pub idle_exit: Option<u64>,
    pub max_tasks: Option<u64>,
    pub show_payloads: bool,
    pub allow_non_dev: bool,
}

/// Longest a payload shown with `--show-payloads` is.
const SHOWN: usize = 400;

/// The most tasks asked for at once.
const MAX_BATCH: u64 = 8;

/// Becomes true when the person stops the worker: Ctrl-C, or the termination signal of a process
/// manager. One task listens from the start.
pub(crate) fn stop_signal() -> tokio::sync::watch::Receiver<bool> {
    let (tell, hear) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            match signal(SignalKind::terminate()) {
                Ok(mut term) => {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {}
                        _ = term.recv() => {}
                    }
                }
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let _ = tell.send(true);
    });
    hear
}

fn clip(text: &str) -> String {
    let flat = one_line(text);
    if flat.chars().count() <= SHOWN {
        flat
    } else {
        format!("{}…", flat.chars().take(SHOWN).collect::<String>())
    }
}

/// Why a workflow file was not read, in words: never the Debug form of the error.
fn unreadable(error: manifest::DocumentError, path: &Path) -> String {
    let (manifest::DocumentError::TooLarge(why)
    | manifest::DocumentError::Unreadable(why)
    | manifest::DocumentError::Malformed(why)) = error;
    format!("{}: {}", path.display(), one_line(&why))
}

fn read_mocks(env: &Env, path: &Path) -> Result<Mocks, String> {
    let path = resolve(env, path);
    let text = manifest::read_limited(&path, "mocks").map_err(|p| p.to_string())?;
    Mocks::parse(&text, manifest::is_yaml(&path)).map_err(|problems| {
        format!(
            "the mock file {} has {} problem{}:\n  {}",
            path.display(),
            problems.len(),
            if problems.len() == 1 { "" } else { "s" },
            problems.join("\n  ")
        )
    })
}

async fn channel(address: &str) -> Result<Channel, String> {
    let mut endpoint = Endpoint::from_shared(address.to_owned())
        .map_err(|_| "the worker address is not a URL like http://127.0.0.1:19000".to_owned())?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30));
    if address.starts_with("https://") {
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new().with_enabled_roots())
            .map_err(|_| "the TLS settings for the worker endpoint are not usable".to_owned())?;
    }
    endpoint.connect().await.map_err(|e| {
        let text = format!("{e:?}").to_lowercase();
        if text.contains("dns") || text.contains("lookup") {
            "the worker endpoint's host name does not resolve (DNS)".to_owned()
        } else if text.contains("refused") {
            "connection refused: nothing listens at the worker endpoint (the engine's worker port is not the API's)".to_owned()
        } else {
            "could not connect to the worker endpoint".to_owned()
        }
    })
}

fn authed<T>(token: &str, message: T) -> Result<tonic::Request<T>, String> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}"))
            .map_err(|_| "the token cannot travel in a header".to_owned())?,
    );
    if let Some(traceparent) = crate::telemetry::current_traceparent() {
        request.metadata_mut().insert(
            "traceparent",
            MetadataValue::try_from(traceparent)
                .map_err(|_| "the trace context cannot travel in a header".to_owned())?,
        );
    }
    Ok(request)
}

/// The machine code the platform put in the trailers of a refusal, or the status.
fn refusal(status: &tonic::Status) -> String {
    status
        .metadata()
        .get("x-sagat-error")
        .and_then(|v| v.to_str().ok())
        .map_or_else(|| format!("{:?}", status.code()), str::to_owned)
}

struct Dev<'a> {
    client: WorkerServiceClient<Channel>,
    token: String,
    namespace: String,
    activities: Vec<String>,
    worker_id: String,
    session: String,
    mocks: &'a Mocks,
    show_payloads: bool,
}

impl Dev<'_> {
    async fn register(&mut self) -> Result<String, String> {
        let request = authed(
            &self.token,
            RegisterWorkerRequest {
                worker_name: format!("sagat-dev-{}", std::process::id()),
                namespace: self.namespace.clone(),
                capabilities: Some(WorkerCapabilities {
                    languages: vec![],
                    activities: self.activities.clone(),
                    pools: vec![],
                    max_inflight: 8,
                    protocol_version: crate::doctor::WORKER_PROTOCOL.to_owned(),
                }),
            },
        )?;
        let answer = self.client.register_worker(request).await.map_err(|status| match refusal(&status).as_str() {
            "NOT_BOUND" => format!(
                "this identity is not bound to those activities in {}: run again with --bind (as an administrator), or ask one to bind it (PUT /namespaces/{{id}}/worker-bindings/{{subject}})",
                self.namespace
            ),
            other => format!("the platform refused the registration ({other}): {}", one_line(status.message())),
        })?;
        let answer = answer.into_inner();
        self.worker_id = answer.worker_id;
        self.session = answer.session_token;
        Ok(answer.accepted_protocol_version)
    }

    async fn poll(&mut self, want: u64) -> Result<(Vec<TaskLease>, u64), tonic::Status> {
        let request = authed(
            &self.token,
            PollTasksRequest {
                worker_id: self.worker_id.clone(),
                session_token: self.session.clone(),
                max_tasks: i32::try_from(want).unwrap_or(1),
            },
        )
        .map_err(tonic::Status::internal)?;
        let answer = self.client.poll_tasks(request).await?.into_inner();
        Ok((
            answer.tasks,
            u64::try_from(answer.retry_after_ms).unwrap_or(0),
        ))
    }

    /// Answers one task. `false` when the worker was told to stop before it answered: the task is
    /// left to its lease, as a worker that went away would leave it.
    async fn serve(
        &mut self,
        task: &TaskLease,
        stop: &mut tokio::sync::watch::Receiver<bool>,
        io: &mut Io<'_>,
    ) -> Result<bool, String> {
        let Some(kind) = Kind::of(&task.task_kind) else {
            return Err(format!(
                "a task of a kind this tool does not know ({})",
                one_line(&task.task_kind)
            ));
        };
        let (answer, delay) = self.mocks.answer(kind, &task.activity_name);
        if delay > 0 {
            tokio::select! {
                () = tokio::time::sleep(Duration::from_millis(delay)) => {}
                _ = stop.changed() => return Ok(false),
            }
        }
        let short: String = task.run_id.chars().take(8).collect();
        let label = match kind {
            Kind::Forward => "",
            Kind::Compensation => " (compensation)",
            Kind::Reconciliation => " (reconciliation)",
        };
        let mut shown = format!("run {short}  {}{label}", one_line(&task.activity_name));
        if self.show_payloads {
            shown.push_str(&format!("\n      input  {}", clip(&task.input_json)));
        }
        if answer == Answer::Lose {
            io.say(format!(
                "{shown}  takes the task and never answers (a worker that died): the lease will run out"
            ));
            return Ok(true);
        }
        let key = format!("dev-{}-{}", task.task_id, task.attempt);
        let (outcome, report) = match &answer {
            Answer::Fail {
                code,
                retryable,
                message,
            } => {
                let request = authed(
                    &self.token,
                    FailStepRequest {
                        task_id: task.task_id.clone(),
                        run_id: task.run_id.clone(),
                        step_id: task.step_id.clone(),
                        attempt: task.attempt,
                        generation: task.generation,
                        lease_token: task.lease_token.clone(),
                        idempotency_key: key,
                        error_code: code.clone(),
                        safe_message: message.chars().take(500).collect(),
                        retryable: *retryable,
                        traceparent: task.traceparent.clone(),
                    },
                )?;
                (
                    format!(
                        "fails [{code}]{}",
                        if *retryable { " (retryable)" } else { "" }
                    ),
                    self.client.fail_step(request).await,
                )
            }
            other => {
                let output = match other {
                    Answer::Output(v) => v.clone(),
                    _ => serde_json::from_str(&task.input_json).unwrap_or_else(|_| json!({})),
                };
                let text = output.to_string();
                let request = authed(
                    &self.token,
                    CompleteStepRequest {
                        task_id: task.task_id.clone(),
                        run_id: task.run_id.clone(),
                        step_id: task.step_id.clone(),
                        attempt: task.attempt,
                        generation: task.generation,
                        lease_token: task.lease_token.clone(),
                        idempotency_key: key,
                        output_json: text.clone(),
                        output_blob_ref: String::new(),
                        output_sha256: String::new(),
                        traceparent: task.traceparent.clone(),
                    },
                )?;
                let effect = (kind == Kind::Reconciliation)
                    .then(|| output["effect"].as_str().unwrap_or("?").to_owned());
                if self.show_payloads {
                    shown.push_str(&format!("\n      output {}", clip(&text)));
                }
                (
                    effect.map_or_else(|| "completes".to_owned(), |e| format!("answers: {e}")),
                    self.client.complete_step(request).await,
                )
            }
        };
        match report {
            Ok(_) => io.say(format!("{shown}  {outcome}")),
            // The lease is no longer ours (the attempt was reclaimed, the run cancelled): say it and go on.
            Err(status) if refusal(&status) == "STALE_ATTEMPT" => {
                io.say(format!(
                    "{shown}  {outcome}, but the attempt was no longer current"
                ));
            }
            Err(status) => {
                return Err(format!(
                    "the platform refused the report ({}): {}",
                    refusal(&status),
                    one_line(status.message())
                ));
            }
        }
        Ok(true)
    }
}

pub(crate) async fn dev(
    common: &Common,
    env: &Env,
    ask: &Ask<'_>,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let mocks = read_mocks(env, ask.mocks)?;
    let mut names: BTreeSet<String> = mocks.names();
    let mut missing = Vec::new();
    if let Some(file) = ask.workflow {
        let path = resolve(env, file);
        let loaded = manifest::load_one(&path).map_err(|e| unreadable(e, &path))?;
        let used = named_by(&loaded.value)?;
        let (forward, undo, lookup) = activities_by_role(&loaded.value);
        let elsewhere = |name: &String, section: &str| {
            let others: Vec<&str> = [
                ("activities", mocks.activities.contains_key(name)),
                ("compensations", mocks.compensations.contains_key(name)),
                ("reconcilers", mocks.reconcilers.contains_key(name)),
            ]
            .into_iter()
            .filter(|(s, there)| *there && *s != section)
            .map(|(s, _)| s)
            .collect();
            if others.is_empty() {
                String::new()
            } else {
                format!(
                    " (the file has an answer for it under {}, where this workflow does not ask for it)",
                    others.join(" and ")
                )
            }
        };
        for name in &forward {
            if !mocks.activities.contains_key(name) {
                missing.push(format!(
                    "no answer for '{}' under activities in the mock file: it will fail as NO_MOCK{}",
                    one_line(name),
                    elsewhere(name, "activities")
                ));
            }
        }
        for name in &undo {
            if !mocks.compensations.contains_key(name) {
                missing.push(format!(
                    "'{}' undoes a step and has no answer under compensations in the mock file: it will succeed with {{}}{}",
                    one_line(name),
                    elsewhere(name, "compensations")
                ));
            }
        }
        for name in &lookup {
            if !mocks.reconcilers.contains_key(name) {
                missing.push(format!(
                    "'{}' looks up whether a step happened and has no answer under reconcilers in the mock file: it will say 'unknown'{}",
                    one_line(name),
                    elsewhere(name, "reconcilers")
                ));
            }
        }
        names.extend(used);
    }
    if names.is_empty() {
        return Err(
            "nothing to serve: the mock file names no activity (and no --workflow was given)"
                .to_owned(),
        );
    }
    let grpc = common
        .grpc
        .clone()
        .or_else(|| env.grpc.clone())
        .ok_or("the worker endpoint is missing: pass --grpc <url> or set SAGAT_GRPC (http://127.0.0.1:19000 for the local stack)")?;
    if !crate::client::is_safe_address(grpc.trim_end_matches('/')) {
        return Err(
            "the worker endpoint is an https URL (plain http only for localhost, 127.0.0.1 or [::1]): the token must not travel in the clear"
                .to_owned(),
        );
    }
    let (_, token) = credentials(common, env)?;
    let namespace = namespace_of(common, env)?;
    let environment = environment_of(common, env, &namespace, ask.allow_non_dev, io).await?;

    if ask.bind {
        bind(common, env, &namespace, &names, io).await?;
    }
    let mut dev = Dev {
        client: WorkerServiceClient::new(channel(&grpc).await?),
        token,
        namespace: namespace.clone(),
        activities: names.iter().cloned().collect(),
        worker_id: String::new(),
        session: String::new(),
        mocks: &mocks,
        show_payloads: ask.show_payloads,
    };
    let protocol = dev.register().await?;
    io.say(format!(
        "serving {} activit{} in {namespace} ({environment}) at protocol {protocol}; stop with Ctrl-C",
        names.len(),
        if names.len() == 1 { "y" } else { "ies" }
    ));
    for message in &missing {
        io.warn(message);
    }
    let idle_limit = ask.idle_exit.map(Duration::from_secs);
    let mut idle_since = Instant::now();
    let mut served = 0u64;
    // Listening for the signal is one task from here to the end: a signal that arrives while the
    // loop is in the middle of anything but a poll must not be lost.
    let mut stop = io.stop();
    'serving: loop {
        if *stop.borrow() {
            io.say("stopping");
            break;
        }
        let remaining = ask.max_tasks.map(|max| max.saturating_sub(served));
        if remaining == Some(0) {
            break;
        }
        let want = remaining.map_or(MAX_BATCH, |r| r.min(MAX_BATCH));
        let polled = tokio::select! {
            polled = dev.poll(want) => polled,
            _ = stop.changed() => {
                io.say("stopping");
                break;
            }
        };
        match polled {
            Ok((tasks, retry_after)) => {
                if tasks.is_empty() {
                    if idle_limit.is_some_and(|limit| idle_since.elapsed() >= limit) {
                        io.say(format!(
                            "no task for {} s: stopping",
                            idle_limit.map_or(0, |l| l.as_secs())
                        ));
                        break;
                    }
                    tokio::select! {
                        () = tokio::time::sleep(Duration::from_millis(retry_after.clamp(100, 1000))) => {}
                        _ = stop.changed() => {
                            io.say("stopping");
                            break 'serving;
                        }
                    }
                    continue;
                }
                idle_since = Instant::now();
                for task in &tasks {
                    if !dev.serve(task, &mut stop, io).await? {
                        io.say("stopping");
                        break 'serving;
                    }
                    served += 1;
                }
            }
            // The session ended (the platform restarted, or it expired): ask for another.
            Err(status) if status.code() == tonic::Code::Unauthenticated => {
                dev.register().await?;
            }
            Err(status) => {
                return Err(format!(
                    "the platform refused to hand out tasks ({}): {}",
                    refusal(&status),
                    one_line(status.message())
                ));
            }
        }
    }
    io.say(format!(
        "served {served} task{}",
        if served == 1 { "" } else { "s" }
    ));
    Ok(EXIT_DONE)
}

/// The activities a workflow document names, each of them a name and not a pattern: a binding is
/// made of what a document says, and a document from somebody else's repository can say `*`.
fn named_by(document: &Value) -> Result<BTreeSet<String>, String> {
    let names = activities_of(document);
    for name in &names {
        if crate::mocks::bad_name(name) {
            return Err(format!(
                "the workflow names '{}' as an activity, which is not an activity name: a worker is bound to names, never to patterns",
                one_line(name)
            ));
        }
    }
    Ok(names)
}

/// Which environment the namespace is, and whether canned answers may be given to its tasks. They
/// may in a `dev` namespace; anywhere else, or when the platform will not say, only when asked.
async fn environment_of(
    common: &Common,
    env: &Env,
    namespace: &str,
    allow_non_dev: bool,
    io: &mut Io<'_>,
) -> Result<String, String> {
    let session = session(common, env, namespace)?;
    let environment = crate::remote::namespaces(&session.client)
        .await
        .ok()
        .and_then(|all| all.into_iter().find(|n| n.slug == namespace))
        .and_then(|n| n.environment);
    match (environment.as_deref(), allow_non_dev) {
        (Some("dev"), _) => Ok("dev".to_owned()),
        (Some(other), true) => {
            io.warn(format!(
                "the namespace {namespace} is {}: its real tasks will get canned answers, as you asked",
                one_line(other)
            ));
            Ok(one_line(other))
        }
        (None, true) => {
            io.warn(format!(
                "could not tell which environment {namespace} is: its real tasks will get canned answers, as you asked"
            ));
            Ok("environment unknown".to_owned())
        }
        (Some(other), false) => Err(format!(
            "the namespace {namespace} is a {} namespace: worker commands here would give canned answers to real tasks (or replace the bindings of real workers). Use a dev namespace, or pass --allow-non-dev if that is what you mean",
            one_line(other)
        )),
        (None, false) => Err(format!(
            "could not tell which environment {namespace} is (listing the namespaces needs the namespace:read permission): pass --allow-non-dev if you are sure it is a development one"
        )),
    }
}

/// `sagat worker bind`: the same binding `worker dev --bind` makes, for a worker written with an SDK.
pub(crate) async fn bind_command(
    common: &Common,
    env: &Env,
    workflow: Option<&Path>,
    activities: &[String],
    allow_non_dev: bool,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for name in activities {
        if crate::mocks::bad_name(name) {
            return Err(format!("'{}' is not an activity name", one_line(name)));
        }
        names.insert(name.clone());
    }
    if let Some(file) = workflow {
        let path = resolve(env, file);
        let loaded = manifest::load_one(&path).map_err(|e| unreadable(e, &path))?;
        names.extend(named_by(&loaded.value)?);
    }
    if names.is_empty() {
        return Err("the workflow names no activity and none was given".to_owned());
    }
    if names.len() > crate::mocks::MAX_NAMES {
        return Err(format!("at most {} activities", crate::mocks::MAX_NAMES));
    }
    let namespace = namespace_of(common, env)?;
    environment_of(common, env, &namespace, allow_non_dev, io).await?;
    bind(common, env, &namespace, &names, io).await?;
    Ok(EXIT_DONE)
}

/// Binds the identity of the token to the activities, as an administrator would. It replaces what
/// the identity was bound to in the namespace: said, because that identity may serve a worker too.
async fn bind(
    common: &Common,
    env: &Env,
    namespace: &str,
    names: &BTreeSet<String>,
    io: &mut Io<'_>,
) -> Result<(), String> {
    let session = session(common, env, namespace)?;
    let client = &session.client;
    let (_, who) = client
        .call("GET", "/auth/context", None, None)
        .await
        .map_err(|e| e.to_string())?;
    let subject = who["subject"]
        .as_str()
        .ok_or("the platform did not say who the token is for")?;
    let known = crate::remote::namespaces(client).await?;
    let id = known
        .iter()
        .find(|n| n.slug == namespace)
        .map(|n| n.id.clone())
        .ok_or_else(|| format!("the tenant has no namespace '{namespace}'"))?;
    let path = format!(
        "/namespaces/{id}/worker-bindings/{}",
        crate::client::encode(subject)
    );
    client
        .call("PUT", &path, Some(&json!({ "activities": names })), None)
        .await
        .map_err(|e| format!("could not bind {}: {e}", one_line(subject)))?;
    io.say(format!(
        "bound {} to {} activit{} in {namespace} (this replaces what it was bound to there)",
        one_line(subject),
        names.len(),
        if names.len() == 1 { "y" } else { "ies" }
    ));
    let _: &Value = &who;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::authed;

    #[tokio::test]
    async fn worker_requests_continue_the_instrumented_command_trace() {
        let traceparent = "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01";
        crate::telemetry::under_traceparent(traceparent.to_owned(), async {
            let request = authed("a-token", ()).expect("request");
            assert_eq!(
                request
                    .metadata()
                    .get("traceparent")
                    .and_then(|value| value.to_str().ok()),
                Some(traceparent)
            );
        })
        .await;
    }
}
