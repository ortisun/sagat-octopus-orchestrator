//! `sagat workflow signal-with-start|update-with-start`: acting on the open run of a business identifier, or
//! starting one and acting on it, in one request (feature 018).

use serde_json::{Value, json};

use crate::args::{Common, IfExists, Reuse};
use crate::client::CallError;
use crate::{EXIT_DONE, EXIT_FAILED, Env, Io, namespace_of, remote, resolve, session};

/// A JSON value from text or from `@path`.
fn json_of(env: &Env, text: Option<&str>, what: &str, default: Value) -> Result<Value, String> {
    let text = match text {
        None => return Ok(default),
        Some(at) if at.starts_with('@') => {
            let path = resolve(env, std::path::Path::new(&at[1..]));
            crate::manifest::read_limited(&path, what).map_err(|p| p.to_string())?
        }
        Some(text) => text.to_owned(),
    };
    serde_json::from_str(&text)
        .map_err(|e| format!("the {what} is not JSON ({e}); give JSON, or @file"))
}

/// What `workflow signal-with-start` was asked.
pub(crate) struct SignalAsk<'a> {
    pub name: &'a str,
    pub business_id: &'a str,
    pub signal: &'a str,
    pub payload: Option<&'a str>,
    pub input: Option<&'a str>,
    pub reuse: Option<Reuse>,
    pub key: Option<&'a str>,
}

pub(crate) async fn signal_with_start(
    common: &Common,
    env: &Env,
    ask: &SignalAsk<'_>,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let input = crate::workflow::input_of(env, ask.input)?;
    let payload = json_of(env, ask.payload, "payload", json!({}))?;
    if !payload.is_object() {
        return Err("the payload of a signal is a JSON object".to_owned());
    }
    let workflow = remote::workflow(&session.client, ask.name).await?;
    let key = ask.key.map_or_else(
        || crate::versions::nonce("signal-with-start", &[&workflow.id, ask.business_id]),
        str::to_owned,
    );
    let mut body = json!({
        "workflowId": workflow.id,
        "businessId": ask.business_id,
        "input": input,
        "signal": { "name": ask.signal, "payload": payload },
    });
    if let Some(policy) = ask.reuse {
        body["businessIdReusePolicy"] = json!(policy.api());
    }
    match session
        .client
        .call("POST", "/runs/signal-with-start", Some(&body), Some(&key))
        .await
    {
        Ok((_, result)) => {
            let run = result["runId"].as_str().unwrap_or("?");
            let woke = result["deliveredTo"].as_array().map_or(0, Vec::len);
            io.say(if result["started"].as_bool() == Some(true) {
                format!("run {run} was started and signalled: {woke} step(s) woke")
            } else {
                format!("run {run} was already open and was signalled: {woke} step(s) woke")
            });
            Ok(EXIT_DONE)
        }
        Err(CallError::Refused(failure)) if matches!(failure.status, 409 | 422) => {
            io.warn(format!("nothing was signalled: {failure}"));
            Ok(EXIT_FAILED)
        }
        Err(other) => Err(other.to_string()),
    }
}

/// What `workflow update-with-start` was asked.
pub(crate) struct UpdateAsk<'a> {
    pub name: &'a str,
    pub business_id: &'a str,
    pub update: &'a str,
    pub update_key: &'a str,
    pub args: Option<&'a str>,
    pub input: Option<&'a str>,
    pub if_exists: Option<IfExists>,
    pub reuse: Option<Reuse>,
    pub start_delay_secs: Option<u64>,
    pub expected_revision: Option<u64>,
    pub key: Option<&'a str>,
}

pub(crate) async fn update_with_start(
    common: &Common,
    env: &Env,
    ask: &UpdateAsk<'_>,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let input = crate::workflow::input_of(env, ask.input)?;
    let args = json_of(env, ask.args, "arguments", json!({}))?;
    let workflow = remote::workflow(&session.client, ask.name).await?;
    let key = ask.key.map_or_else(
        || crate::versions::nonce("update-with-start", &[&workflow.id, ask.business_id]),
        str::to_owned,
    );
    let mut update = json!({ "name": ask.update, "updateKey": ask.update_key, "args": args });
    if let Some(revision) = ask.expected_revision {
        update["expectedStateRevision"] = json!(revision);
    }
    let mut body = json!({
        "workflowId": workflow.id,
        "businessId": ask.business_id,
        "input": input,
        "update": update,
    });
    if let Some(policy) = ask.if_exists {
        body["businessIdConflictPolicy"] = json!(policy.api());
    }
    if let Some(policy) = ask.reuse {
        body["businessIdReusePolicy"] = json!(policy.api());
    }
    if let Some(seconds) = ask.start_delay_secs {
        body["startDelayMs"] = json!(seconds.saturating_mul(1000));
    }
    match session
        .client
        .call("POST", "/runs/update-with-start", Some(&body), Some(&key))
        .await
    {
        Ok((_, result)) => {
            let run = result["runId"].as_str().unwrap_or("?");
            let which = if result["started"].as_bool() == Some(true) {
                "was started"
            } else {
                "was already open"
            };
            if result["update"]["outcome"].as_str() == Some("APPLIED") {
                io.say(format!("run {run} {which} and the update was applied"));
                Ok(EXIT_DONE)
            } else {
                io.warn(format!(
                    "run {run} {which} but the update was rejected: {}",
                    crate::one_line(result["update"]["reason"].as_str().unwrap_or("?"))
                ));
                Ok(EXIT_FAILED)
            }
        }
        Err(CallError::Refused(failure)) if matches!(failure.status, 404 | 409 | 422) => {
            io.warn(format!("nothing was updated: {failure}"));
            Ok(EXIT_FAILED)
        }
        Err(other) => Err(other.to_string()),
    }
}
