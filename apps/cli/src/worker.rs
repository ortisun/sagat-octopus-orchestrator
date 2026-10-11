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
use crate::mocks::{Answer, Kind, Mocks, activities_of};
use crate::{EXIT_DONE, Env, Io, credentials, manifest, namespace_of, one_line, resolve, session};

/// What `worker dev` was asked.
pub(crate) struct Ask<'a> {
    pub mocks: &'a Path,
    pub workflow: Option<&'a Path>,
    pub bind: bool,
    pub idle_exit: Option<u64>,
    pub max_tasks: Option<u64>,
    pub show_payloads: bool,
}

/// Longest a payload shown with `--show-payloads` is.
const SHOWN: usize = 400;

fn clip(text: &str) -> String {
    let flat = one_line(text);
    if flat.chars().count() <= SHOWN {
        flat
    } else {
        format!("{}…", flat.chars().take(SHOWN).collect::<String>())
    }
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

    async fn poll(&mut self) -> Result<(Vec<TaskLease>, u64), tonic::Status> {
        let request = authed(
            &self.token,
            PollTasksRequest {
                worker_id: self.worker_id.clone(),
                session_token: self.session.clone(),
                max_tasks: 8,
            },
        )
        .map_err(tonic::Status::internal)?;
        let answer = self.client.poll_tasks(request).await?.into_inner();
        Ok((
            answer.tasks,
            u64::try_from(answer.retry_after_ms).unwrap_or(0),
        ))
    }

    /// Answers one task; returns the line that tells what was done.
    async fn serve(&mut self, task: &TaskLease, io: &mut Io<'_>) -> Result<(), String> {
        let Some(kind) = Kind::of(&task.task_kind) else {
            return Err(format!(
                "a task of a kind this tool does not know ({})",
                one_line(&task.task_kind)
            ));
        };
        let (answer, delay) = self.mocks.answer(kind, &task.activity_name);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
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
            return Ok(());
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
        Ok(())
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
        let loaded = manifest::load_one(&resolve(env, file)).map_err(|e| format!("{e:?}"))?;
        let used = activities_of(&loaded.value);
        for name in &used {
            if !mocks.activities.contains_key(name)
                && !mocks.compensations.contains_key(name)
                && !mocks.reconcilers.contains_key(name)
            {
                missing.push(name.clone());
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
        .or_else(|| std::env::var("SAGAT_GRPC").ok())
        .ok_or("the worker endpoint is missing: pass --grpc <url> or set SAGAT_GRPC (http://127.0.0.1:19000 for the local stack)")?;
    let (_, token) = credentials(common, env)?;
    let namespace = namespace_of(common, env)?;

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
        "serving {} activit{} in {namespace} at protocol {protocol}; stop with Ctrl-C",
        names.len(),
        if names.len() == 1 { "y" } else { "ies" }
    ));
    for name in &missing {
        io.warn(format!(
            "no answer for '{name}' in the mock file: a compensation will succeed, a reconciliation will say 'unknown', anything else will fail as NO_MOCK"
        ));
    }
    let idle_limit = ask.idle_exit.map(Duration::from_secs);
    let mut idle_since = Instant::now();
    let mut served = 0u64;
    loop {
        if ask.max_tasks.is_some_and(|max| served >= max) {
            break;
        }
        let polled = tokio::select! {
            polled = dev.poll() => polled,
            _ = tokio::signal::ctrl_c() => {
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
                    tokio::time::sleep(Duration::from_millis(retry_after.clamp(100, 1000))).await;
                    continue;
                }
                idle_since = Instant::now();
                for task in &tasks {
                    dev.serve(task, io).await?;
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

/// `sagat worker bind`: the same binding `worker dev --bind` makes, for a worker written with an SDK.
pub(crate) async fn bind_command(
    common: &Common,
    env: &Env,
    workflow: Option<&Path>,
    activities: &[String],
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
        let loaded = manifest::load_one(&resolve(env, file)).map_err(|e| format!("{e:?}"))?;
        names.extend(activities_of(&loaded.value));
    }
    if names.is_empty() {
        return Err("the workflow names no activity and none was given".to_owned());
    }
    if names.len() > crate::mocks::MAX_NAMES {
        return Err(format!("at most {} activities", crate::mocks::MAX_NAMES));
    }
    let namespace = namespace_of(common, env)?;
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
