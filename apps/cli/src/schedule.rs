//! `sagat schedule …`: the schedules of a namespace (feature 018). Each command is one request of the REST API and
//! its answer in words; whether a schedule is valid, and what happens at a time, is the platform's to say.

use serde_json::{Value, json};

use crate::args::{ActionBusiness, Common, ScheduleCommand};
use crate::client::{CallError, Client};
use crate::{EXIT_DONE, EXIT_FAILED, Env, Io, namespace_of, remote, resolve, session};

/// What the platform refuses is said and the command fails; anything else is an error of the tool.
fn refusal(io: &mut Io<'_>, what: &str, error: CallError) -> Result<u8, String> {
    match error {
        CallError::Refused(failure) if matches!(failure.status, 404 | 409 | 422 | 429) => {
            io.warn(format!("{what}: {failure}"));
            Ok(EXIT_FAILED)
        }
        other => Err(other.to_string()),
    }
}

fn text(value: &Value, key: &str) -> String {
    crate::one_line(value[key].as_str().unwrap_or("-"))
}

/// A key for a request that changes something: the one given, or one made for this call.
fn key_of(given: Option<&String>, kind: &str, parts: &[&str]) -> String {
    given
        .cloned()
        .unwrap_or_else(|| crate::versions::nonce(kind, parts))
}

/// The document of a schedule from a file: JSON, the shape of the contract.
fn document(env: &Env, file: &std::path::Path) -> Result<Value, String> {
    let path = resolve(env, file);
    let text = crate::manifest::read_limited(&path, "schedule").map_err(|p| p.to_string())?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| format!("the schedule is not JSON ({e}): write it as the contract says"))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err("a schedule is a JSON object".to_owned())
    }
}

/// The document with the business id of the action the flags ask for: what they give goes over what the file says. The platform
/// decides whether the prefix is valid.
fn with_business(mut body: Value, business: &ActionBusiness) -> Result<Value, String> {
    if business.prefix.is_none() && business.if_exists.is_none() && business.reuse.is_none() {
        return Ok(body);
    }
    let Some(action) = body.get_mut("action").and_then(Value::as_object_mut) else {
        return Err(
            "the schedule has no action to put the business id prefix in: write it as the contract says"
                .to_owned(),
        );
    };
    match &business.prefix {
        Some(prefix) => {
            action.insert("businessIdPrefix".to_owned(), json!(prefix));
        }
        // a policy with no prefix of its own goes with the one the file names; with none it means nothing
        None if !action.contains_key("businessIdPrefix") => {
            return Err(
                "--if-exists and --reuse go with a business id prefix: give --business-id-prefix, or write one in the action of the file"
                    .to_owned(),
            );
        }
        None => {}
    }
    if let Some(policy) = business.if_exists {
        action.insert("businessIdConflictPolicy".to_owned(), json!(policy.api()));
    }
    if let Some(policy) = business.reuse {
        action.insert("businessIdReusePolicy".to_owned(), json!(policy.api()));
    }
    Ok(body)
}

