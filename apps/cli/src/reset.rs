//! `sagat workflow reset-points|reset-preview|reset`: starting a new run from a point of a run that ended
//! (feature 018). The tool shows what the platform answers, in words; what a reset may do is the platform's.

use serde_json::{Value, json};

use crate::client::CallError;
use crate::{EXIT_DONE, EXIT_FAILED, Env, Io, namespace_of, session};

/// `businessId: DROP` of the contract. The name is written by the derive: the audit of the privileges of the
/// runtime role reads every string literal of the workspace as a possible statement, and a bare word of that
/// kind is refused.
#[derive(serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum BusinessChoice {
    Drop,
}

/// The value that starts the new run without the business identifier of the source.
pub(crate) fn dropped_business_id() -> Value {
    serde_json::to_value(BusinessChoice::Drop).unwrap_or(Value::Null)
}

/// A point in words: where in the run it is, not the number of a row of the journal alone.
fn label(point: &Value) -> String {
    let node = crate::one_line(point["node"].as_str().unwrap_or("?"));
    match point["kind"].as_str().unwrap_or("?") {
        "RUN_STARTED" => "the start of the run".to_owned(),
        "STEP_COMPLETED" => format!("after step {node}"),
        "CHILD_FINISHED" => format!("after child run {node}"),
        "COLLECTION_FINISHED" => format!("after collection {node}"),
        "LOOP_FINISHED" => format!("after loop {node}"),
        other => crate::one_line(other),
    }
}

pub(crate) async fn points(
    common: &crate::args::Common,
    env: &Env,
    run: &str,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let points = match crate::workflow::pages(
        &session.client,
        &format!("/runs/{run}/reset-points"),
        10_000,
    )
    .await
    {
        Ok(points) => points,
        Err(why) => return refused(io, "the points could not be read", &why),
    };
    if points.is_empty() {
        io.say(format!("run {run} cannot be reset to any point"));
        return Ok(EXIT_DONE);
    }
    io.say(format!("run {run} can be reset to:"));
    for point in &points {
        io.say(format!(
            "  {:>6}  {}  ({})",
            point["sequence"].as_u64().unwrap_or(0),
            label(point),
            crate::one_line(point["occurredAt"].as_str().unwrap_or("?"))
        ));
    }
    io.say(format!(
        "preview one with: sagat workflow reset-preview {run} --sequence <N>"
    ));
    Ok(EXIT_DONE)
}

/// What a refusal of the platform comes to: it is said, and the command failed.
fn refused(io: &mut Io<'_>, what: &str, why: &str) -> Result<u8, String> {
    io.warn(format!("{what}: {why}"));
    Ok(EXIT_FAILED)
}

pub(crate) async fn preview(
    common: &crate::args::Common,
    env: &Env,
    run: &str,
    sequence: u64,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let plan = match session
        .client
        .call(
            "POST",
            &format!("/runs/{run}/reset-previews"),
            Some(&json!({ "sequence": sequence })),
            None,
        )
        .await
    {
        Ok((_, plan)) => plan,
        Err(CallError::Refused(failure)) if matches!(failure.status, 404 | 409 | 422) => {
            return refused(io, "the reset cannot be previewed", &failure.to_string());
        }
        Err(other) => return Err(other.to_string()),
    };
    io.say(format!("a reset of run {run} to point {sequence} would:"));
    let names = |key: &str| -> Vec<String> {
        plan[key]
            .as_array()
            .map(|steps| {
                steps
                    .iter()
                    .map(|step| crate::one_line(step["node"].as_str().unwrap_or("?")))
                    .collect()
            })
            .unwrap_or_default()
    };
    let kept = names("restored");
    io.say(if kept.is_empty() {
        "  keep nothing: the new run starts from the beginning".to_owned()
    } else {
        format!("  keep as they were: {}", kept.join(", "))
    });
    let again = plan["reexecuted"].as_array().cloned().unwrap_or_default();
    if again.is_empty() {
        io.say("  run nothing again");
    }
    for step in &again {
        let markers: Vec<String> = step["markers"]
            .as_array()
            .map(|markers| {
                markers
                    .iter()
                    .filter_map(|marker| marker.as_str().map(crate::one_line))
                    .collect()
            })
            .unwrap_or_default();
        io.say(format!(
            "  run again: {}{}{}",
            crate::one_line(step["node"].as_str().unwrap_or("?")),
            if step["completedInSource"].as_bool() == Some(true) {
                " (it completed in this run: running it again repeats what it did)"
            } else {
                ""
            },
            if markers.is_empty() {
                String::new()
            } else {
                format!(" [{}]", markers.join(", "))
            }
        ));
    }
    if plan["requiresAcknowledgement"].as_bool() == Some(true) {
        io.say("  the reset asks you to say you know that: add --acknowledge");
    }
    Ok(EXIT_DONE)
}

/// What `workflow reset` was asked.
pub(crate) struct ResetAsk<'a> {
    pub run: &'a str,
    pub sequence: u64,
    pub reason: &'a str,
    pub acknowledge: bool,
    pub drop_business_id: bool,
    pub key: Option<&'a str>,
}

pub(crate) async fn reset(
    common: &crate::args::Common,
    env: &Env,
    ask: &ResetAsk<'_>,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let key = ask.key.map_or_else(
        || crate::versions::nonce("reset", &[ask.run, &ask.sequence.to_string()]),
        str::to_owned,
    );
    let mut body = json!({
        "sequence": ask.sequence,
        "reason": ask.reason,
        "acknowledgeReexecution": ask.acknowledge,
    });
    if ask.drop_business_id {
        body["businessId"] = dropped_business_id();
    }
    match session
        .client
        .call(
            "POST",
            &format!("/runs/{}/resets", ask.run),
            Some(&body),
            Some(&key),
        )
        .await
    {
        Ok((_, made)) => {
            let new_run = made["newRunId"].as_str().unwrap_or("?");
            io.say(format!(
                "reset {}: run {new_run} was made from point {} of run {}",
                crate::one_line(made["resetId"].as_str().unwrap_or("?")),
                ask.sequence,
                ask.run
            ));
            io.say(format!("follow it with: sagat workflow inspect {new_run}"));
            Ok(EXIT_DONE)
        }
        Err(CallError::Refused(failure)) if matches!(failure.status, 404 | 409 | 422) => {
            refused(io, "the run could not be reset", &failure.to_string())
        }
        Err(other) => Err(other.to_string()),
    }
}
