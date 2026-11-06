//! The feature 018 steps: schedules, business identifiers, batch jobs, the with-start calls and
//! resets. Each step is one call of the public typed API, and its observation is the JSON that
//! `sdks/typescript/conformance/runner.ts` writes (`scheduleJson`, `actionJson`, `descriptionJson`,
//! `backfillJson`, `jobJson`, `resetJson`, `receiptJson`), field by field.
use super::*;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

/// Every step operation this module plays.
const OPS: &[&str] = &[
    "scheduleCreate",
    "scheduleGet",
    "scheduleList",
    "scheduleUpdate",
    "scheduleDelete",
    "schedulePause",
    "scheduleResume",
    "scheduleTrigger",
    "scheduleBackfill",
    "scheduleCancelBackfill",
    "scheduleBackfills",
    "scheduleActions",
    "awaitSchedule",
    "awaitBackfill",
    "businessIdGet",
    "signalWithStart",
    "updateWithStart",
    "batchPreview",
    "batchCreate",
    "batchGet",
    "batchList",
    "batchCancel",
    "batchItems",
    "awaitBatch",
    "resetPoints",
    "resetPreview",
    "reset",
    "resets",
    "getReset",
];

pub(super) fn plays(op: &str) -> bool {
    OPS.contains(&op)
}

// ------------------------------------------------------------------------------------ references

/// A string of a step that is exactly one reference becomes its value (`{{namespaceId}}`,
/// `{{workflow:<key>}}`, `{{step:<id>:<field>}}`, `{{now:<offsetMs>}}`, `{{hour:<offsetHours>}}`);
/// anything else stays as it is. A reference that cannot be resolved is the failure of the step.
pub(super) fn resolve(
    value: &Value,
    config: &Value,
    results: &BTreeMap<String, Value>,
) -> Result<Value, String> {
    match value {
        Value::String(text) => reference(text, config, results),
        Value::Array(items) => items
            .iter()
            .map(|item| resolve(item, config, results))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(map) => map
            .iter()
            .map(|(key, item)| Ok((key.clone(), resolve(item, config, results)?)))
            .collect::<Result<serde_json::Map<_, _>, String>>()
            .map(Value::Object),
        other => Ok(other.clone()),
    }
}

fn inner<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.strip_prefix("{{")?
        .strip_suffix("}}")?
        .strip_prefix(prefix)
        .filter(|rest| !rest.is_empty() && !rest.contains('}'))
}
fn offset(text: &str, prefix: &str) -> Option<i64> {
    let digits = inner(text, prefix)?;
    let unsigned = digits.strip_prefix('-').unwrap_or(digits);
    (!unsigned.is_empty() && unsigned.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}
fn reference(
    text: &str,
    config: &Value,
    results: &BTreeMap<String, Value>,
) -> Result<Value, String> {
    if text == "{{namespaceId}}" {
        return Ok(config["namespaceId"].clone());
    }
    if let Some(key) = inner(text, "workflow:") {
        return config["workflows"]
            .get(key)
            .map(|workflow| workflow["workflowId"].clone())
            .ok_or_else(|| format!("no workflow {key}"));
    }
    if let Some((id, field)) = inner(text, "step:").and_then(|rest| rest.split_once(':'))
        && !id.is_empty()
        && !field.is_empty()
    {
        return results
            .get(id)
            .and_then(|result| result.get(field))
            .cloned()
            .ok_or_else(|| format!("step {id} has no {field}"));
    }
    if let Some(ms) = offset(text, "now:") {
        let at = OffsetDateTime::now_utc() + time::Duration::milliseconds(ms);
        return Ok(json!(instant(Some(at)).as_str().unwrap_or_default()));
    }
    if let Some(hours) = offset(text, "hour:") {
        let now = OffsetDateTime::now_utc();
        let start = now
            .replace_minute(0)
            .and_then(|at| at.replace_second(0))
            .and_then(|at| at.replace_nanosecond(0))
            .map_err(|_| "no start of hour".to_owned())?;
        return Ok(json!(
            instant(Some(start + time::Duration::hours(hours)))
                .as_str()
                .unwrap_or_default()
        ));
    }
    Ok(Value::String(text.to_owned()))
}

// ---------------------------------------------------------------------------------- observations

/// An instant as UTC text, whole milliseconds, ending in `Z`; `null` when there is none.
pub(super) fn instant(at: Option<OffsetDateTime>) -> Value {
    at.and_then(|at| {
        let at = at.to_offset(UtcOffset::UTC);
        at.replace_nanosecond(at.nanosecond() / 1_000_000 * 1_000_000)
            .ok()?
            .format(&Rfc3339)
            .ok()
    })
    .map_or(Value::Null, Value::String)
}
/// An instant a fixed spec determines, as epoch milliseconds.
fn epoch(at: Option<OffsetDateTime>) -> Value {
    at.map_or(Value::Null, |at| {
        json!((at.unix_timestamp_nanos() / 1_000_000) as i64)
    })
}
/// A JSON number as JavaScript writes it: `50`, not `50.0`, when it is whole.
fn number(value: Option<f64>) -> Value {
    value.map_or(Value::Null, |n| {
        if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
            json!(n as i64)
        } else {
            json!(n)
        }
    })
}
fn cursor(next: &Option<String>) -> Value {
    next.as_ref().map_or(Value::Null, |_| json!("<cursor>"))
}

