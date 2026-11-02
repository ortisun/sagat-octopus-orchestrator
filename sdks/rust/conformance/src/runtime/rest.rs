//! Observation adapters call public typed APIs; the server retains all workflow semantics.
use super::*;
fn pick(raw: Value, keys: &[&str]) -> Value {
    json!(
        keys.iter()
            .map(|key| (
                (*key).to_owned(),
                raw.get(*key).cloned().unwrap_or(Value::Null)
            ))
            .collect::<BTreeMap<_, _>>()
    )
}
fn call(step: &Value) -> CallOptions {
    CallOptions {
        traceparent: step["traceparent"].as_str().map(str::to_owned),
        idempotency_key: step["idempotencyKey"].as_str().map(str::to_owned),
        ..Default::default()
    }
}
fn reference(
    value: &str,
    config: &Value,
    results: &BTreeMap<String, Value>,
) -> Result<String, SagatError> {
    if value == "{{namespaceId}}" {
        return Ok(text(config, "namespaceId").into());
    }
    for (prefix, key) in [("{{chainId:", "runId"), ("{{currentRunId:", "currentRunId")] {
        if let Some(name) = value
            .strip_prefix(prefix)
            .and_then(|s| s.strip_suffix("}}"))
        {
            return results
                .get(name)
                .and_then(|r| r[key].as_str())
                .map(str::to_owned)
                .ok_or_else(|| {
                    SagatError::validation("a scenario reference has no chain or head")
                });
        }
    }
    Ok(value.into())
}
fn chain_result(chain: ChainInspection) -> Value {
    let mut output = pick(
        chain.raw,
        &[
            "chainId",
            "currentRunId",
            "revision",
            "cancelRequested",
            "finalOutcome",
        ],
    );
    output["segments"]=json!(chain.segments.into_iter().map(|s|json!({"index":s.index,"runId":s.run_id,"predecessorRunId":s.predecessor_run_id,"successorRunId":s.successor_run_id})).collect::<Vec<_>>());
    output
}
pub(super) async fn play(
    op: &str,
    client: &Client,
    step: &Value,
    config: &Value,
    results: &BTreeMap<String, Value>,
) -> Result<Option<Value>, SagatError> {
    let namespace = reference(text(step, "namespaceId"), config, results)?;
    let chain = reference(text(step, "chainId"), config, results)?;
    Ok(Some(match op {
        "signal" => {
            let result = client
                .signal(
                    &run_of(step, results)?,
                    text(step, "name"),
                    &step.get("payload").cloned().unwrap_or_else(|| json!({})),
                    call(step),
                )
                .await?;
            json!({"deliveredTo":result.delivered_to})
        }
        "publishEvent" => {
            let mut event = PublishEvent::new(
                text(step, "type"),
                step.get("data").cloned().unwrap_or_else(|| json!({})),
            );
            event.event_id = step["eventId"].as_str().map(str::to_owned);
            event.correlation_key = step["correlationKey"].as_str().map(str::to_owned);
            pick(
                client.publish_event(event, call(step)).await?.raw,
                &["eventId", "status"],
            )
        }
        "query" => pick(
            client
                .query(
                    &run_of(step, results)?,
                    text(step, "name"),
                    &step["args"],
                    call(step),
                )
                .await?
                .raw,
            &["runVersion", "stateRevision", "result"],
        ),
        "update" => pick(
            client
                .update(
                    &run_of(step, results)?,
                    text(step, "name"),
                    UpdateRequest {
                        update_key: text(step, "updateKey").into(),
                        args: step["args"].clone(),
                        expected_state_revision: step["expectedStateRevision"].as_u64(),
                    },
                    call(step),
                )
                .await?
                .raw,
            &[
                "operation",
                "updateKey",
                "status",
                "result",
                "beforeStateRevision",
                "afterStateRevision",
                "runVersion",
                "replayed",
            ],
        ),
        "listApprovals" => {
            let page = PageOptions {
                limit: step["limit"].as_u64().map(|n| n as u16),
                cursor: step["cursor"].as_str().map(str::to_owned),
            };
            let run = run_of(step, results)?;
            let mut approvals = client
                .list_approvals(&run, page.clone(), call(step))
                .await?;
            for _ in 0..100 {
                if step["waitForItem"] != true || !approvals.items.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
                approvals = client
                    .list_approvals(&run, page.clone(), call(step))
                    .await?;
            }
            json!({"items":approvals.items.into_iter().map(|item|pick(item.raw,&["approvalId","runId","nodeId","instancePath","status","deadline","settledAt"])).collect::<Vec<_>>(),"nextCursor":approvals.next_cursor})
        }
        "decideApproval" => {
            let id = step["approvalId"]
                .as_str()
                .or_else(|| {
                    results
                        .get(text(step, "approval"))
                        .and_then(|r| r["items"][0]["approvalId"].as_str())
                })
                .ok_or_else(|| SagatError::validation("a scenario step has no approval"))?;
            pick(
                client
                    .decide_approval(
                        id,
                        ApprovalDecisionRequest {
                            decision_key: text(step, "decisionKey").into(),
                            decision: serde_json::from_value(step["decision"].clone())
                                .map_err(|_| SagatError::validation("invalid scenario decision"))?,
                            reason_code: step["reasonCode"].as_str().map(str::to_owned),
                        },
                        call(step),
                    )
                    .await?
                    .raw,
                &[
                    "receiptId",
                    "approvalId",
                    "decisionKey",
                    "decision",
                    "reasonCode",
                    "actor",
                    "decidedAt",
                    "outcomeSequence",
                    "replayed",
                ],
            )
        }
        "queryChain" => pick(
            client
                .query_chain(
                    &namespace,
                    &chain,
                    text(step, "name"),
                    ChainQueryRequest {
                        args: step["args"].clone(),
                        expected_run_id: step["expectedRunId"]
                            .as_str()
                            .map(|s| reference(s, config, results))
                            .transpose()?,
                    },
                    call(step),
                )
                .await?
                .raw,
            &["runId", "runVersion", "stateRevision", "result"],
        ),
        "updateChain" => pick(
            client
                .update_chain(
                    &namespace,
                    &chain,
                    text(step, "name"),
                    ChainUpdateRequest {
                        update_key: text(step, "updateKey").into(),
                        args: step["args"].clone(),
                        expected_state_revision: step["expectedStateRevision"].as_u64(),
                        expected_run_id: step["expectedRunId"]
                            .as_str()
                            .map(|s| reference(s, config, results))
                            .transpose()?,
                    },
                    call(step),
                )
                .await?
                .raw,
            &[
                "runId",
                "operation",
                "updateKey",
                "status",
                "result",
                "beforeStateRevision",
                "afterStateRevision",
                "runVersion",
                "replayed",
            ],
        ),
        "signalChain" => {
            let result = client
                .signal_chain(
                    &namespace,
                    &chain,
                    text(step, "name"),
                    &step["payload"],
                    ChainCommand {
                        expected_run_id: reference(text(step, "expectedRunId"), config, results)?,
                    },
                    call(step),
                )
                .await?;
            json!({"deliveredTo":result.delivered_to})
        }
        "cancelChain" => {
            client
                .cancel_chain(
                    &namespace,
                    &chain,
                    text(step, "reason"),
                    ChainCommand {
                        expected_run_id: reference(text(step, "expectedRunId"), config, results)?,
                    },
                    call(step),
                )
                .await?;
            json!({"accepted":true})
        }
        "listChainRuns" => {
            let cursor = step["cursor"]
                .as_str()
                .map(|s| {
                    if let Some(name) = s
                        .strip_prefix("{{nextCursor:")
                        .and_then(|s| s.strip_suffix("}}"))
                    {
                        results
                            .get(name)
                            .and_then(|r| r["nextCursor"].as_str())
                            .map(str::to_owned)
                            .ok_or_else(|| SagatError::validation("scenario page has no cursor"))
                    } else {
                        Ok(s.to_owned())
                    }
                })
                .transpose()?;
            let page = client
                .list_chain_runs(
                    &namespace,
                    &chain,
                    PageOptions {
                        limit: step["limit"].as_u64().map(|n| n as u16),
                        cursor,
                    },
                    call(step),
                )
                .await?;
            json!({"items":page.items.into_iter().map(|item|pick(item.raw,&["index","runId","status","completionKind","predecessorRunId","successorRunId","createdAt","finishedAt"])).collect::<Vec<_>>(),"nextCursor":page.next_cursor})
        }
        "inspectChain" => {
            let mut inspected = client.inspect_chain(&namespace, &chain, call(step)).await?;
            if let Some(previous) = step["waitForHeadChange"].as_str() {
                let previous = reference(previous, config, results)?;
                for _ in 0..100 {
                    if inspected.current_run_id.to_string() != previous {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    inspected = client.inspect_chain(&namespace, &chain, call(step)).await?;
                }
            }
            chain_result(inspected)
        }
        "waitForChainCompletion" => chain_result(
            client
                .wait_for_chain_completion(
                    &namespace,
                    &chain,
                    WaitOptions {
                        timeout: millis(step, "timeoutMs", 60000),
                        poll_interval: Duration::from_millis(100),
                        ..Default::default()
                    },
                )
                .await?,
        ),
        _ => {
            return Err(SagatError::validation(
                "unsupported scenario REST operation",
            ));
        }
    }))
}