pub(crate) async fn run(
    common: &Common,
    env: &Env,
    command: &ScheduleCommand,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let client = &session.client;
    let base = format!(
        "/namespaces/{}/schedules",
        remote::namespace_id(client, &namespace).await?
    );
    match command {
        ScheduleCommand::List { state, limit } => {
            list(client, &base, &namespace, state.as_deref(), *limit, io).await
        }
        ScheduleCommand::Get { id } => get(client, &base, id, io).await,
        ScheduleCommand::Create {
            file,
            key,
            business,
        } => {
            let body = with_business(document(env, file)?, business)?;
            let key = key_of(key.as_ref(), "schedule-create", &[&base]);
            match client.call("POST", &base, Some(&body), Some(&key)).await {
                Ok((_, made)) => {
                    io.say(format!(
                        "schedule {} created: {} (revision {})",
                        text(&made, "scheduleId"),
                        text(&made, "name"),
                        made["revision"].as_u64().unwrap_or(0)
                    ));
                    Ok(EXIT_DONE)
                }
                Err(error) => refusal(io, "the schedule was not created", error),
            }
        }
        ScheduleCommand::Update {
            id,
            file,
            revision,
            key,
            business,
        } => {
            let mut body = with_business(document(env, file)?, business)?;
            body["expectedRevision"] = json!(revision);
            let key = key_of(
                key.as_ref(),
                "schedule-update",
                &[id, &revision.to_string()],
            );
            match client
                .call("PUT", &format!("{base}/{id}"), Some(&body), Some(&key))
                .await
            {
                Ok((_, made)) => {
                    io.say(format!(
                        "schedule {id} updated: revision {}",
                        made["revision"].as_u64().unwrap_or(0)
                    ));
                    Ok(EXIT_DONE)
                }
                Err(error) => refusal(io, "the schedule was not updated", error),
            }
        }
        ScheduleCommand::Delete { id, revision, key } => {
            let key = key_of(
                key.as_ref(),
                "schedule-delete",
                &[id, &revision.to_string()],
            );
            let path = crate::client::with_query(
                &format!("{base}/{id}"),
                &[("expectedRevision", &revision.to_string())],
            );
            match client.call("DELETE", &path, None, Some(&key)).await {
                Ok(_) => {
                    io.say(format!("schedule {id} deleted"));
                    Ok(EXIT_DONE)
                }
                Err(error) => refusal(io, "the schedule was not deleted", error),
            }
        }
        ScheduleCommand::Pause { id, note, key } | ScheduleCommand::Resume { id, note, key } => {
            let (verb, kind) = if matches!(command, ScheduleCommand::Pause { .. }) {
                ("pause", "schedule-pause")
            } else {
                ("resume", "schedule-resume")
            };
            let key = key_of(key.as_ref(), kind, &[id, note]);
            match client
                .call(
                    "POST",
                    &format!("{base}/{id}/{verb}"),
                    Some(&json!({ "note": note })),
                    Some(&key),
                )
                .await
            {
                Ok((_, made)) => {
                    io.say(format!("schedule {id} is now {}", text(&made, "state")));
                    Ok(EXIT_DONE)
                }
                Err(error) => refusal(io, &format!("the schedule was not {verb}d"), error),
            }
        }
        ScheduleCommand::Trigger { id, overlap, key } => {
            let body = overlap
                .as_ref()
                .map_or_else(|| json!({}), |policy| json!({ "overlap": policy }));
            let key = key_of(key.as_ref(), "schedule-trigger", &[id]);
            match client
                .call(
                    "POST",
                    &format!("{base}/{id}/trigger"),
                    Some(&body),
                    Some(&key),
                )
                .await
            {
                Ok((_, action)) => {
                    io.say(format!(
                        "schedule {id} was triggered: {}{}",
                        text(&action, "outcome"),
                        action["runId"]
                            .as_str()
                            .map_or_else(String::new, |run| format!(
                                ", run {}",
                                crate::one_line(run)
                            ))
                    ));
                    Ok(EXIT_DONE)
                }
                Err(error) => refusal(io, "the schedule was not triggered", error),
            }
        }
        ScheduleCommand::Backfill {
            id,
            from,
            to,
            overlap,
            key,
        } => {
            let mut body = json!({ "start": from, "end": to });
            if let Some(policy) = overlap {
                body["overlap"] = json!(policy);
            }
            let key = key_of(key.as_ref(), "schedule-backfill", &[id, from, to]);
            match client
                .call(
                    "POST",
                    &format!("{base}/{id}/backfills"),
                    Some(&body),
                    Some(&key),
                )
                .await
            {
                Ok((_, backfill)) => {
                    io.say(format!(
                        "backfill {} of schedule {id} is {}: {} to {}",
                        text(&backfill, "backfillId"),
                        text(&backfill, "state"),
                        text(&backfill, "start"),
                        text(&backfill, "end")
                    ));
                    Ok(EXIT_DONE)
                }
                Err(error) => refusal(io, "the backfill was not made", error),
            }
        }
        ScheduleCommand::Actions { id, limit } => actions(client, &base, id, *limit, io).await,
    }
}

