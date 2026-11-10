//! `sagat batch …`: one action over many runs (feature 018). The selector, the action and what each run
//! allows are the platform's; this tool builds the request, shows the answer in words and never invents a count.

use serde_json::{Value, json};

use crate::args::{BatchCommand, BatchRequest, Common};
use crate::client::{CallError, Client};
use crate::{EXIT_DONE, EXIT_FAILED, Env, Io, namespace_of, remote, resolve, session};

fn refusal(io: &mut Io<'_>, what: &str, error: CallError) -> Result<u8, String> {
    match error {
        CallError::Refused(failure) if matches!(failure.status, 403 | 404 | 409 | 422 | 429) => {
            io.warn(format!("{what}: {failure}"));
            Ok(EXIT_FAILED)
        }
        other => Err(other.to_string()),
    }
}

fn text(value: &Value, key: &str) -> String {
    crate::one_line(value[key].as_str().unwrap_or("-"))
}

/// JSON from text or from `@path`.
fn json_arg(env: &Env, argument: &str, what: &str) -> Result<Value, String> {
    let text = if let Some(file) = argument.strip_prefix('@') {
        let path = resolve(env, std::path::Path::new(file));
        crate::manifest::read_limited(&path, what).map_err(|p| p.to_string())?
    } else {
        argument.to_owned()
    };
    serde_json::from_str(&text)
        .map_err(|e| format!("the {what} is not JSON ({e}); give JSON, or @file"))
}

/// The body of a creation from what was asked.
fn creation(env: &Env, ask: &BatchRequest) -> Result<Value, String> {
    let mut body = json!({
        "selector": json_arg(env, &ask.selector, "selector")?,
        "action": ask.action,
        "reason": ask.reason,
    });
    if let Some(max) = ask.max_items {
        body["maxItems"] = json!(max);
    }
    if let Some(rate) = &ask.rate {
        let rate: f64 = rate
            .parse()
            .map_err(|_| "--rate is a number of items per second".to_owned())?;
        body["itemsPerSecond"] = json!(rate);
    }
    if let Some(name) = &ask.signal {
        let payload = ask
            .payload
            .as_deref()
            .map_or(Ok(json!({})), |p| json_arg(env, p, "payload"))?;
        if !payload.is_object() {
            return Err("the payload of a signal is a JSON object".to_owned());
        }
        body["signal"] = json!({ "name": name, "payload": payload });
    }
    if let Some(name) = &ask.update {
        let args = ask
            .args
            .as_deref()
            .map_or(Ok(json!({})), |a| json_arg(env, a, "arguments"))?;
        body["update"] = json!({ "name": name, "args": args });
    }
    if let Some(step) = &ask.step_id {
        body["stepId"] = json!(step);
    }
    if ask.drop_business_id {
        body["businessId"] = crate::reset::dropped_business_id();
    }
    Ok(body)
}

fn counts(job: &Value) -> String {
    let c = &job["counts"];
    let n = |key: &str| c[key].as_u64().unwrap_or(0);
    format!(
        "{} selected: {} succeeded, {} skipped, {} failed, {} cancelled, {} deferred",
        n("selected"),
        n("succeeded"),
        n("skipped"),
        n("failed"),
        n("cancelled"),
        n("deferred")
    )
}

