//! `sagat workflow validate|publish|run|inspect|cancel`: one document, and the runs of a workflow.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::args::Common;
use crate::client::{CallError, Client};
use crate::diagnostics::{Diagnostic, of_document};
use crate::manifest::{self, DocumentError};
use crate::{
    EXIT_DONE, EXIT_FAILED, EXIT_TIMEOUT, Env, Io, credentials, namespace_of, remote, resolve,
    session,
};

/// The statuses after which a run no longer changes.
const FINAL: [&str; 7] = [
    "SUCCEEDED",
    "FAILED",
    "CANCELLED",
    "TIMED_OUT",
    "COMPENSATED",
    "PARTIAL",
    "COMPENSATION_FAILED",
];

fn problem_of(error: &DocumentError, path: &Path) -> Result<Diagnostic, String> {
    let (code, message) = match error {
        DocumentError::TooLarge(why) => ("DOCUMENT_TOO_LARGE", why.clone()),
        DocumentError::Malformed(why) => ("DOCUMENT_MALFORMED", why.clone()),
        DocumentError::Unreadable(why) => return Err(why.clone()),
    };
    Ok(Diagnostic {
        warning: false,
        code: code.to_owned(),
        message,
        at: Some(format!("file {}", path.display())),
        help: crate::diagnostics::help(code),
    })
}

fn say_diagnostics(io: &mut Io<'_>, diagnostics: &[Diagnostic]) {
    for line in crate::diagnostics::render(diagnostics, io.docs()) {
        io.say(line);
    }
}

pub(crate) async fn validate(
    common: &Common,
    env: &Env,
    file: &Path,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let path = resolve(env, file);
    let loaded = match manifest::load_one(&path) {
        Ok(loaded) => loaded,
        Err(error) => {
            say_diagnostics(io, &[problem_of(&error, &path)?]);
            return Ok(EXIT_FAILED);
        }
    };
    let diagnostics = of_document(&loaded.document);
    say_diagnostics(io, &diagnostics);
    let errors = diagnostics.iter().filter(|d| !d.warning).count();
    if errors > 0 {
        io.warn(format!(
            "{} is not publishable: {errors} problem{}",
            path.display(),
            if errors == 1 { "" } else { "s" }
        ));
        return Ok(EXIT_FAILED);
    }
    let name = &loaded.entry.name;
    let given = common.server.is_some()
        || common.token_file.is_some()
        || env.server.is_some()
        || env.token.is_some();
    if !given {
        io.say(format!(
            "ok    {name}: a valid document. Not checked against the platform (secrets, scripts): pass --server and a token to do that."
        ));
        return Ok(EXIT_DONE);
    }
    credentials(common, env)?;
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let known = remote::workflows(&session.client).await?;
    match known.get(name) {
        None => io.say(format!(
            "ok    {name}: a valid document. The platform has no workflow of this name in {namespace} yet, so what only it can say (secrets, scripts) is checked when you publish."
        )),
        Some(existing) => {
            if crate::validate::check_on_platform(&session, &existing.id, &loaded.value, name, io)
                .await?
            {
                io.say(format!(
                    "ok    {name}: valid, and the platform would publish it in {namespace}"
                ));
            } else {
                return Ok(EXIT_FAILED);
            }
        }
    }
    Ok(EXIT_DONE)
}

pub(crate) async fn publish(
    common: &Common,
    env: &Env,
    file: &Path,
    notes: Option<&str>,
    dry_run: bool,
    accept_breaking: bool,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let path = resolve(env, file);
    let mut loaded = match manifest::load_one(&path) {
        Ok(loaded) => loaded,
        Err(error) => {
            say_diagnostics(io, &[problem_of(&error, &path)?]);
            return Ok(EXIT_FAILED);
        }
    };
    if !manifest::is_slug(&loaded.entry.name) {
        say_diagnostics(
            io,
            &[Diagnostic {
                warning: false,
                code: "SCHEMA".to_owned(),
                message: "metadata.name is a workflow name: lower case letters, digits and hyphens, 3 to 63 characters".to_owned(),
                at: Some("at /metadata/name".to_owned()),
                help: crate::diagnostics::help("SCHEMA"),
            }],
        );
        return Ok(EXIT_FAILED);
    }
    loaded.entry.release_notes = notes.map(str::to_owned);
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    crate::sync::publish_documents(
        &session,
        &[loaded],
        &namespace,
        dry_run,
        accept_breaking,
        io,
    )
    .await
}

