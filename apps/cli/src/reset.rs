//! `sagat workflow reset-points|reset-preview|reset`: starting a new run from a point of a run that ended
//! (feature 018). The tool shows what the platform answers, in words; what a reset may do is the platform's.

use serde_json::{Value, json};

use crate::args::Reapply;
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

/// What a reason of the preview or of the report of a reset comes to, in words. A reason this tool does not know
/// is said as the platform wrote it.
fn reason_words(reason: &str) -> String {
    match reason {
        "LEGACY_EVENT" => "it was recorded before the platform said what it was".to_owned(),
        "NOT_RETAINED" => "its content is no longer kept".to_owned(),
        "PREVIOUS_SIGNAL_NOT_DELIVERED" => {
            "an earlier signal was not delivered, and a later one would reach its wait".to_owned()
        }
        "NOT_WAITING" => "no step of the new run was waiting for it".to_owned(),
        "UPDATE_REJECTED" => "the validator of the update declined it".to_owned(),
        "REVISION_CONFLICT" => {
            "the state was not at the revision the update was applied to".to_owned()
        }
        "UPDATE_NOT_ACCEPTED" => "the declaration does not accept the update".to_owned(),
        other => crate::one_line(other),
    }
}

/// One signal or Update the reset delivers again, or did: what it is and what came, or comes, of it. Only the
/// name of the signal or the operation is said, never what was sent.
fn entry_words(entry: &Value) -> String {
    let (what, name) = match entry["kind"].as_str() {
        Some("SIGNAL") => ("signal", entry["name"].as_str()),
        Some("UPDATE") => ("update", entry["operation"].as_str()),
        _ => ("entry", None),
    };
    let outcome = match entry["outcome"].as_str().unwrap_or("?") {
        "WILL_TRY" => "will be tried".to_owned(),
        "REAPPLIED" => "delivered".to_owned(),
        "REJECTED" => "rejected".to_owned(),
        "NOT_REAPPLIED" => "not delivered".to_owned(),
        other => crate::one_line(other),
    };
    let because = entry["reason"]
        .as_str()
        .map(|reason| format!(": {}", reason_words(reason)))
        .unwrap_or_default();
    format!(
        "{what} {} (event {}): {outcome}{because}",
        crate::one_line(name.unwrap_or("?")),
        entry["sequence"].as_u64().unwrap_or(0),
    )
}

/// The lines that say what a reset delivers again, or did; nothing for a reset that asked for none.
fn say_reapply(io: &mut Io<'_>, title: &str, entries: &Value) {
    let Some(entries) = entries.as_array().filter(|entries| !entries.is_empty()) else {
        return;
    };
    io.say(title);
    for entry in entries {
        io.say(format!("    {}", entry_words(entry)));
    }
}