fn schedule_json(s: &Schedule) -> Value {
    json!({
        "scheduleId": s.schedule_id, "name": s.name, "origin": s.origin, "state": s.state,
        "revision": s.revision, "exhausted": s.exhausted,
        "action": {
            "workflowId": s.action.workflow_id, "versionPolicy": s.action.version_policy,
            "versionId": s.action.version_id, "inputDigest": s.action.input_digest,
            "nominalTimeField": s.action.nominal_time_field, "runTimeoutMs": s.action.run_timeout_ms,
        },
        "note": s.note, "noteBy": s.note_by, "remainingActions": s.remaining_actions,
        "nextActionEpochMs": epoch(s.next_action_at),
        "createdAt": instant(Some(s.created_at)), "updatedAt": instant(s.updated_at),
    })
}
fn action_json(a: &ScheduleActionRecord) -> Value {
    json!({
        "kind": a.kind, "origin": a.origin, "revision": a.revision, "outcome": a.outcome,
        "reason": a.reason, "runId": a.run_id, "backfillId": a.backfill_id,
        "nominalTime": instant(Some(a.nominal_time)), "actualTime": instant(a.actual_time),
        "summaryCount": a.summary_count,
        "summaryLastNominalTime": instant(a.summary_last_nominal_time),
        "decidedAt": instant(Some(a.decided_at)),
    })
}
fn description_json(d: &ScheduleDescription) -> Value {
    let mut out = schedule_json(d);
    out["nextActions"] = json!(
        d.next_actions
            .iter()
            .map(|n| json!({
                "nominalTimeEpochMs": epoch(Some(n.nominal_time)),
                "actualTimeEpochMs": epoch(Some(n.actual_time)),
            }))
            .collect::<Vec<_>>()
    );
    out["recentActions"] = json!(d.recent_actions.iter().map(action_json).collect::<Vec<_>>());
    out["openRuns"] = json!(d.open_runs);
    out["bufferedCount"] = json!(d.buffered_count);
    out["counters"] = json!(d.counters);
    out
}
fn backfill_json(b: &ScheduleBackfill) -> Value {
    json!({
        "backfillId": b.backfill_id, "start": instant(Some(b.start)), "end": instant(Some(b.end)),
        "cursor": instant(b.cursor), "state": b.state, "overlap": b.overlap, "counts": b.counts,
    })
}
fn counts_json(c: &BatchJobCounts) -> Value {
    json!({
        "selected": c.selected, "succeeded": c.succeeded, "skipped": c.skipped,
        "failed": c.failed, "cancelled": c.cancelled, "deferred": c.deferred,
    })
}
fn job_json(j: &BatchJob) -> Value {
    json!({
        "batchId": j.batch_id, "action": j.action, "state": j.state, "failureReason": j.failure_reason,
        "createdBy": j.created_by, "hasPayload": j.has_payload, "maxItems": j.max_items,
        "itemsPerSecond": number(j.items_per_second), "counts": counts_json(&j.counts),
        "createdAt": instant(Some(j.created_at)), "startedAt": instant(j.started_at),
        "finishedAt": instant(j.finished_at), "expiresAt": instant(j.expires_at),
    })
}
fn reset_json(r: &Reset) -> Value {
    json!({
        "resetId": r.reset_id, "sourceRunId": r.source_run_id, "sequence": r.sequence,
        "state": r.state, "failureReason": r.failure_reason, "newRunId": r.new_run_id,
        "reapplyReport": r.reapply_report, "createdAt": instant(r.created_at),
    })
}
fn receipt_json(r: &Option<UpdateReceipt>) -> Value {
    r.as_ref().map_or(Value::Null, |r| {
        json!({
            "operation": r.operation, "updateKey": r.update_key, "status": r.status,
            "result": r.result, "beforeStateRevision": r.before_state_revision,
            "afterStateRevision": r.after_state_revision, "runVersion": r.run_version,
            "replayed": r.replayed,
        })
    })
}