/// The input of a run: JSON text, or `@path` for a file.
fn input_of(env: &Env, input: Option<&str>) -> Result<Value, String> {
    let text = match input {
        None => return Ok(json!({})),
        Some(at) if at.starts_with('@') => {
            let path = resolve(env, Path::new(&at[1..]));
            manifest::read_limited(&path, "input").map_err(|p| p.to_string())?
        }
        Some(text) => text.to_owned(),
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| format!("the input is not JSON ({e}); give an object, or @file"))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err("the input of a run is a JSON object".to_owned())
    }
}

pub(crate) async fn run(
    common: &Common,
    env: &Env,
    ask: &RunAsk<'_>,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let client = &session.client;
    let input = input_of(env, ask.input)?;
    let workflow = remote::workflow(client, ask.name).await?;
    let Some(active) = &workflow.active else {
        return Err(format!(
            "{} has no published version in {namespace}: publish it first (sagat workflow publish <file>)",
            ask.name
        ));
    };
    let key = ask.key.map_or_else(
        || crate::versions::nonce("run", &[&workflow.id]),
        str::to_owned,
    );
    let (_, started) = client
        .call(
            "POST",
            "/runs",
            Some(&json!({ "workflowId": workflow.id, "input": input })),
            Some(&key),
        )
        .await
        .map_err(|e| e.to_string())?;
    let run = started["runId"]
        .as_str()
        .ok_or("the platform's answer is not a run")?
        .to_owned();
    io.say(format!(
        "run {run} started: {} v{}",
        ask.name, active.number
    ));
    if !ask.wait {
        io.say(format!("follow it with: sagat workflow inspect {run}"));
        return Ok(EXIT_DONE);
    }
    let deadline = Instant::now() + Duration::from_secs(ask.timeout);
    let mut last = String::new();
    let mut wait = Duration::from_millis(250);
    loop {
        let (_, body) = client
            .call("GET", &format!("/runs/{run}"), None, None)
            .await
            .map_err(|e| e.to_string())?;
        let status = crate::one_line(body["status"].as_str().unwrap_or("?"));
        if status != last {
            io.say(format!("  {status}"));
            last = status.clone();
        }
        if FINAL.contains(&status.as_str()) {
            inspect_run(client, &run, io).await?;
            return Ok(if status == "SUCCEEDED" {
                EXIT_DONE
            } else {
                EXIT_FAILED
            });
        }
        // Waiting longer would change nothing: only a person moves these.
        if matches!(status.as_str(), "MANUAL_REVIEW" | "PAUSED") {
            io.warn(format!(
                "the run is {status}: it waits for a person (the Studio, or the operator actions of the API). It was not cancelled; follow it with sagat workflow inspect {run}"
            ));
            return Ok(EXIT_TIMEOUT);
        }
        if Instant::now() >= deadline {
            io.warn(format!(
                "the run is {status} after {} s: it may be waiting for an event, a timer, or a worker \
                 that serves its activities (sagat worker dev). It was not cancelled; follow it with sagat workflow inspect {run}",
                ask.timeout
            ));
            return Ok(EXIT_TIMEOUT);
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_secs(2));
    }
}

/// What `workflow run` was asked.
pub(crate) struct RunAsk<'a> {
    pub name: &'a str,
    pub input: Option<&'a str>,
    pub wait: bool,
    pub timeout: u64,
    pub key: Option<&'a str>,
}

