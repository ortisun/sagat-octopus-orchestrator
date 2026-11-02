mod control;
#[path = "faults.rs"]
mod faults;
mod rest;
use futures_util::{FutureExt, StreamExt};
use sagat_octopus_orchestrator_sdk::*;
use sha2::Digest;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
type Failure = Box<dyn std::error::Error>;
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}
fn millis(value: &Value, key: &str, default: u64) -> Duration {
    Duration::from_millis(value.get(key).and_then(Value::as_u64).unwrap_or(default))
}
#[derive(Default)]
struct Measurement {
    active: usize,
    peak: usize,
    deliveries: Vec<Value>,
    log: Vec<Value>,
}
type Shared = Arc<Mutex<Measurement>>;
fn run_json(run: Run) -> Value {
    let mut result = json!({});
    for key in [
        "runId",
        "workflowId",
        "versionId",
        "status",
        "version",
        "paused",
        "startedBy",
        "createdAt",
        "updatedAt",
        "finishedAt",
        "deadline",
        "traceId",
        "correlationId",
        "depth",
        "parent",
    ] {
        result[key] = run.raw.get(key).cloned().unwrap_or(Value::Null);
    }
    for key in ["chain", "rerunOf"] {
        if let Some(value) = run.raw.get(key).filter(|v| !v.is_null()) {
            result[key] = value.clone();
        }
    }
    result
}
fn log_entry(ctx: &ActivityContext, input: &Value, name: &str, kind: &str, outcome: &str) -> Value {
    let task = &ctx.task;
    let mut entry = json!({"activity":name,"kind":kind,"node":task.node,"attempt":task.attempt,"input":input,"taskKind":match task.task_kind{TaskKind::Forward=>"FORWARD",TaskKind::Compensation=>"COMPENSATION",TaskKind::Reconciliation=>"RECONCILIATION"},"targetNode":task.target_node,"targetIdempotencyKey":task.target_idempotency_key,"idempotencyKey":task.idempotency_key,"traceparentPresent":!task.traceparent.is_empty(),"traceId":task.traceparent.split('-').nth(1).unwrap_or_default(),"steps":task.steps,"deadlineAhead":ctx.deadline_epoch_ms() as i128>time::OffsetDateTime::now_utc().unix_timestamp_nanos()/1_000_000,"abortedReason":match ctx.cancellation_reason(){None=>Value::Null,Some(CancellationReason::RunCancelled)=>json!("run-cancelled"),Some(CancellationReason::LeaseLost)=>json!("lease-lost"),Some(CancellationReason::Shutdown)=>json!("shutdown")},"outcome":outcome});
    if let Some(instance) = &task.instance {
        entry["instance"] = json!({"instancePath":instance.instance_path,"definitionNodeId":instance.definition_node_id});
    }
    if let Some(index) = task.index {
        entry["scopeInput"] = json!({"item":task.item,"index":index});
    }
    entry
}
enum Produced {
    Json(Value),
    Blob(BlobOutput),
    Verdict(Verdict),
}
async fn behavior(
    ctx: ActivityContext,
    input: Value,
    name: String,
    kind: String,
    spec: Value,
    shared: Shared,
) -> Result<Produced, ActivityFailure> {
    {
        let mut measure = shared
            .lock()
            .map_err(|_| ActivityFailure::permanent("CONFORMANCE", "measurement unavailable"))?;
        measure.active += 1;
        measure.peak = measure.peak.max(measure.active);
        measure.deliveries.push(json!({"runId":ctx.task.run_id,"node":ctx.task.node,"takenAtUnixMs":time::OffsetDateTime::now_utc().unix_timestamp_nanos()/1_000_000}));
    }
    let mut outcome = "returned";
    let result=std::panic::AssertUnwindSafe(async {
        let hold=spec["holdUntilLogged"][ctx.task.index.unwrap_or(0) as usize].as_u64().unwrap_or(0) as usize;
        loop {if shared.lock().map_err(|_|ActivityFailure::permanent("CONFORMANCE","measurement unavailable"))?.log.len()>=hold{break;}tokio::select!{_=tokio::time::sleep(Duration::from_millis(10))=>{},_=ctx.cancellation.cancelled()=>{outcome="cancelled";return Ok(Produced::Json(Value::Null));}}}
        let fails=spec.get("fails").filter(|_|spec.get("failsUntilAttempt").and_then(Value::as_i64).is_none_or(|until|i64::from(ctx.task.attempt)<until));
        if let Some(fails)=fails{outcome="failed";return Err(if fails["retryable"]==true{ActivityFailure::retryable(text(fails,"code"),text(fails,"message"))}else{ActivityFailure::permanent(text(fails,"code"),text(fails,"message"))});}
        if spec.get("throws").is_some(){outcome="threw";panic!("conformance activity panic");}
        if let Some(message)=spec.get("outcomeUnknown").and_then(Value::as_str){outcome="failed";return Err(ActivityFailure::permanent("OUTCOME_UNKNOWN",message));}
        if spec["hang"]==true{ctx.cancellation.cancelled().await;outcome="cancelled";return if spec["onCancel"]["raises"]==true{Err(ActivityFailure::cancelled())}else{Ok(Produced::Json(Value::Null))};}
        if let Some(delay)=spec.get("sleepMs").and_then(Value::as_u64){
            if spec["ignoresCancel"]==true{tokio::time::sleep(Duration::from_millis(delay)).await;}else{
                tokio::select!{_=tokio::time::sleep(Duration::from_millis(delay))=>{},_=ctx.cancellation.cancelled()=>{outcome="cancelled";return if spec["onCancel"]["raises"]==true{Err(ActivityFailure::cancelled())}else{Ok(Produced::Json(spec["onCancel"]["returns"].clone()))};}}
            }
        }
        if let Some(secret)=spec.get("readsSecret").and_then(Value::as_str){
            let mut output=spec.get("returns").cloned().unwrap_or_else(||json!({}));
            match ctx.secrets().await {
                Ok(secrets)=>{output["secretLength"]=json!(secrets.get(secret)?.ok_or_else(||ActivityFailure::permanent("CONFORMANCE","secret is missing"))?.as_bytes().len());}
                Err(error) if spec["reportsSecretError"]==true=>output["secretError"]=json!(error.code),
                Err(error)=>return Err(error.into()),
            }return Ok(Produced::Json(output));
        }
        if let Some(size)=spec.get("blobRoundTripBytes").and_then(Value::as_u64){
            let body=vec![b'x';size as usize];let marker=ctx.write_blob(std::io::Cursor::new(body.clone()),WriteBlobOptions{expected_size:Some(size),expected_sha256:Some(format!("{:x}",sha2::Sha256::digest(&body))),..Default::default()}).await?;
            let mut stream=ctx.read_blob(marker.descriptor()).await?;let mut read=Vec::new();while let Some(chunk)=stream.next().await{read.extend_from_slice(&chunk?);}
if read!=body{return Err(ActivityFailure::permanent("CONFORMANCE","blob round trip failed"));}
            return Ok(Produced::Json(json!({"blobSize":marker.descriptor().size,"blobSha256":marker.descriptor().sha256,"roundTrip":true})));
        }
        if let Some(size)=spec.get("blobInputBytes").and_then(Value::as_u64){
            let descriptor:BlobDescriptor=serde_json::from_value(input["$blob"].clone()).map_err(|_|ActivityFailure::permanent("CONFORMANCE","invalid input blob"))?;
            let mut stream=ctx.read_blob(&descriptor).await?;let mut read=Vec::new();while let Some(chunk)=stream.next().await{read.extend_from_slice(&chunk?);}
if read!=vec![b'i';size as usize]{return Err(ActivityFailure::permanent("CONFORMANCE","run input blob did not match"));}
            return Ok(Produced::Json(json!({"inputBlobSize":descriptor.size,"inputBlobSha256":descriptor.sha256})));
        }
        for (key,byte,typed) in [("blobOutputBytes",b'y',true),("blobInlineBytes",b'z',false)]{if let Some(size)=spec.get(key).and_then(Value::as_u64){
            let marker=ctx.write_blob(std::io::Cursor::new(vec![byte;size as usize]),WriteBlobOptions::default()).await?;
            return Ok(if typed{Produced::Blob(marker)}else{Produced::Json(serde_json::to_value(marker).map_err(|_|ActivityFailure::permanent("CONFORMANCE","invalid blob output"))?)});
        }}
        if let Some(verdict)=spec.get("verdict").and_then(Value::as_str){return Ok(Produced::Verdict(match verdict{"applied"=>Verdict::applied(spec.get("output").cloned()),"not_applied"=>Verdict::not_applied(),_=>Verdict::unknown()}));}
        if spec["echo"]==true{return Ok(Produced::Json(json!({"input":input,"steps":ctx.task.steps})));}
        if spec["echoInput"]==true{return Ok(Produced::Json(input.clone()));}
        if let Some(count)=spec.get("returnsText").and_then(Value::as_u64){return Ok(Produced::Json(json!("x".repeat(count as usize))));}
        Ok(Produced::Json(spec.get("returns").cloned().unwrap_or(Value::Null)))
    }).catch_unwind().await;
    let entry = log_entry(&ctx, &input, &name, &kind, outcome);
    if let Ok(mut measure) = shared.lock() {
        measure.active -= 1;
        measure.log.push(entry);
    }
    match result {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

struct Worker {
    definition: Value,
    hosts: Vec<WorkerHost>,
    measure: Shared,
}
fn host(definition: &Value, config: &Value, measure: Shared) -> Result<WorkerHost, SagatError> {
    let mut options = WorkerOptions::new(
        text(config, "grpcUrl"),
        text(config, "namespace"),
        text(definition, "workerName"),
        TokenSource::bearer(text(
            config,
            if definition["token"] == "unbound" {
                "unboundWorkerToken"
            } else {
                "workerToken"
            },
        )),
    );
    options.max_inflight = definition["maxInflight"].as_u64().unwrap_or(4) as usize;
    if let Some(value) = definition.get("protocolVersion").and_then(Value::as_str) {
        options.protocol_version = value.into();
    }
    options.transport = match text(definition, "transport") {
        "poll" => WorkerTransport::Poll,
        "stream" => WorkerTransport::Stream,
        _ => WorkerTransport::Auto,
    };
    options.heartbeat_interval = definition
        .get("heartbeatIntervalMs")
        .and_then(Value::as_u64)
        .map(Duration::from_millis);
    options.retry.initial_delay = Duration::from_millis(20);
    options.retry.max_delay = Duration::from_millis(200);
    let mut host = WorkerHost::new(options)?;
    for (name, activity) in definition["activities"].as_object().into_iter().flatten() {
        let name = name.clone();
        let kind = activity
            .get("as")
            .and_then(Value::as_str)
            .unwrap_or("activity")
            .to_owned();
        let spec = activity
            .get("behavior")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let shared = measure.clone();
        let no_value = !["echo", "echoInput"].iter().any(|key| spec[*key] == true)
            && [
                "returns",
                "returnsText",
                "readsSecret",
                "blobRoundTripBytes",
                "blobInputBytes",
                "blobOutputBytes",
                "blobInlineBytes",
                "verdict",
            ]
            .iter()
            .all(|key| spec.get(*key).is_none_or(Value::is_null))
            && spec["onCancel"]["returns"].is_null();
        let register_name = name.clone();
        if kind == "reconciler" {
            host.reconciler::<Value, _, _>(&register_name, move |ctx, input| {
                let future = behavior(
                    ctx,
                    input,
                    name.clone(),
                    kind.clone(),
                    spec.clone(),
                    shared.clone(),
                );
                async move {
                    match future.await? {
                        Produced::Verdict(value) => Ok(value),
                        _ => Err(ActivityFailure::permanent(
                            "RECONCILIATION_ANSWER_INVALID",
                            "invalid reconciliation answer",
                        )),
                    }
                }
            })?;
        } else if spec.get("blobOutputBytes").is_some() {
            host.activity::<Value, BlobOutput, _, _>(&register_name, move |ctx, input| {
                let future = behavior(
                    ctx,
                    input,
                    name.clone(),
                    kind.clone(),
                    spec.clone(),
                    shared.clone(),
                );
                async move {
                    match future.await? {
                        Produced::Blob(value) => Ok(value),
                        _ => Err(ActivityFailure::permanent(
                            "CONFORMANCE",
                            "expected typed blob output",
                        )),
                    }
                }
            })?;
        } else if no_value {
            host.activity::<Value, (), _, _>(&register_name, move |ctx, input| {
                let future = behavior(
                    ctx,
                    input,
                    name.clone(),
                    kind.clone(),
                    spec.clone(),
                    shared.clone(),
                );
                async move { future.await.map(|_| ()) }
            })?;
        } else {
            host.activity::<Value, Value, _, _>(&register_name, move |ctx, input| {
                let future = behavior(
                    ctx,
                    input,
                    name.clone(),
                    kind.clone(),
                    spec.clone(),
                    shared.clone(),
                );
                async move {
                    match future.await? {
                        Produced::Json(value) => Ok(value),
                        _ => Err(ActivityFailure::permanent(
                            "CONFORMANCE",
                            "expected JSON output",
                        )),
                    }
                }
            })?;
        }
    }
    Ok(host)
}
async fn start_worker(worker: &mut Worker, config: &Value) -> Result<Value, SagatError> {
    let mut host = host(&worker.definition, config, worker.measure.clone())?;
    host.start().await?;
    let result = json!({"protocolVersion":host.protocol_version()});
    worker.hosts.push(host);
    Ok(result)
}
fn run_of(step: &Value, results: &BTreeMap<String, Value>) -> Result<String, SagatError> {
    if let Some(page) = step["page"].as_str() {
        return results
            .get(page)
            .and_then(|r| {
                r["items"][step["index"].as_u64().unwrap_or(0) as usize]["runId"].as_str()
            })
            .map(str::to_owned)
            .ok_or_else(|| SagatError::validation("a scenario page has no run"));
    }
    let result = results
        .get(text(step, "run"))
        .ok_or_else(|| SagatError::validation("a scenario step has no run"))?;
    result
        .get("runId")
        .or_else(|| result.get("newRunId"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| SagatError::validation("a scenario step has no run"))
}
pub async fn run(scenario: &Value, config: &Value) -> Result<Value, Failure> {
    let plan = faults::Plan::new(&scenario.get("faults").cloned().unwrap_or_else(|| json!([])))?;
    let proxies = faults::Proxies::start(config, &plan).await?;
    let config = &proxies.config;
    let mut clients = BTreeMap::new();
    for (name, key) in [
        ("default", "clientToken"),
        ("bad", "badToken"),
        ("other", "otherTenantToken"),
        ("worker", "workerToken"),
        ("operator", "operatorToken"),
        ("viewer", "viewerToken"),
        ("runner", "runnerToken"),
        ("revoked", "revokedToken"),
    ] {
        let mut options = ClientOptions::new(
            text(config, "apiUrl"),
            text(config, "namespace"),
            TokenSource::bearer(text(config, key)),
        );
        options.retry.initial_delay = Duration::from_millis(20);
        options.retry.max_delay = Duration::from_millis(200);
        clients.insert(name, Client::new(options)?);
    }
    let mut workers = BTreeMap::new();
    for (name, definition) in scenario["workers"].as_object().into_iter().flatten() {
        let mut worker = Worker {
            definition: definition.clone(),
            hosts: vec![],
            measure: Arc::default(),
        };
        if definition["autostart"] != false {
            start_worker(&mut worker, config).await?;
        }
        workers.insert(name.clone(), worker);
    }
    let mut observations = Vec::new();
    let mut results = BTreeMap::new();
    for step in scenario["steps"]
        .as_array()
        .ok_or("scenario steps missing")?
    {
        let op = text(step, "op");
        let token = step["token"].as_str().unwrap_or("default");
        let client = clients.get(token).ok_or("unknown scenario token")?;
        if !matches!(
            op,
            "start"
                | "startWithBlob"
                | "workflowByName"
                | "get"
                | "list"
                | "startWorker"
                | "stopWorker"
                | "restartWorker"
                | "waitForCompletion"
                | "events"
                | "sleep"
                | "control"
                | "cancel"
                | "operate"
                | "awaitJournal"
                | "awaitWorkerLog"
                | "probeWorker"
                | "signal"
                | "publishEvent"
                | "query"
                | "update"
                | "listApprovals"
                | "decideApproval"
                | "queryChain"
                | "updateChain"
                | "signalChain"
                | "cancelChain"
                | "listChainRuns"
                | "inspectChain"
                | "waitForChainCompletion"
        ) {
            return Err(
                format!("Rust runtime scenario operation {op} is not implemented yet").into(),
            );
        }
        let result: Result<Option<Value>,SagatError> = async { let output=match op{
            "start"|"workflowByName"=>{
                let workflow=&config["workflows"][text(step,"workflow")];let reference=text(workflow,if op=="start"{"workflowId"}else{"name"});
                let options=StartOptions{call:CallOptions{idempotency_key:step["idempotencyKey"].as_str().map(str::to_owned),traceparent:step["traceparent"].as_str().map(str::to_owned),..Default::default()},correlation_id:step["correlationId"].as_str().map(str::to_owned),deadline:step.get("deadlineInMs").and_then(Value::as_u64).map(|n|RunDeadline::After(Duration::from_millis(n))),..Default::default()};
                Some(run_json(client.start(reference,&step.get("input").cloned().unwrap_or_else(||json!({})),options).await?))
            }
            "startWithBlob"=>{
                let body=if let Some(text)=step["inputBlobText"].as_str(){text.as_bytes().to_vec()}else{vec![b'i';step["inputBlobBytes"].as_u64().unwrap_or(0) as usize]};
                let descriptor=client.upload_run_input(std::io::Cursor::new(body.clone()),UploadRunInputOptions{expected_size:Some(body.len() as u64),expected_sha256:Some(format!("{:x}",sha2::Sha256::digest(&body))),..Default::default()}).await?;
                let options=StartOptions{call:CallOptions{idempotency_key:step["idempotencyKey"].as_str().map(str::to_owned),traceparent:step["traceparent"].as_str().map(str::to_owned),..Default::default()},correlation_id:step["correlationId"].as_str().map(str::to_owned),deadline:step.get("deadlineInMs").and_then(Value::as_u64).map(|n|RunDeadline::After(Duration::from_millis(n))),..Default::default()};
                Some(run_json(client.start_with_blob(text(&config["workflows"][text(step,"workflow")],"workflowId"),&descriptor,options).await?))
            }
            "get"=>Some(run_json(client.get(&run_of(step,&results)?,Default::default()).await?)),
            "waitForCompletion"=>Some(run_json(client.wait_for_completion(&run_of(step,&results)?,WaitOptions{timeout:millis(step,"timeoutMs",30000),poll_interval:Duration::from_millis(50),..Default::default()}).await?)),
            "events"=>{let page=client.events(&run_of(step,&results)?,PageOptions{limit:Some(200),..Default::default()},Default::default()).await?;Some(json!({"events":page.items.iter().map(|e|json!({"type":e.event_type,"stepId":e.step_id,"attempt":e.attempt})).collect::<Vec<_>>()}))}
            "list"=>{
                let filter=ListOptions{status:step.get("status").cloned().map(serde_json::from_value).transpose().map_err(|_|SagatError::validation("invalid scenario status"))?,limit:step["limit"].as_u64().map(|n|n as u16),..Default::default()};
                let mut ids=Vec::new();if step["all"]==true{let stream=client.list_all(filter,Default::default());futures_util::pin_mut!(stream);while let Some(run)=stream.next().await{ids.push(run?.id);}}else{ids.extend(client.list(filter.clone(),Default::default()).await?.items.into_iter().map(|r|r.id));for _ in 0..100{if ids.len()>=step["waitForCount"].as_u64().unwrap_or(0) as usize{break;}tokio::time::sleep(Duration::from_millis(50)).await;ids=client.list(filter.clone(),Default::default()).await?.items.into_iter().map(|r|r.id).collect();}}
                Some(if step["countOnly"]==true{json!({"count":ids.len()})}else{json!({"items":ids})})
            }
            "startWorker"=>Some(start_worker(workers.get_mut(text(step,"worker")).ok_or_else(||SagatError::validation("unknown scenario worker"))?,config).await?),
            "stopWorker"|"restartWorker"=>{
                let worker=workers.get_mut(text(step,"worker")).ok_or_else(||SagatError::validation("unknown scenario worker"))?;
                for host in &mut worker.hosts{host.stop(millis(step,"timeoutMs",5000)).await?;}
                if op=="restartWorker"{let host=worker.hosts.last_mut().ok_or_else(||SagatError::validation("worker was never started"))?;host.start().await?;Some(json!({"protocolVersion":host.protocol_version()}))}else{None}
            }
            "cancel"|"operate"=>{let result=client.operate(&run_of(step,&results)?,if op=="cancel"{"CANCEL"}else{text(step,"action")},text(step,"reason"),OperationOptions{expected_run_version:step["expectedRunVersion"].as_u64(),step_id:step["stepId"].as_str().map(str::to_owned),..Default::default()}).await?;let mut value=json!({"accepted":result.accepted,"runStatus":result.run_status});if let Some(id)=result.new_run_id{value["newRunId"]=json!(id);}Some(value)}
            "sleep"=>{tokio::time::sleep(millis(step,"ms",0)).await;None}
            "control"=>{let mut body=json!({"action":step["action"]});if step.get("run").is_some(){body["run"]=json!(run_of(step,&results)?);}
if let Some(node)=step.get("node"){body["node"]=node.clone();}let response=control::client()?.post(format!("{}/control",text(config,"controlUrl"))).json(&body).send().await.map_err(|_|SagatError::validation("control transport failed"))?;if !response.status().is_success(){return Err(SagatError::validation("control request refused"));}None}
            "awaitJournal"=>{let run=run_of(step,&results)?;let end=tokio::time::Instant::now()+millis(step,"timeoutMs",20000);loop{let journal=client.events(&run,PageOptions{limit:Some(200),..Default::default()},Default::default()).await?;if journal.items.iter().any(|e|e.event_type==text(step,"eventType")&&(step.get("stepId").is_none()||e.step_id.as_deref()==step["stepId"].as_str())){break;}
if tokio::time::Instant::now()>=end{return Err(SagatError::validation("the scenario journal barrier was not reached"));}tokio::time::sleep(Duration::from_millis(25)).await;}None}
            "awaitWorkerLog"=>{let worker=workers.get(text(step,"worker")).ok_or_else(||SagatError::validation("unknown scenario worker"))?;let end=tokio::time::Instant::now()+millis(step,"timeoutMs",20000);loop{if worker.measure.lock().map_err(|_|SagatError::validation("measurement unavailable"))?.log.len()>=step["count"].as_u64().unwrap_or(0) as usize{break;}
if tokio::time::Instant::now()>=end{return Err(SagatError::validation("the scenario worker log barrier was not reached"));}tokio::time::sleep(Duration::from_millis(20)).await;}None}
            "probeWorker"=>{let mut options=WorkerOptions::new(text(config,"grpcUrl"),text(config,"namespace"),format!("probe-{}",text(config,"unique")),TokenSource::bearer(text(config,"workerToken")));options.protocol_version=text(step,"protocolVersion").into();options.retry.max_attempts=1;let mut probe=WorkerHost::new(options)?;probe.activity("conformance.probe",|_:ActivityContext,_:Value|async{Ok(())})?;let result=probe.start().await.map(|_|json!({"accepted":probe.protocol_version()}));let _=probe.stop(Duration::from_millis(500)).await;Some(result?)}
            _=>rest::play(op,client,step,config,&results).await?,
        }; Ok(output) }.await;
        let mut observation = json!({"id":step["id"],"op":op,"ok":result.is_ok()});
        match result {
            Ok(Some(result)) => {
                results.insert(text(step, "id").into(), result.clone());
                observation["result"] = result;
            }
            Ok(None) => {}
            Err(error) => observation["error"] = error.as_json(),
        }
        observations.push(observation);
    }
    let mut log = json!({});
    let mut measurements = json!({});
    for (name, worker) in &mut workers {
        for host in &mut worker.hosts {
            host.stop(Duration::from_secs(2)).await?;
        }
        let measure = worker
            .measure
            .lock()
            .map_err(|_| "measurement unavailable")?;
        log[name] = json!(measure.log);
        measurements[name] = json!({"peakInflight":measure.peak,"deliveries":measure.deliveries});
    }
    Ok(
        json!({"scenario":scenario["name"],"language":"rust","sdkVersion":VERSION,"observations":observations,"workerLog":log,"workerMeasurements":measurements,"faultsApplied":plan.applied()}),
    )
}