pub(crate) async fn run(
    common: &Common,
    env: &Env,
    command: &BatchCommand,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let client = &session.client;
    let base = format!(
        "/namespaces/{}/batch-operations",
        remote::namespace_id(client, &namespace).await?
    );
    match command {
        BatchCommand::Preview {
            selector,
            max_items,
        } => {
            let mut body = json!({ "selector": json_arg(env, selector, "selector")? });
            if let Some(max) = max_items {
                body["maxItems"] = json!(max);
            }
            match client
                .call("POST", &format!("{base}/preview"), Some(&body), None)
                .await
            {
                Ok((_, preview)) => {
                    io.say(format!(
                        "{} run(s) match{}",
                        preview["matched"].as_u64().unwrap_or(0),
                        if preview["capped"].as_bool() == Some(true) {
                            " (more than the limit: the job would take the first ones)"
                        } else {
                            ""
                        }
                    ));
                    for run in preview["sample"].as_array().into_iter().flatten() {
                        io.say(format!(
                            "  {}",
                            crate::one_line(run.as_str().unwrap_or("?"))
                        ));
                    }
                    Ok(EXIT_DONE)
                }
                Err(error) => refusal(io, "the selection could not be previewed", error),
            }
        }
        BatchCommand::Create(ask) => {
            let body = creation(env, ask)?;
            let key = ask.key.clone().unwrap_or_else(|| {
                crate::versions::nonce("batch-create", &[&base, &ask.action, &ask.reason])
            });
            match client.call("POST", &base, Some(&body), Some(&key)).await {
                Ok((_, job)) => {
                    io.say(format!(
                        "batch job {} created: {} ({})",
                        text(&job, "batchId"),
                        text(&job, "action"),
                        text(&job, "state")
                    ));
                    io.say(format!(
                        "follow it with: sagat batch get {}",
                        text(&job, "batchId")
                    ));
                    Ok(EXIT_DONE)
                }
                Err(error) => refusal(io, "the batch job was not created", error),
            }
        }
        BatchCommand::Get { id } => match client
            .call("GET", &format!("{base}/{id}"), None, None)
            .await
        {
            Ok((_, job)) => {
                io.say(format!(
                    "batch job {id}  {}  {}",
                    text(&job, "action"),
                    text(&job, "state")
                ));
                io.say(format!("  {}", counts(&job)));
                if let Some(reason) = job["failureReason"].as_str() {
                    io.say(format!("  failed: {}", crate::one_line(reason)));
                }
                Ok(EXIT_DONE)
            }
            Err(error) => refusal(io, "the batch job could not be read", error),
        },
        BatchCommand::List { state, limit } => {
            list(client, &base, state.as_deref(), *limit, io).await
        }
        BatchCommand::Items { id, state, limit } => {
            items(client, &base, id, state.as_deref(), *limit, io).await
        }
        BatchCommand::Cancel { id, reason, key } => {
            let key = key
                .clone()
                .unwrap_or_else(|| crate::versions::nonce("batch-cancel", &[id, reason]));
            // the contract (`cancelBatchOperation`) asks for the reason in a JSON body; a POST without one is a 400
            let body = json!({ "reason": reason });
            match client
                .call(
                    "POST",
                    &format!("{base}/{id}/cancel"),
                    Some(&body),
                    Some(&key),
                )
                .await
            {
                Ok((_, job)) => {
                    io.say(format!(
                        "batch job {id} is {}: what is in flight finishes, the rest is cancelled",
                        text(&job, "state")
                    ));
                    Ok(EXIT_DONE)
                }
                Err(error) => refusal(io, "the batch job was not cancelled", error),
            }
        }
    }
}

async fn list(
    client: &Client,
    base: &str,
    state: Option<&str>,
    limit: u32,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let mut query = vec![("limit", limit.to_string())];
    if let Some(state) = state {
        query.push(("state", state.to_owned()));
    }
    let query: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let page = match client
        .call("GET", &crate::client::with_query(base, &query), None, None)
        .await
    {
        Ok((_, page)) => page,
        Err(error) => return refusal(io, "the batch jobs could not be read", error),
    };
    let items = page["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        io.say("there are no batch jobs");
        return Ok(EXIT_DONE);
    }
    for job in &items {
        io.say(format!(
            "{}  {:<18} {:<10} {}",
            text(job, "batchId"),
            text(job, "action"),
            text(job, "state"),
            counts(job)
        ));
    }
    Ok(EXIT_DONE)
}

async fn items(
    client: &Client,
    base: &str,
    id: &str,
    state: Option<&str>,
    limit: u32,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let mut query = vec![("limit", limit.to_string())];
    if let Some(state) = state {
        query.push(("state", state.to_owned()));
    }
    let query: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let page = match client
        .call(
            "GET",
            &crate::client::with_query(&format!("{base}/{id}/items"), &query),
            None,
            None,
        )
        .await
    {
        Ok((_, page)) => page,
        Err(error) => return refusal(io, "the items could not be read", error),
    };
    let items = page["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        io.say(format!(
            "batch job {id} has no items{}",
            if state.is_some() {
                " in that state"
            } else {
                ""
            }
        ));
        return Ok(EXIT_DONE);
    }
    for item in &items {
        io.say(format!(
            "{:>6}  {}  {:<10} {}",
            item["ordinal"].as_u64().unwrap_or(0),
            text(item, "runId"),
            text(item, "state"),
            item["reason"]
                .as_str()
                .or_else(|| item["errorCode"].as_str())
                .map_or_else(String::new, crate::one_line)
        ));
    }
    Ok(EXIT_DONE)
}