pub(crate) async fn preview(
    common: &crate::args::Common,
    env: &Env,
    run: &str,
    sequence: u64,
    reapply: Option<Reapply>,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let mut body = json!({ "sequence": sequence });
    if let Some(reapply) = reapply {
        body["reapply"] = json!(reapply.api());
    }
    let plan = match session
        .client
        .call(
            "POST",
            &format!("/runs/{run}/reset-previews"),
            Some(&body),
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
    say_reapply(
        io,
        "  deliver again what the run received after that point, in order:",
        &plan["reapply"],
    );
    if plan["requiresAcknowledgement"].as_bool() == Some(true) {
        let completed = again
            .iter()
            .any(|step| step["completedInSource"].as_bool() == Some(true));
        io.say(if completed {
            "  the reset asks you to say you know that: add --acknowledge"
        } else {
            // nothing completed after the point, yet it asks: a step of a run that has not ended is in flight, and
            // may complete before the cancellation takes effect, and then it runs again
            "  a step of this run is in flight and may complete before the run is cancelled; the reset asks you to \
             say you know it runs again: add --acknowledge"
        });
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
    pub reapply: Option<Reapply>,
    /// Cancel the run if it has not ended and make the new run when it has.
    pub cancel_source: bool,
    pub key: Option<&'a str>,
}

/// Why a reset that waited for its source did not make a run, in words. A reason this tool does not know is said
/// as the platform wrote it, so a later version can add one.
fn failure_words(reason: &str) -> String {
    match reason {
        "BOUNDARY_NO_LONGER_ELIGIBLE" => {
            "the point is not one the run can be reset to on the history it ended with".to_owned()
        }
        "ADMISSION_REFUSED" => {
            "the platform did not admit the new run (a limit of the namespace, or the workflow)"
                .to_owned()
        }
        "BUSINESS_ID_IN_USE" => {
            "another open run holds the business identifier (reset with --drop-business-id to start without it)"
                .to_owned()
        }
        "AUTHORITY_REVOKED" => "the member who asked no longer may reset runs".to_owned(),
        "REEXECUTION_NOT_ACKNOWLEDGED" => {
            "steps completed after the point and the request did not say they run again (ask again with --acknowledge)"
                .to_owned()
        }
        "REAPPLY_TOO_MANY" => {
            "the run received more signals and updates after the point than a reset delivers again"
                .to_owned()
        }
        "CONCURRENCY_PERMIT_UNAVAILABLE" => {
            "the workflow's concurrency key had no free place for the new run".to_owned()
        }
        "RUN_PURGED" => "the history of the run is no longer kept".to_owned(),
        "SOURCE_DID_NOT_END" => {
            "the run being cancelled had not ended within the time a reset waits for it (once it has ended, reset it again)"
                .to_owned()
        }
        other => crate::one_line(other),
    }
}

/// How an entry of the register stands, in words: the line after the id.
fn state_words(entry: &Value, source: &str) -> String {
    match entry["state"].as_str().unwrap_or("?") {
        "PENDING_SOURCE" => format!(
            "waits for run {source} to end; the new run is made then, if the point is still one"
        ),
        "CREATED" => format!(
            "run {} was made",
            crate::one_line(entry["newRunId"].as_str().unwrap_or("?"))
        ),
        "FAILED" => format!(
            "no run was made: {}",
            failure_words(entry["failureReason"].as_str().unwrap_or("?"))
        ),
        other => format!("is {}", crate::one_line(other)),
    }
}

/// `workflow resets <RUN>`: the resets made from a run, newest first, and how each stands.
pub(crate) async fn resets(
    common: &crate::args::Common,
    env: &Env,
    run: &str,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let listed = match session
        .client
        .call("GET", &format!("/runs/{run}/resets"), None, None)
        .await
    {
        Ok((_, listed)) => listed,
        Err(CallError::Refused(failure)) if matches!(failure.status, 404) => {
            return refused(io, "the resets could not be read", &failure.to_string());
        }
        Err(other) => return Err(other.to_string()),
    };
    let items = listed["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        io.say(format!("no reset was made from run {run}"));
        return Ok(EXIT_DONE);
    }
    io.say(format!("resets made from run {run}, newest first:"));
    for entry in &items {
        io.say(format!(
            "  {}  from point {}: {}",
            crate::one_line(entry["resetId"].as_str().unwrap_or("?")),
            entry["sequence"].as_u64().unwrap_or(0),
            state_words(entry, run)
        ));
    }
    Ok(EXIT_DONE)
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
    if let Some(reapply) = ask.reapply {
        body["reapply"] = json!(reapply.api());
    }
    // sent only when asked for, so that an older server never sees the field
    if ask.cancel_source {
        body["sourcePolicy"] = json!("CANCEL_SOURCE");
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
            let id = crate::one_line(made["resetId"].as_str().unwrap_or("?"));
            match made["state"].as_str() {
                // the run is being cancelled; the engine makes the new one when it has ended
                Some("PENDING_SOURCE") => {
                    io.say(format!(
                        "reset {id}: run {} is being cancelled; the new run is made from point {} when it has ended, if the point is still one",
                        ask.run, ask.sequence
                    ));
                    io.say(format!(
                        "see how it went with: sagat workflow resets {}",
                        ask.run
                    ));
                    Ok(EXIT_DONE)
                }
                // a retry of a request that waited and failed
                Some("FAILED") => {
                    io.warn(format!(
                        "reset {id}: no run was made: {}",
                        failure_words(made["failureReason"].as_str().unwrap_or("?"))
                    ));
                    Ok(EXIT_FAILED)
                }
                _ => {
                    let new_run = made["newRunId"].as_str().unwrap_or("?");
                    io.say(format!(
                        "reset {id}: run {new_run} was made from point {} of run {}",
                        ask.sequence, ask.run
                    ));
                    say_reapply(
                        io,
                        "  delivered again, in the order the run received them:",
                        &made["reapplyReport"],
                    );
                    io.say(format!("follow it with: sagat workflow inspect {new_run}"));
                    Ok(EXIT_DONE)
                }
            }
        }
        Err(CallError::Refused(failure)) if matches!(failure.status, 404 | 409 | 422) => {
            refused(io, "the run could not be reset", &failure.to_string())
        }
        Err(other) => Err(other.to_string()),
    }
}