async fn list(
    client: &Client,
    base: &str,
    namespace: &str,
    state: Option<&str>,
    limit: u32,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let mut query = vec![("limit", limit.to_string())];
    if let Some(state) = state {
        query.push(("state", state.to_owned()));
    }
    let query: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let (_, page) = client
        .call("GET", &crate::client::with_query(base, &query), None, None)
        .await
        .map_err(|e| e.to_string())?;
    let items = page["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        io.say(format!("{namespace} has no schedules"));
        return Ok(EXIT_DONE);
    }
    for schedule in &items {
        io.say(format!(
            "{}  {:<28} {:<7} revision {}  {}",
            text(schedule, "scheduleId"),
            text(schedule, "name"),
            text(schedule, "state"),
            schedule["revision"].as_u64().unwrap_or(0),
            text(schedule, "origin")
        ));
    }
    Ok(EXIT_DONE)
}

async fn get(client: &Client, base: &str, id: &str, io: &mut Io<'_>) -> Result<u8, String> {
    let path = crate::client::with_query(
        &format!("{base}/{id}"),
        &[("nextCount", "5"), ("recentCount", "5")],
    );
    let schedule = match client.call("GET", &path, None, None).await {
        Ok((_, schedule)) => schedule,
        Err(error) => return refusal(io, "the schedule could not be read", error),
    };
    io.say(format!(
        "schedule {id}  {}  {}  revision {}",
        text(&schedule, "name"),
        text(&schedule, "state"),
        schedule["revision"].as_u64().unwrap_or(0)
    ));
    for next in schedule["nextActions"]
        .as_array()
        .into_iter()
        .flatten()
        .take(5)
    {
        io.say(format!("  next: {}", text(next, "actualTime")));
    }
    for action in schedule["recentActions"]
        .as_array()
        .into_iter()
        .flatten()
        .take(5)
    {
        io.say(format!(
            "  recent: {}  {}",
            text(action, "nominalTime"),
            text(action, "outcome")
        ));
    }
    // the business id its runs are started under: the prefix is the customer's own text, shown to them and nowhere else
    if let Some(prefix) = schedule["action"]["businessIdPrefix"].as_str() {
        io.say(format!(
            "  business id: {}-<time of the action>  (held by an open run: {}; remembered by a closed one: {})",
            crate::one_line(prefix),
            text(&schedule["action"], "businessIdConflictPolicy"),
            text(&schedule["action"], "businessIdReusePolicy")
        ));
    }
    io.say(format!(
        "  {} open run(s), {} buffered",
        schedule["openRuns"].as_array().map_or(0, Vec::len),
        schedule["bufferedCount"].as_u64().unwrap_or(0)
    ));
    // a schedule a cron trigger owns says whose it is: its rule is the trigger's, changed only by publishing the workflow
    if schedule["origin"].as_str() == Some("TRIGGER") {
        let control = match schedule["controllable"].as_bool() {
            Some(true) => "pause, resume and run now are open (the cron triggers were handed over)",
            Some(false) => {
                "pause, resume and run now are refused until an operator has handed the cron triggers over"
            }
            None => {
                "pause, resume and run now work once an operator has handed the cron triggers over"
            }
        };
        io.say(format!(
            "  owned by the cron trigger {}: its rule changes only when the workflow is published; {control}",
            text(&schedule, "triggerId")
        ));
    }
    Ok(EXIT_DONE)
}

async fn actions(
    client: &Client,
    base: &str,
    id: &str,
    limit: u32,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let path = crate::client::with_query(
        &format!("{base}/{id}/actions"),
        &[("limit", &limit.to_string())],
    );
    let page = match client.call("GET", &path, None, None).await {
        Ok((_, page)) => page,
        Err(error) => return refusal(io, "the actions could not be read", error),
    };
    let items = page["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        io.say(format!("schedule {id} has no actions yet"));
        return Ok(EXIT_DONE);
    }
    for action in &items {
        io.say(format!(
            "{}  {:<9} {:<22} {}{}",
            text(action, "nominalTime"),
            text(action, "kind"),
            text(action, "outcome"),
            action["runId"]
                .as_str()
                .map_or_else(|| "-".to_owned(), crate::one_line),
            // why nothing started, when the platform says (a business id that was held, a ceiling)
            action["reason"]
                .as_str()
                .map_or_else(String::new, |reason| format!(
                    "  {}",
                    crate::one_line(reason)
                ))
        ));
    }
    Ok(EXIT_DONE)
}