// ----------------------------------------------------------------------------------------- steps

fn invalid(message: &str) -> SagatError {
    SagatError::validation(message)
}
fn call(step: &Value) -> CallOptions {
    CallOptions {
        idempotency_key: step["idempotencyKey"].as_str().map(str::to_owned),
        ..Default::default()
    }
}
fn typed<T: serde::de::DeserializeOwned>(value: &Value, what: &str) -> Result<T, SagatError> {
    serde_json::from_value(value.clone())
        .map_err(|_| invalid(&format!("the scenario's {what} is not a valid request")))
}
fn page_limit(step: &Value) -> Option<u16> {
    step["limit"].as_u64().map(|n| n as u16)
}
fn some(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}
/// The text of `field` in the result of the step that `step[key]` names.
fn from_step(
    step: &Value,
    key: &str,
    field: &str,
    results: &BTreeMap<String, Value>,
) -> Result<String, SagatError> {
    results
        .get(text(step, key))
        .and_then(|result| result.get(field))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| invalid("a scenario step has no such result"))
}
/// The id of the first step whose result holds this run, or `?`.
fn step_of_run(run_id: &str, order: &[String], results: &BTreeMap<String, Value>) -> String {
    order
        .iter()
        .find(|id| {
            results.get(*id).is_some_and(|found| {
                found["runId"].as_str() == Some(run_id)
                    || found["newRunId"].as_str() == Some(run_id)
            })
        })
        .cloned()
        .unwrap_or_else(|| "?".into())
}
/// A start delay given by a scenario in milliseconds (possibly fractional) as a `Duration`.
pub(super) fn delay_of(value: &Value) -> Result<Option<Duration>, SagatError> {
    let Some(ms) = value.as_f64() else {
        return Ok(None);
    };
    if !ms.is_finite() || ms < 0.0 {
        return Err(invalid("a Duration cannot be negative or not a number"));
    }
    let whole = ms.trunc();
    Ok(Some(
        Duration::from_millis(whole as u64) + Duration::from_nanos(((ms - whole) * 1e6) as u64),
    ))
}
async fn awaiting<F, Fut>(what: &str, step: &Value, mut done: F) -> Result<(), SagatError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool, SagatError>>,
{
    let end = tokio::time::Instant::now() + millis(step, "timeoutMs", 30_000);
    while !done().await? {
        if tokio::time::Instant::now() > end {
            return Err(invalid(&format!("timed out waiting for {what}")));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

pub(super) async fn play(
    op: &str,
    client: &Client,
    step: &Value,
    config: &Value,
    results: &BTreeMap<String, Value>,
    order: &[String],
) -> Result<Option<Value>, SagatError> {
    let namespace = text(config, "namespaceId");
    let schedules = client.schedules();
    let batch = client.batch();
    let schedule = || from_step(step, "schedule", "scheduleId", results);
    Ok(Some(match op {
        "scheduleCreate" => {
            let write: ScheduleWrite = typed(&step["write"], "schedule")?;
            schedule_json(&schedules.create(namespace, &write, call(step)).await?)
        }
        "scheduleGet" => description_json(
            &schedules
                .get(
                    namespace,
                    &schedule()?,
                    ScheduleDescribeOptions {
                        next_count: step["nextCount"].as_u64().map(|n| n as u32),
                        recent_count: step["recentCount"].as_u64().map(|n| n as u32),
                    },
                    Default::default(),
                )
                .await?,
        ),
        "scheduleList" => {
            let filter = ScheduleListFilter {
                state: some(&step["filter"]["state"]),
                origin: some(&step["filter"]["origin"]),
                workflow_id: some(&step["filter"]["workflowId"]),
                name: some(&step["filter"]["name"]),
                limit: page_limit(step),
                cursor: None,
            };
            let read = |s: &Schedule| json!({"name":s.name,"state":s.state,"origin":s.origin,"revision":s.revision});
            if step["all"] == true {
                let stream = schedules.list_all(namespace, filter, Default::default());
                futures_util::pin_mut!(stream);
                let mut items = Vec::new();
                while let Some(item) = stream.next().await {
                    items.push(read(&item?));
                }
                json!({"items":items,"nextCursor":null})
            } else {
                let page = schedules
                    .list(namespace, filter, Default::default())
                    .await?;
                json!({"items":page.items.iter().map(read).collect::<Vec<_>>(),"nextCursor":cursor(&page.next_cursor)})
            }
        }
        "scheduleUpdate" => {
            let write: ScheduleWrite = typed(&step["write"], "schedule")?;
            schedule_json(
                &schedules
                    .update(
                        namespace,
                        &schedule()?,
                        &write,
                        step["expectedRevision"].as_u64().unwrap_or(0),
                        call(step),
                    )
                    .await?,
            )
        }
        "scheduleDelete" => {
            schedules
                .delete(
                    namespace,
                    &schedule()?,
                    step["expectedRevision"].as_u64().unwrap_or(0),
                    call(step),
                )
                .await?;
            json!({"deleted":true})
        }
        "schedulePause" => schedule_json(
            &schedules
                .pause(namespace, &schedule()?, text(step, "note"), call(step))
                .await?,
        ),
        "scheduleResume" => schedule_json(
            &schedules
                .resume(namespace, &schedule()?, text(step, "note"), call(step))
                .await?,
        ),
        "scheduleTrigger" => action_json(
            &schedules
                .trigger(
                    namespace,
                    &schedule()?,
                    step["overlap"].as_str().map(OverlapPolicy::from),
                    call(step),
                )
                .await?,
        ),
        "scheduleBackfill" => backfill_json(
            &schedules
                .backfill(
                    namespace,
                    &schedule()?,
                    text(step, "start"),
                    text(step, "end"),
                    step["overlap"].as_str().map(OverlapPolicy::from),
                    call(step),
                )
                .await?,
        ),
        "scheduleCancelBackfill" => backfill_json(
            &schedules
                .cancel_backfill(
                    namespace,
                    &schedule()?,
                    &from_step(step, "backfill", "backfillId", results)?,
                    call(step),
                )
                .await?,
        ),
        "scheduleBackfills" => json!({"items":schedules
            .list_backfills(namespace, &schedule()?, Default::default())
            .await?
            .iter()
            .map(backfill_json)
            .collect::<Vec<_>>()}),
        "scheduleActions" => {
            let page = schedules
                .list_actions(
                    namespace,
                    &schedule()?,
                    ScheduleActionFilter {
                        outcome: some(&step["outcome"]),
                        limit: page_limit(step),
                        cursor: None,
                    },
                    Default::default(),
                )
                .await?;
            json!({"items":page.items.iter().map(action_json).collect::<Vec<_>>(),"nextCursor":cursor(&page.next_cursor)})
        }
        "awaitSchedule" => {
            // The scheduler decides the times of a backfill: wait for its counters, never for a delay.
            let id = schedule()?;
            let counter = text(step, "counter");
            let at_least = step["atLeast"].as_u64().unwrap_or(1);
            awaiting(&format!("schedule counter {counter}"), step, || async {
                let described = schedules
                    .get(
                        namespace,
                        &id,
                        ScheduleDescribeOptions {
                            next_count: Some(0),
                            recent_count: Some(0),
                        },
                        Default::default(),
                    )
                    .await?;
                Ok(described.counters.get(counter).copied().unwrap_or(0) >= at_least)
            })
            .await?;
            return Ok(None);
        }
        "awaitBackfill" => {
            // A backfill counts the times it found already acted on in its own counts.
            let id = schedule()?;
            let backfill = from_step(step, "backfill", "backfillId", results)?;
            let state = text(step, "state");
            awaiting(&format!("backfill state {state}"), step, || async {
                Ok(schedules
                    .list_backfills(namespace, &id, Default::default())
                    .await?
                    .iter()
                    .any(|b| b.backfill_id.to_string() == backfill && b.state == state))
            })
            .await?;
            return Ok(None);
        }
        "businessIdGet" => {
            let b = client
                .business_ids()
                .get(namespace, text(step, "businessId"), Default::default())
                .await?;
            json!({
                "businessId": b.business_id, "executionId": b.execution_id,
                "currentRunId": b.current_run_id, "open": b.open, "lastStatus": b.last_status,
                "workflowId": b.workflow_id, "boundAt": instant(Some(b.bound_at)),
                "reusableAfter": instant(b.reusable_after),
            })
        }
        "signalWithStart" => {
            let workflow = if step["workflow"].is_string() {
                text(&config["workflows"][text(step, "workflow")], "workflowId")
            } else {
                text(step, "workflowId")
            };
            let options = WithStartOptions {
                call: call(step),
                business_id_reuse_policy: step["businessIdReusePolicy"]
                    .as_str()
                    .map(BusinessIdReusePolicy::from),
                ..Default::default()
            };
            let signal = &step["signal"];
            let r = client
                .signal_with_start(
                    workflow,
                    text(step, "businessId"),
                    &step.get("input").cloned().unwrap_or_else(|| json!({})),
                    text(signal, "name"),
                    &signal.get("payload").cloned().unwrap_or_else(|| json!({})),
                    options,
                )
                .await?;
            json!({"runId":r.run_id,"started":r.started,"deliveredTo":r.delivered_to})
        }
        "updateWithStart" => {
            let update = &step["update"];
            let options = UpdateWithStartOptions {
                with_start: WithStartOptions {
                    call: call(step),
                    business_id_reuse_policy: step["businessIdReusePolicy"]
                        .as_str()
                        .map(BusinessIdReusePolicy::from),
                    ..Default::default()
                },
                business_id_conflict_policy: step["businessIdConflictPolicy"]
                    .as_str()
                    .map(BusinessIdConflictPolicy::from),
                start_delay: delay_of(&step["startDelayMs"])?,
                expected_state_revision: update["expectedStateRevision"].as_u64(),
            };
            let r = client
                .update_with_start(
                    text(&config["workflows"][text(step, "workflow")], "workflowId"),
                    text(step, "businessId"),
                    &step.get("input").cloned().unwrap_or_else(|| json!({})),
                    text(update, "name"),
                    text(update, "updateKey"),
                    &update.get("args").cloned().unwrap_or(Value::Null),
                    options,
                )
                .await?;
            json!({"runId":r.run_id,"started":r.started,"update":{
                "outcome": r.update.outcome, "reason": r.update.reason,
                "receipt": receipt_json(&r.update.receipt),
            }})
        }
        "batchPreview" => {
            let selector: BatchSelector = typed(&step["selector"], "selector")?;
            let p = batch
                .preview(
                    namespace,
                    &selector,
                    step["maxItems"].as_u64(),
                    Default::default(),
                )
                .await?;
            // The sample is a set of runs in the order the server chose; the scenario compares which of
            // its own runs it holds.
            let mut steps: Vec<String> = p
                .sample
                .iter()
                .map(|id| step_of_run(&id.to_string(), order, results))
                .collect();
            steps.sort();
            json!({"matched":p.matched,"capped":p.capped,"sampleSize":p.sample.len(),"sampleSteps":steps})
        }
        "batchCreate" => {
            // What does not change while the job runs (its state and counters do): the job as accepted.
            let request: BatchRequest = typed(&step["request"], "batch request")?;
            let j = batch.create(namespace, &request, call(step)).await?;
            json!({
                "batchId": j.batch_id, "action": j.action, "createdBy": j.created_by,
                "hasPayload": j.has_payload, "maxItems": j.max_items, "itemsPerSecond": number(j.items_per_second),
            })
        }
        "batchGet" => job_json(
            &batch
                .get(
                    namespace,
                    &from_step(step, "batch", "batchId", results)?,
                    Default::default(),
                )
                .await?,
        ),
        "batchList" => {
            let page = batch
                .list(
                    namespace,
                    BatchListFilter {
                        state: some(&step["state"]),
                        limit: page_limit(step),
                        cursor: None,
                    },
                    Default::default(),
                )
                .await?;
            json!({"items":page.items.iter().map(|j| json!({"action":j.action,"state":j.state,"counts":counts_json(&j.counts)})).collect::<Vec<_>>(),"nextCursor":cursor(&page.next_cursor)})
        }
        "batchCancel" => job_json(
            &batch
                .cancel(
                    namespace,
                    &from_step(step, "batch", "batchId", results)?,
                    text(step, "reason"),
                    call(step),
                )
                .await?,
        ),
        "batchItems" => {
            let page = batch
                .list_items(
                    namespace,
                    &from_step(step, "batch", "batchId", results)?,
                    BatchListFilter {
                        state: some(&step["state"]),
                        limit: page_limit(step),
                        cursor: None,
                    },
                    Default::default(),
                )
                .await?;
            // The order of the items is the server's; which of the scenario's runs sits at which ordinal
            // is not part of the contract, so the items are named by the step that started the run and
            // sorted by it.
            let mut items: Vec<(String, Value)> = page
                .items
                .iter()
                .map(|i| {
                    let run_step = step_of_run(&i.run_id.to_string(), order, results);
                    (
                        run_step.clone(),
                        json!({
                            "runStep": run_step, "state": i.state, "reason": i.reason,
                            "errorCode": i.error_code, "attempts": i.attempts, "newRunId": i.new_run_id,
                        }),
                    )
                })
                .collect();
            items.sort_by(|a, b| a.0.cmp(&b.0));
            json!({"items":items.into_iter().map(|(_, item)| item).collect::<Vec<_>>(),"nextCursor":cursor(&page.next_cursor)})
        }
        "awaitBatch" => {
            let id = from_step(step, "batch", "batchId", results)?;
            let state = text(step, "state");
            awaiting(&format!("batch state {state}"), step, || async {
                Ok(batch.get(namespace, &id, Default::default()).await?.state == state)
            })
            .await?;
            return Ok(None);
        }
        "resetPoints" => {
            let page = client
                .reset_points(
                    &run_of(step, results)?,
                    PageOptions {
                        limit: page_limit(step),
                        cursor: None,
                    },
                    Default::default(),
                )
                .await?;
            json!({"items":page.items.iter().map(|p| json!({
                "sequence": p.sequence, "kind": p.kind, "node": p.node,
                "occurredAt": instant(Some(p.occurred_at)),
            })).collect::<Vec<_>>(),"nextCursor":cursor(&page.next_cursor)})
        }
        "resetPreview" => {
            let plan = client
                .preview_reset(
                    &run_of(step, results)?,
                    step["sequence"].as_u64().unwrap_or(0),
                    step["reapply"].as_str().map(ReapplyPolicy::from),
                    Default::default(),
                )
                .await?;
            json!({
                "sequence": plan.sequence,
                "restored": plan.restored.iter().map(|r| json!({"node":r.node,"kind":r.kind})).collect::<Vec<_>>(),
                "reexecuted": plan.reexecuted.iter().map(|r| json!({"node":r.node,"completedInSource":r.completed_in_source,"markers":r.markers})).collect::<Vec<_>>(),
                "reapply": plan.reapply, "requiresAcknowledgement": plan.requires_acknowledgement,
                "prefixEvents": plan.prefix_events,
            })
        }
        "reset" => {
            let request = ResetRequest {
                sequence: step["sequence"].as_u64().unwrap_or(0),
                reason: text(step, "reason").to_owned(),
                reapply: step["reapply"].as_str().map(ReapplyPolicy::from),
                source_policy: step["sourcePolicy"].as_str().map(SourcePolicy::from),
                business_id: step["businessId"].as_str().map(BusinessIdDisposition::from),
                acknowledge_reexecution: step["acknowledgeReexecution"].as_bool(),
            };
            reset_json(
                &client
                    .reset(&run_of(step, results)?, &request, call(step))
                    .await?,
            )
        }
        "resets" => json!({"items":client
            .list_resets(&run_of(step, results)?, Default::default())
            .await?
            .iter()
            .map(reset_json)
            .collect::<Vec<_>>()}),
        "getReset" => reset_json(
            &client
                .get_reset(
                    &run_of(step, results)?,
                    &from_step(step, "reset", "resetId", results)?,
                    Default::default(),
                )
                .await?,
        ),
        _ => return Err(invalid("a scenario operation is not implemented")),
    }))
}