pub(crate) async fn runs(
    common: &Common,
    env: &Env,
    name: &str,
    limit: u32,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let client = &session.client;
    let workflow = remote::workflow(client, name).await?;
    let (_, page) = client
        .call(
            "GET",
            &crate::client::with_query(
                "/runs",
                &[("workflowId", &workflow.id), ("limit", &limit.to_string())],
            ),
            None,
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
    let items = page["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        io.say(format!("{name} has no runs in {namespace}"));
        return Ok(EXIT_DONE);
    }
    for run in &items {
        let text = |key: &str| crate::one_line(run[key].as_str().unwrap_or("-"));
        io.say(format!(
            "{}  {:<12} started {}{}",
            text("runId"),
            text("status"),
            text("createdAt"),
            run["finishedAt"]
                .as_str()
                .map_or_else(String::new, |at| format!(
                    ", finished {}",
                    crate::one_line(at)
                ))
        ));
    }
    Ok(EXIT_DONE)
}

pub(crate) async fn inspect(
    common: &Common,
    env: &Env,
    run: &str,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    inspect_run(&session.client, run, io).await?;
    Ok(EXIT_DONE)
}

/// The status, the journal and the attempts of a run: what it did, in order, and where it failed.
/// No payload is shown (the journal holds references to them), so nothing a run handled is printed.
async fn inspect_run(client: &Client, run: &str, io: &mut Io<'_>) -> Result<(), String> {
    let (_, body) = client
        .call("GET", &format!("/runs/{run}"), None, None)
        .await
        .map_err(|e| e.to_string())?;
    let text = |key: &str| crate::one_line(body[key].as_str().unwrap_or("-"));
    io.say(format!(
        "run {run}  {}  workflow {}  version {}",
        text("status"),
        text("workflowId"),
        text("versionId")
    ));
    io.say(format!(
        "  started {} by {}{}",
        text("createdAt"),
        text("startedBy"),
        body["finishedAt"]
            .as_str()
            .map_or_else(String::new, |at| format!(
                ", finished {}",
                crate::one_line(at)
            ))
    ));
    if let Some(trace) = body["traceId"].as_str() {
        io.say(format!("  trace {}", crate::one_line(trace)));
    }
    let events = pages(client, &format!("/runs/{run}/events"), 1000).await?;
    io.say(format!("journal ({} entries):", events.len()));
    for event in &events {
        let mut line = format!(
            "  {:>4}  {}",
            event["seq"].as_u64().unwrap_or(0),
            crate::one_line(event["eventType"].as_str().unwrap_or("?"))
        );
        if let Some(step) = event["stepId"].as_str() {
            line.push_str(&format!("  {}", crate::one_line(step)));
            if let Some(attempt) = event["attempt"].as_u64() {
                line.push_str(&format!(" #{attempt}"));
            }
        }
        if let Some(code) = event["cause"]["code"].as_str() {
            line.push_str(&format!("  [{}]", crate::one_line(code)));
        }
        io.say(line);
    }
    let (_, attempts) = client
        .call("GET", &format!("/runs/{run}/attempts"), None, None)
        .await
        .map_err(|e| e.to_string())?;
    let items = attempts["items"].as_array().cloned().unwrap_or_default();
    if !items.is_empty() {
        io.say("attempts:");
        for attempt in &items {
            let mut line = format!(
                "  {}  #{}  {}",
                crate::one_line(attempt["node"].as_str().unwrap_or("?")),
                attempt["attempt"].as_u64().unwrap_or(0),
                crate::one_line(attempt["outcome"].as_str().unwrap_or("?"))
            );
            if let Some(code) = attempt["code"].as_str() {
                line.push_str(&format!("  [{}]", crate::one_line(code)));
            }
            io.say(line);
        }
    }
    Ok(())
}

/// Every item of a listing the run's reads give, at most `most` of them.
async fn pages(client: &Client, path: &str, most: usize) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut query = vec![("limit", "200")];
        if let Some(cursor) = &cursor {
            query.push(("cursor", cursor));
        }
        let (_, body) = client
            .call("GET", &crate::client::with_query(path, &query), None, None)
            .await
            .map_err(|e| e.to_string())?;
        items.extend(body["items"].as_array().cloned().unwrap_or_default());
        match body["nextCursor"].as_str() {
            Some(next) if items.len() < most => cursor = Some(next.to_owned()),
            _ => break,
        }
    }
    items.truncate(most);
    Ok(items)
}

pub(crate) async fn cancel(
    common: &Common,
    env: &Env,
    run: &str,
    reason: &str,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let key = crate::versions::nonce("cancel", &[run]);
    match session
        .client
        .call(
            "POST",
            &format!("/runs/{run}/operations"),
            Some(&json!({ "action": "CANCEL", "reason": reason })),
            Some(&key),
        )
        .await
    {
        Ok((_, answer)) => {
            io.say(format!(
                "cancel of run {run} accepted: the run is {}",
                answer["runStatus"].as_str().unwrap_or("?")
            ));
            Ok(EXIT_DONE)
        }
        Err(CallError::Refused(failure)) if failure.status == 409 => {
            io.warn(format!("the run could not be cancelled: {failure}"));
            Ok(EXIT_FAILED)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Starts a new run of the same version, with the same input, from a run that ended. The answer
/// names the new run; a run that has not ended is refused by the server (409) and nothing starts.
pub(crate) async fn rerun(
    common: &Common,
    env: &Env,
    run: &str,
    reason: &str,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let key = crate::versions::nonce("rerun", &[run]);
    match session
        .client
        .call(
            "POST",
            &format!("/runs/{run}/operations"),
            Some(&json!({ "action": "RERUN_AS_NEW", "reason": reason })),
            Some(&key),
        )
        .await
    {
        Ok((_, answer)) => {
            io.say(format!(
                "run {run} was started again as {}",
                answer["newRunId"].as_str().unwrap_or("?")
            ));
            Ok(EXIT_DONE)
        }
        Err(CallError::Refused(failure)) if failure.status == 409 => {
            io.warn(format!("the run could not be started again: {failure}"));
            Ok(EXIT_FAILED)
        }
        Err(e) => Err(e.to_string()),
    }
}
