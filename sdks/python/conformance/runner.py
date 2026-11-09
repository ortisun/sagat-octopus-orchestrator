"""The conformance runner of the Python SDK (tests/contract/sdk-conformance/README.md).

It uses only the public API of the SDK (``import sagat_octopus_orchestrator``): the builders in ``build``
mode, the client and the worker host in ``run`` mode, plus one plain HTTP call to the control endpoint.

    python -m conformance.runner build <scenario.json>
    python -m conformance.runner run <scenario.json> <config.json> <out.json>

It fails loudly: any scenario that does not finish exits non-zero with the reason on stderr, and ``out.json``
is written only on success.
"""

from __future__ import annotations

import asyncio
import hashlib
import json
import re
import sys
import traceback
from collections.abc import Awaitable, Callable
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

import httpx

from sagat_octopus_orchestrator import (
    SDK_VERSION,
    ActivityContext,
    ActivityFailure,
    ApprovalPolicy,
    BatchFilter,
    BatchJob,
    BatchRequest,
    BatchSelector,
    BatchSignal,
    BatchUpdate,
    BlobDescriptor,
    BlobOutput,
    Body,
    ClientConfig,
    Flow,
    QueryDefinition,
    Reset,
    RetryPolicy,
    Run,
    SagatClient,
    SagatError,
    Schedule,
    ScheduleAction,
    ScheduleActionRecord,
    ScheduleBackfill,
    ScheduleCalendarSpec,
    ScheduleDescription,
    ScheduleDst,
    ScheduleInterval,
    SchedulePolicies,
    ScheduleRange,
    ScheduleSpec,
    ScheduleWrite,
    StateDefinition,
    SwitchCase,
    UpdateDefinition,
    UpdateWithStartReceipt,
    Verdict,
    WorkerConfig,
    WorkerHost,
    body,
    flow,
    format_instant,
    trace_id_of,
    workflow,
)

from .faults import Fault, FaultPlan

Obj = dict[str, Any]

SCENARIO_DEADLINE = 280.0
"""Seconds. The harness gives a runner 300 s and then kills it without saying why; this says where it was."""


def read(path: str) -> Obj:
    return json.loads(Path(path).read_text(encoding="utf-8"))  # type: ignore[no-any-return]


def templated(value: Any, unique: str) -> Any:
    """Replaces ``{{unique}}`` in every string of the scenario."""
    if isinstance(value, str):
        return value.replace("{{unique}}", unique)
    if isinstance(value, list):
        return [templated(item, unique) for item in value]
    if isinstance(value, dict):
        return {key: templated(item, unique) for key, item in value.items()}
    return value


def snake(options: Obj | None) -> Obj:
    """``{"timeoutMs": 1}`` as ``{"timeout_ms": 1}``: the scenario names options as the IR does."""
    return {re.sub(r"(?<!^)(?=[A-Z])", "_", key).lower(): value for key, value in (options or {}).items()}


# ----------------------------------------------------------------------------------------- builders


def apply(target: Flow, ops: list[Obj]) -> None:
    for op in ops:
        kind = op["op"]
        if kind == "step":
            target.step(op["id"], op["activity"], **snake(op.get("options")))
        elif kind == "database":
            target.database(op["id"], **snake(op["options"]))
        elif kind == "icebergWrite":
            target.iceberg_write(op["id"], **snake(op["options"]))
        elif kind == "script":
            target.script(op["id"], op["artifactDigest"], **snake(op.get("options")))
        elif kind == "sleep":
            target.sleep(op["id"], op["durationMs"])
        elif kind == "waitForEvent":
            target.wait_for_event(op["id"], **snake(op["options"]))
        elif kind == "invokeChild":
            target.invoke_child(op["id"], op["workflow"], **snake(op.get("options")))
        elif kind == "condition":
            target.condition(op["id"], op["expression"], op["ifTrue"], op["ifFalse"])
        elif kind == "switch":
            target.switch(
                op["id"],
                [SwitchCase(id=item["id"], when=item["when"], target=item["target"]) for item in op["cases"]],
                op["default"],
            )
        elif kind == "forEach":
            target.for_each(
                op["id"],
                op["items"],
                op["body"],
                op["result"],
                max_items=op["maxItems"],
                max_concurrency=op["maxConcurrency"],
            )
        elif kind == "map":
            target.map(
                op["id"],
                op["items"],
                op["body"],
                op["result"],
                max_items=op["maxItems"],
                max_concurrency=op["maxConcurrency"],
            )
        elif kind == "while":
            target.while_loop(
                op["id"],
                op["condition"],
                op["body"],
                op["initial"],
                op["carry"],
                max_iterations=op["maxIterations"],
                timeout_ms=op["timeoutMs"],
                delay_ms=op.get("delayMs"),
                on_error=op.get("onError"),
            )
        elif kind == "repeatUntil":
            target.repeat_until(
                op["id"],
                op["condition"],
                op["body"],
                op["initial"],
                op["carry"],
                max_iterations=op["maxIterations"],
                timeout_ms=op["timeoutMs"],
                delay_ms=op.get("delayMs"),
                on_error=op.get("onError"),
            )
        elif kind == "approval":
            policy = op["policy"]
            target.approval(
                op["id"],
                ApprovalPolicy(
                    subjects=policy.get("subjects"),
                    roles=policy.get("roles"),
                    exclude_initiator=policy.get("excludeInitiator"),
                ),
                timeout_ms=op["timeoutMs"],
                on_approve=op["onApprove"],
                on_reject=op["onReject"],
                on_expire=op["onExpire"],
            )
        elif kind == "continueAsNew":
            target.continue_as_new(op["id"], op["input"], state=op.get("state"))
        elif kind == "return":
            if not isinstance(target, Body):
                raise ValueError("return is only valid in a named body")
            target.return_value(op["id"], op["result"])
        elif kind == "parallel":
            branches = []
            for branch_ops in op["branches"]:
                branch = flow()
                apply(branch, branch_ops)
                branches.append(branch)
            target.parallel(op["id"], op["join"], branches)
        elif kind == "saga":
            body = flow()
            apply(body, op["body"])
            target.saga(op["id"], body, **snake(op.get("options")))
        elif kind == "end":
            target.end(op["id"], **snake(op.get("options")))
        else:
            raise ValueError(f"unknown builder op {kind}")


def builder_of(definition: Obj) -> Any:
    builder = workflow(definition["name"])
    if definition.get("description"):
        builder.description(definition["description"])
    for key, value in (definition.get("labels") or {}).items():
        builder.label(key, value)
    for trigger in definition.get("triggers") or []:
        fields = snake(trigger)
        builder.trigger(fields.pop("kind"), **fields)
    if definition.get("inputSchema") is not None:
        builder.input_schema(definition["inputSchema"])
    if definition.get("outputSchema") is not None:
        builder.output_schema(definition["outputSchema"])
    if definition.get("limits") is not None:
        builder.limits(**snake(definition["limits"]))
    if definition.get("state") is not None:
        declared = definition["state"]
        queries = {
            name: QueryDefinition(
                args_schema=query.get("argsSchema"),
                result_schema=query.get("resultSchema"),
                expression=query.get("expression"),
                disclosure=query.get("disclosure"),
            )
            for name, query in (declared.get("queries") or {}).items()
        }
        updates = {
            name: UpdateDefinition(
                args_schema=update.get("argsSchema"),
                validate=update.get("validate"),
                state=update.get("state"),
                result=update.get("result"),
                result_schema=update.get("resultSchema"),
                disclosure=update.get("disclosure"),
                reapply_on_reset=update.get("reapplyOnReset", False),
            )
            for name, update in (declared.get("updates") or {}).items()
        }
        builder.state(
            StateDefinition(
                schema=declared.get("schema"),
                initial=declared.get("initial"),
                queries=queries,
                updates=updates,
            )
        )
    if definition.get("chainPolicy") is not None:
        policy = definition["chainPolicy"]
        builder.chain_policy(max_segments=policy["maxSegments"], timeout_ms=policy["timeoutMs"])
    for name, body_definition in (definition.get("bodies") or {}).items():
        nested = body()
        apply(nested, body_definition["program"])
        builder.body(name, nested)
    apply(builder, definition["program"])
    return builder


def build(scenario: Obj) -> Obj:
    workflows: Obj = {}
    canonical: Obj = {}
    for key, definition in (scenario.get("workflows") or {}).items():
        builder = builder_of(definition)
        workflows[key] = builder.build()
        canonical[key] = builder.to_json()  # RFC 8785 text, compared byte for byte
    errors: Obj = {}
    for key, invalid in (scenario.get("invalidWorkflows") or {}).items():
        contains = str(invalid.get("contains") or "")
        try:
            candidate = builder_of(invalid["workflow"])
            candidate.build()
            candidate.to_json()  # the canonical text refuses what build() lets through (an unsafe integer)
            errors[key] = {"raised": False}
        except SagatError as error:
            errors[key] = {"raised": True, "code": error.code, "mentions": contains == "" or contains in error.message}
        except Exception:
            errors[key] = {"raised": True, "code": "UNEXPECTED", "mentions": False}
    built: Obj = {"workflows": workflows, "canonical": canonical}
    if errors:
        built["errors"] = errors
    return built


# --------------------------------------------------------------------------------------- observations


def instant(value: Any) -> str | None:
    return format_instant(value) if value is not None else None


def run_json(run: Run) -> Obj:
    observed: Obj = {
        "runId": run.run_id,
        "workflowId": run.workflow_id,
        "versionId": run.version_id,
        "status": run.status,
        "version": run.version,
        "paused": run.paused,
        "startedBy": run.started_by,
        "createdAt": instant(run.created_at),
        "updatedAt": instant(run.updated_at),
        "finishedAt": instant(run.finished_at),
        "deadline": instant(run.deadline),
        "traceId": run.trace_id,
        "correlationId": run.correlation_id,
        "depth": run.depth,
        "parent": (
            {"runId": run.parent.run_id, "node": run.parent.node, "cancelPolicy": run.parent.cancel_policy}
            if run.parent
            else None
        ),
    }
    # Present only for a run in a chain, so the observations of every other run stay as they were.
    if run.chain is not None:
        observed["chain"] = {
            "chainId": run.chain.chain_id,
            "segmentIndex": run.chain.segment_index,
            "completionKind": run.chain.completion_kind,
            "successorRunId": run.chain.successor_run_id,
        }
    # Present only for a run started again from an ended one.
    if run.rerun_of is not None:
        observed["rerunOf"] = run.rerun_of
    # Present only for a run started under a business identifier, or held by a start delay.
    if run.business_id is not None:
        observed["businessId"] = run.business_id
    if run.start_delayed_until is not None:
        observed["startDelayedUntil"] = instant(run.start_delayed_until)
    return observed


# The feature 018 results. Instants that depend on when the scenario ran are written as instants (the harness compares
# them as `<time>`); the ones a fixed spec determines are written as epoch milliseconds, so that a wrong parse of an
# instant shows.
_EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)


def epoch(value: datetime | None) -> int | None:
    return None if value is None else (value - _EPOCH) // timedelta(milliseconds=1)


def number(value: Any) -> Any:
    """A JSON number as JavaScript prints it: a whole number has no fraction (``50.0`` is ``50``)."""
    return int(value) if isinstance(value, float) and value.is_integer() else value


def numbers(values: Any) -> Obj:
    return {name: number(count) for name, count in values.items()}


def schedule_json(schedule: Schedule) -> Obj:
    action = schedule.action
    return {
        "scheduleId": schedule.schedule_id,
        "name": schedule.name,
        "origin": schedule.origin,
        "state": schedule.state,
        "revision": schedule.revision,
        "exhausted": schedule.exhausted,
        "action": {
            "workflowId": action.workflow_id,
            "versionPolicy": action.version_policy,
            "versionId": action.version_id,
            "inputDigest": action.input_digest,
            "nominalTimeField": action.nominal_time_field,
            "runTimeoutMs": number(action.run_timeout_ms),
        },
        "note": schedule.note,
        "noteBy": schedule.note_by,
        "remainingActions": number(schedule.remaining_actions),
        "nextActionEpochMs": epoch(schedule.next_action_at),
        "createdAt": instant(schedule.created_at),
        "updatedAt": instant(schedule.updated_at),
    }


def action_json(record: ScheduleActionRecord) -> Obj:
    return {
        "kind": record.kind,
        "origin": record.origin,
        "revision": record.revision,
        "outcome": record.outcome,
        "reason": record.reason,
        "runId": record.run_id,
        "backfillId": record.backfill_id,
        "nominalTime": instant(record.nominal_time),
        "actualTime": instant(record.actual_time),
        "summaryCount": number(record.summary_count),
        "summaryLastNominalTime": instant(record.summary_last_nominal_time),
        "decidedAt": instant(record.decided_at),
    }


def description_json(description: ScheduleDescription) -> Obj:
    return {
        **schedule_json(description),
        "nextActions": [
            {"nominalTimeEpochMs": epoch(entry.nominal_time), "actualTimeEpochMs": epoch(entry.actual_time)}
            for entry in description.next_actions
        ],
        "recentActions": [action_json(record) for record in description.recent_actions],
        "openRuns": list(description.open_runs),
        "bufferedCount": number(description.buffered_count),
        "counters": numbers(description.counters),
    }


def backfill_json(backfill: ScheduleBackfill) -> Obj:
    return {
        "backfillId": backfill.backfill_id,
        "start": instant(backfill.start),
        "end": instant(backfill.end),
        "cursor": instant(backfill.cursor),
        "state": backfill.state,
        "overlap": backfill.overlap,
        "counts": numbers(backfill.counts),
    }


def job_json(job: BatchJob) -> Obj:
    counts = job.counts
    return {
        "batchId": job.batch_id,
        "action": job.action,
        "state": job.state,
        "failureReason": job.failure_reason,
        "createdBy": job.created_by,
        "hasPayload": job.has_payload,
        "maxItems": number(job.max_items),
        "itemsPerSecond": number(job.items_per_second),
        "counts": {
            "selected": number(counts.selected),
            "succeeded": number(counts.succeeded),
            "skipped": number(counts.skipped),
            "failed": number(counts.failed),
            "cancelled": number(counts.cancelled),
            "deferred": number(counts.deferred),
        },
        "createdAt": instant(job.created_at),
        "startedAt": instant(job.started_at),
        "finishedAt": instant(job.finished_at),
        "expiresAt": instant(job.expires_at),
    }


def reset_json(reset: Reset) -> Obj:
    return {
        "resetId": reset.reset_id,
        "sourceRunId": reset.source_run_id,
        "sequence": reset.sequence,
        "state": reset.state,
        "failureReason": reset.failure_reason,
        "newRunId": reset.new_run_id,
        "reapplyReport": list(reset.reapply_report),
        "createdAt": instant(reset.created_at),
    }


def receipt_json(receipt: UpdateWithStartReceipt | None) -> Obj | None:
    if receipt is None:
        return None
    return {
        "operation": receipt.operation,
        "updateKey": receipt.update_key,
        "status": receipt.status,
        "result": receipt.result,
        "beforeStateRevision": receipt.before_state_revision,
        "afterStateRevision": receipt.after_state_revision,
        "runVersion": receipt.run_version,
        "replayed": receipt.replayed,
    }


# The requests of feature 018: the scenario names them as the wire does (camelCase JSON); the SDK takes dataclasses.


def ranges_of(raw: Any) -> list[ScheduleRange] | None:
    if raw is None:
        return None
    return [ScheduleRange(start=item["start"], end=item.get("end"), step=item.get("step")) for item in raw]


def calendar_of(raw: Obj) -> ScheduleCalendarSpec:
    return ScheduleCalendarSpec(
        second=ranges_of(raw.get("second")),
        minute=ranges_of(raw.get("minute")),
        hour=ranges_of(raw.get("hour")),
        day_of_month=ranges_of(raw.get("dayOfMonth")),
        month=ranges_of(raw.get("month")),
        year=ranges_of(raw.get("year")),
        day_of_week=ranges_of(raw.get("dayOfWeek")),
        comment=raw.get("comment"),
    )


def spec_of(raw: Obj) -> ScheduleSpec:
    dst = raw.get("dst")
    return ScheduleSpec(
        calendars=[calendar_of(item) for item in raw["calendars"]] if "calendars" in raw else None,
        cron=raw.get("cron"),
        intervals=(
            [ScheduleInterval(every_ms=item["everyMs"], offset_ms=item.get("offsetMs")) for item in raw["intervals"]]
            if "intervals" in raw
            else None
        ),
        exclusions=[calendar_of(item) for item in raw["exclusions"]] if "exclusions" in raw else None,
        start_at=raw.get("startAt"),
        end_at=raw.get("endAt"),
        timezone=raw.get("timezone"),
        jitter_ms=raw.get("jitterMs"),
        dst=ScheduleDst(gap=dst.get("gap"), ambiguous=dst.get("ambiguous")) if dst is not None else None,
    )


def write_of(raw: Obj) -> ScheduleWrite:
    policies = raw.get("policies")
    action = raw["action"]
    return ScheduleWrite(
        name=raw["name"],
        spec=spec_of(raw["spec"]),
        action=ScheduleAction(
            workflow_id=action["workflowId"],
            input=action["input"],
            version_policy=action.get("versionPolicy"),
            version_id=action.get("versionId"),
            nominal_time_field=action.get("nominalTimeField"),
            run_timeout_ms=action.get("runTimeoutMs"),
        ),
        policies=(
            SchedulePolicies(
                overlap=policies.get("overlap"),
                catchup_window_ms=policies.get("catchupWindowMs"),
                catchup_mode=policies.get("catchupMode"),
                pause_on_failure=policies.get("pauseOnFailure"),
                buffer_limit=policies.get("bufferLimit"),
            )
            if policies is not None
            else None
        ),
        paused=raw.get("paused"),
        note=raw.get("note"),
        remaining_actions=raw.get("remainingActions"),
    )


def selector_of(raw: Obj) -> BatchSelector:
    chosen = raw.get("filter")
    return BatchSelector(
        filter=(
            BatchFilter(
                workflow_id=chosen.get("workflowId"),
                version_id=chosen.get("versionId"),
                status=chosen.get("status"),
                origin=chosen.get("origin"),
                schedule_id=chosen.get("scheduleId"),
                created_after=chosen.get("createdAfter"),
                created_before=chosen.get("createdBefore"),
                business_id_prefix=chosen.get("businessIdPrefix"),
            )
            if chosen is not None
            else None
        ),
        run_ids=raw.get("runIds"),
        query=raw.get("query"),
    )


def batch_request_of(raw: Obj) -> BatchRequest:
    signal = raw.get("signal")
    update = raw.get("update")
    return BatchRequest(
        selector=selector_of(raw["selector"]),
        action=raw["action"],
        reason=raw["reason"],
        signal=BatchSignal(name=signal["name"], payload=signal["payload"]) if signal is not None else None,
        update=BatchUpdate(name=update["name"], args=update.get("args")) if update is not None else None,
        step_id=raw.get("stepId"),
        business_id=raw.get("businessId"),
        max_items=raw.get("maxItems"),
        items_per_second=raw.get("itemsPerSecond"),
    )


def error_json(error: BaseException) -> Obj:
    if isinstance(error, SagatError):
        return error.to_dict()
    sys.stderr.write("".join(traceback.format_exception(error)))
    return {"code": "UNEXPECTED", "message": type(error).__name__}


# ------------------------------------------------------------------------------------------ workers


def behavior_handler(name: str, kind: str, behavior: Obj, log: list[Obj], measurement: Obj) -> Any:
    async def handler(run_input: Any, ctx: ActivityContext) -> Any:
        measurement["active"] += 1
        measurement["peakInflight"] = max(measurement["peakInflight"], measurement["active"])
        measurement["deliveries"].append(
            {"runId": ctx.run_id, "node": ctx.node, "takenAtUnixMs": int(_now().timestamp() * 1000)}
        )
        outcome = "returned"
        try:
            # `holdUntilLogged`: a scoped instance waits until the worker has logged that many finished
            # attempts, which orders completions by what happened rather than by how long anything took.
            holds = behavior.get("holdUntilLogged")
            hold_until = holds[ctx.index or 0] if holds else 0
            while len(log) < hold_until:
                try:
                    await asyncio.wait_for(ctx.cancellation.wait(), timeout=0.01)
                except asyncio.TimeoutError:
                    continue
                outcome = "cancelled"
                return None
            until = behavior.get("failsUntilAttempt")
            if (until is not None and ctx.attempt < until) or (until is None and behavior.get("fails")):
                outcome = "failed"
                fails = behavior["fails"]
                raise ActivityFailure(fails["code"], fails["message"], fails["retryable"])
            if behavior.get("throws") is not None:
                outcome = "threw"
                raise RuntimeError(str(behavior["throws"]))
            if behavior.get("outcomeUnknown") is not None:
                outcome = "failed"
                raise ActivityFailure.outcome_unknown(str(behavior["outcomeUnknown"]))
            if behavior.get("hang"):
                await ctx.cancellation.wait()  # the host sets the token and does no more
                outcome = "cancelled"
                if (behavior.get("onCancel") or {}).get("raises"):
                    raise asyncio.CancelledError  # the handler's own cancellation error
                return None
            if behavior.get("sleepMs") is not None and behavior.get("ignoresCancel"):
                await asyncio.sleep(behavior["sleepMs"] / 1000)  # never looks at its token
            elif behavior.get("sleepMs") is not None:
                try:
                    await asyncio.wait_for(ctx.cancellation.wait(), timeout=behavior["sleepMs"] / 1000)
                except asyncio.TimeoutError:
                    pass  # the sleep ran its course
                else:
                    outcome = "cancelled"
                    if (behavior.get("onCancel") or {}).get("raises"):
                        raise asyncio.CancelledError
                    return (behavior.get("onCancel") or {}).get("returns")
            if behavior.get("readsSecret") is not None:
                try:
                    secrets = await ctx.secrets()
                except SagatError as error:
                    # `reportsSecretError`: the refusal's canonical code is the output, so the scenario can compare it.
                    if behavior.get("reportsSecretError"):
                        return {**(behavior.get("returns") or {}), "secretError": error.code}
                    raise
                return {**(behavior.get("returns") or {}), "secretLength": len(secrets.get(behavior["readsSecret"]))}
            if behavior.get("blobRoundTripBytes") is not None:
                body = b"x" * behavior["blobRoundTripBytes"]
                descriptor = await ctx.write_blob(
                    [body],
                    content_type="application/octet-stream",
                    expected_size=len(body),
                    expected_sha256=hashlib.sha256(body).hexdigest(),
                )
                read = bytearray()
                async for chunk in ctx.read_blob(descriptor):
                    read.extend(chunk)
                if read != body:
                    raise RuntimeError("blob round trip failed")
                return {"blobSize": descriptor.size, "blobSha256": descriptor.sha256, "roundTrip": True}
            if behavior.get("blobInputBytes") is not None:
                raw = run_input["$blob"]
                descriptor = BlobDescriptor(raw["id"], raw["size"], raw["sha256"], raw["contentType"])
                read = bytearray()
                async for chunk in ctx.read_blob(descriptor):
                    read.extend(chunk)
                if read != b"i" * behavior["blobInputBytes"]:
                    raise RuntimeError("run-input blob did not match")
                return {"inputBlobSize": descriptor.size, "inputBlobSha256": descriptor.sha256}
            if behavior.get("blobOutputBytes") is not None:
                descriptor = await ctx.write_blob(
                    [b"y" * behavior["blobOutputBytes"]], content_type="application/octet-stream"
                )
                return BlobOutput(descriptor)
            if behavior.get("blobInlineBytes") is not None:
                descriptor = await ctx.write_blob(
                    [b"z" * behavior["blobInlineBytes"]], content_type="application/octet-stream"
                )
                return {
                    "$blob": {
                        "id": descriptor.id,
                        "size": descriptor.size,
                        "sha256": descriptor.sha256,
                        "contentType": descriptor.content_type,
                    }
                }
            if behavior.get("verdict") is not None:
                verdict = behavior["verdict"]
                if verdict == "applied":
                    return Verdict.applied(behavior["output"]) if "output" in behavior else Verdict.applied()
                return Verdict.not_applied() if verdict == "not_applied" else Verdict.unknown()
            if behavior.get("echo"):
                return {"input": run_input, "steps": dict(ctx.steps)}
            if behavior.get("echoInput"):
                return run_input
            if behavior.get("returnsText") is not None:
                return "x" * behavior["returnsText"]
            return behavior.get("returns")
        finally:
            measurement["active"] -= 1
            log.append(
                {
                    "activity": name,
                    "kind": kind,
                    "node": ctx.node,
                    "attempt": ctx.attempt,
                    **(
                        {
                            "instance": {
                                "instancePath": [
                                    {
                                        "scopeNodeId": segment.scope_node_id,
                                        "iterationIndex": segment.iteration_index,
                                    }
                                    for segment in ctx.instance.instance_path
                                ],
                                "definitionNodeId": ctx.instance.definition_node_id,
                            }
                        }
                        if ctx.instance is not None
                        else {}
                    ),
                    **({"scopeInput": {"item": ctx.item, "index": ctx.index}} if ctx.index is not None else {}),
                    "input": run_input,
                    "taskKind": ctx.task_kind,
                    "targetNode": ctx.target_node,
                    "targetIdempotencyKey": ctx.target_idempotency_key,
                    "idempotencyKey": ctx.idempotency_key,
                    "traceparentPresent": ctx.traceparent != "",
                    "traceId": trace_id_of(ctx.traceparent) or "",
                    "steps": dict(ctx.steps),
                    "deadlineAhead": ctx.deadline > _now(),
                    "abortedReason": ctx.cancel_reason,
                    "outcome": outcome,
                }
            )

    return handler


def _now() -> Any:
    from datetime import datetime, timezone

    return datetime.now(timezone.utc)


def host_of(definition: Obj, config: Obj, plan: FaultPlan, log: list[Obj], measurement: Obj) -> WorkerHost:
    heartbeat = definition.get("heartbeatIntervalMs")
    token = config["unboundWorkerToken"] if definition.get("token") == "unbound" else config["workerToken"]
    host = WorkerHost(
        WorkerConfig(
            grpc_url=config["grpcUrl"],
            namespace=config["namespace"],
            token=token,
            worker_name=definition["workerName"],
            max_inflight=definition.get("maxInflight", 4),
            protocol_version=definition.get("protocolVersion"),
            transport=definition.get("transport", "auto"),
            heartbeat_interval=heartbeat / 1000 if heartbeat else None,
            interceptors=plan.interceptors(),
            retry=RetryPolicy(initial_delay=0.02, max_delay=0.2),
        )
    )
    for name, activity in definition["activities"].items():
        kind = activity.get("as") or "activity"
        handler = behavior_handler(name, kind, activity.get("behavior") or {}, log, measurement)
        if kind == "compensation":
            host.register_compensation(name, handler)
        elif kind == "reconciler":
            host.register_reconciler(name, handler)
        else:
            host.register_activity(name, handler)
    return host


# --------------------------------------------------------------------------------------------- run


class Progress:
    """The step being played, so that a scenario that does not finish says where it stopped."""

    step = "(not started)"


async def run(scenario: Obj, config: Obj) -> Obj:
    unique: str = config["unique"]
    plan = FaultPlan([Fault.of(raw) for raw in scenario.get("faults") or []])
    api_path = urlsplit(config["apiUrl"]).path.rstrip("/")

    def client_for(token: str) -> SagatClient:
        return SagatClient(
            ClientConfig(
                api_url=config["apiUrl"],
                namespace=config["namespace"],
                token=token,
                retry=RetryPolicy(initial_delay=0.02, max_delay=0.2),
                transport=plan.transport(api_path),
            )
        )

    clients = {
        "default": client_for(config["clientToken"]),
        "bad": client_for(config["badToken"]),
        "other": client_for(config["otherTenantToken"]),
        # Members of the tenant with one role each, the worker identity, and an operator the scenario revokes.
        "worker": client_for(config["workerToken"]),
        "operator": client_for(config["operatorToken"]),
        "viewer": client_for(config["viewerToken"]),
        "runner": client_for(config["runnerToken"]),
        "revoked": client_for(config["revokedToken"]),
    }
    logs: dict[str, list[Obj]] = {}
    measurements: dict[str, Obj] = {}
    hosts: dict[str, tuple[Obj, list[WorkerHost]]] = {}
    for key, definition in (scenario.get("workers") or {}).items():
        logs[key] = []
        measurements[key] = {"active": 0, "peakInflight": 0, "deliveries": []}
        hosts[key] = (definition, [])

    async def start_worker(key: str) -> Obj:
        if key not in hosts:
            raise ValueError(f"no worker {key}")
        definition, started = hosts[key]
        host = host_of(definition, config, plan, logs[key], measurements[key])
        started.append(host)
        await host.start()
        return {"protocolVersion": host.protocol_version}

    observations: list[Obj] = []
    results: dict[str, Obj] = {}

    def run_of(step: Obj) -> str:
        if isinstance(step.get("page"), str):
            # A segment of a page of listChainRuns, by position.
            items = results.get(step["page"], {}).get("items")
            index = int(step.get("index", 0))
            if not isinstance(items, list) or not 0 <= index < len(items) or not isinstance(items[index], dict):
                raise ValueError(f"step {step['page']} has no segment {index}")
            return str(items[index]["runId"])
        # A rerun answers `newRunId` and no `runId`: the step then names the run it started.
        named = results.get(step["run"], {})
        found = named.get("runId", named.get("newRunId"))
        if not isinstance(found, str):
            raise ValueError(f"step {step['run']} has no run")
        return found

    def approval_of(step: Obj) -> str:
        direct = step.get("approvalId")
        if isinstance(direct, str):
            return direct
        items = results.get(str(step.get("approval")), {}).get("items")
        if not isinstance(items, list) or not items or not isinstance(items[0], dict):
            raise ValueError(f"step {step.get('approval')} has no approval")
        approval_id = items[0].get("approvalId")
        if not isinstance(approval_id, str):
            raise ValueError(f"step {step.get('approval')} has no approval")
        return approval_id

    def chain_ref(value: str) -> str:
        if value == "{{namespaceId}}":
            return str(config["namespaceId"])
        for prefix, field in (("{{chainId:", "runId"), ("{{currentRunId:", "currentRunId")):
            if value.startswith(prefix) and value.endswith("}}"):
                source = value[len(prefix) : -2]
                resolved = results.get(source, {}).get(field)
                if not isinstance(resolved, str):
                    raise ValueError(f"step {source} has no {field}")
                return resolved
        return value

    namespace_id = str(config["namespaceId"])

    # References inside the parameters of a step: a value an earlier step produced (`{{step:<id>:<field>}}`), a
    # workflow id (`{{workflow:<key>}}`), the namespace id, and instants computed when the step runs
    # (`{{now:<offsetMs>}}`, and `{{hour:<offsetHours>}}` for the start of a UTC hour). A string that is exactly one
    # reference becomes its value.
    def refs(value: Any) -> Any:
        if isinstance(value, str):
            if value == "{{namespaceId}}":
                return config["namespaceId"]
            if found := re.fullmatch(r"\{\{workflow:([^}]+)\}\}", value):
                return config["workflows"][found[1]]["workflowId"]
            if found := re.fullmatch(r"\{\{step:([^:}]+):([^}]+)\}\}", value):
                produced = results.get(found[1], {})
                if found[2] not in produced:
                    raise ValueError(f"step {found[1]} has no {found[2]}")
                return produced[found[2]]
            if found := re.fullmatch(r"\{\{now:(-?\d+)\}\}", value):
                return format_instant(datetime.now(timezone.utc) + timedelta(milliseconds=int(found[1])))
            if found := re.fullmatch(r"\{\{hour:(-?\d+)\}\}", value):
                hour = datetime.now(timezone.utc).replace(minute=0, second=0, microsecond=0)
                return format_instant(hour + timedelta(hours=int(found[1])))
            return value
        if isinstance(value, list):
            return [refs(item) for item in value]
        if isinstance(value, dict):
            return {key: refs(item) for key, item in value.items()}
        return value

    def step_of_run(run_id: str) -> str:
        """The id of the step whose result holds this run, or `?` for a run no step of the scenario started."""
        for step_id, held in results.items():
            if held.get("runId") == run_id or held.get("newRunId") == run_id:
                return step_id
        return "?"

    def step_result(step_id: str, name: str) -> str:
        found = results.get(step_id, {}).get(name)
        if not isinstance(found, str):
            raise ValueError(f"step {step_id} has no {name}")
        return found

    async def awaiting(what: str, timeout_ms: float, done: Callable[[], Awaitable[bool]]) -> None:
        end = asyncio.get_running_loop().time() + timeout_ms / 1000
        while not await done():
            if asyncio.get_running_loop().time() > end:
                raise ValueError(f"timed out waiting for {what}")
            await asyncio.sleep(0.05)

    try:
        for key, (definition, _) in hosts.items():
            if definition.get("autostart") is not False:
                Progress.step = f"autostart of worker {key}"
                await start_worker(key)

        for given in scenario["steps"]:
            Progress.step = f"{given['id']} ({given['op']})"
            client = clients[given.get("token") or "default"]
            try:
                step = refs(given)
            except Exception as error:
                # A reference to a step that failed is a failure of this step, recorded like any other.
                observations.append({"id": given["id"], "op": given["op"], "ok": False, "error": error_json(error)})
                continue
            observation: Obj = {"id": step["id"], "op": step["op"]}
            op = step["op"]
            try:
                result: Obj | None = None
                if op == "start":
                    workflow_id = config["workflows"][step["workflow"]]["workflowId"]
                    deadline_in = step.get("deadlineInMs")
                    delay_ms = step.get("startDelayMs")
                    started_run = await client.start(
                        workflow_id,
                        step.get("input") or {},
                        idempotency_key=step.get("idempotencyKey"),
                        correlation_id=step.get("correlationId"),
                        traceparent=step.get("traceparent"),
                        deadline_in=deadline_in / 1000 if deadline_in is not None else None,
                        business_id=step.get("businessId"),
                        business_id_conflict_policy=step.get("businessIdConflictPolicy"),
                        business_id_reuse_policy=step.get("businessIdReusePolicy"),
                        start_delay=timedelta(milliseconds=delay_ms) if delay_ms is not None else None,
                    )
                    # `started` only when the step asked for a business identifier: the other observations stay as
                    # they were.
                    result = run_json(started_run)
                    if "businessId" in step:
                        result["started"] = started_run.started
                elif op == "scheduleCreate":
                    result = schedule_json(
                        await client.schedules.create(
                            namespace_id, write_of(step["write"]), idempotency_key=step.get("idempotencyKey")
                        )
                    )
                elif op == "scheduleGet":
                    result = description_json(
                        await client.schedules.get(
                            namespace_id,
                            step_result(step["schedule"], "scheduleId"),
                            next_count=step.get("nextCount"),
                            recent_count=step.get("recentCount"),
                        )
                    )
                elif op == "scheduleList":
                    names: list[Obj] = []
                    wanted = step.get("filter") or {}
                    list_cursor: str | None = None
                    if step.get("all"):
                        async for schedule in client.schedules.list_all(
                            namespace_id,
                            limit=step.get("limit"),
                            state=wanted.get("state"),
                            origin=wanted.get("origin"),
                            workflow_id=wanted.get("workflowId"),
                            name=wanted.get("name"),
                        ):
                            names.append(
                                {
                                    "name": schedule.name,
                                    "state": schedule.state,
                                    "origin": schedule.origin,
                                    "revision": schedule.revision,
                                }
                            )
                    else:
                        schedules = await client.schedules.list(
                            namespace_id,
                            limit=step.get("limit"),
                            state=wanted.get("state"),
                            origin=wanted.get("origin"),
                            workflow_id=wanted.get("workflowId"),
                            name=wanted.get("name"),
                        )
                        names.extend(
                            {"name": item.name, "state": item.state, "origin": item.origin, "revision": item.revision}
                            for item in schedules.items
                        )
                        list_cursor = schedules.next_cursor
                    result = {"items": names, "nextCursor": None if list_cursor is None else "<cursor>"}
                elif op == "scheduleUpdate":
                    result = schedule_json(
                        await client.schedules.update(
                            namespace_id,
                            step_result(step["schedule"], "scheduleId"),
                            write_of(step["write"]),
                            step["expectedRevision"],
                            idempotency_key=step.get("idempotencyKey"),
                        )
                    )
                elif op == "scheduleDelete":
                    await client.schedules.delete(
                        namespace_id,
                        step_result(step["schedule"], "scheduleId"),
                        step["expectedRevision"],
                        idempotency_key=step.get("idempotencyKey"),
                    )
                    result = {"deleted": True}
                elif op == "schedulePause":
                    result = schedule_json(
                        await client.schedules.pause(
                            namespace_id,
                            step_result(step["schedule"], "scheduleId"),
                            step["note"],
                            idempotency_key=step.get("idempotencyKey"),
                        )
                    )
                elif op == "scheduleResume":
                    result = schedule_json(
                        await client.schedules.resume(
                            namespace_id,
                            step_result(step["schedule"], "scheduleId"),
                            step["note"],
                            idempotency_key=step.get("idempotencyKey"),
                        )
                    )
                elif op == "scheduleTrigger":
                    result = action_json(
                        await client.schedules.trigger(
                            namespace_id,
                            step_result(step["schedule"], "scheduleId"),
                            overlap=step.get("overlap"),
                            idempotency_key=step.get("idempotencyKey"),
                        )
                    )
                elif op == "scheduleBackfill":
                    result = backfill_json(
                        await client.schedules.backfill(
                            namespace_id,
                            step_result(step["schedule"], "scheduleId"),
                            step["start"],
                            step["end"],
                            overlap=step.get("overlap"),
                            idempotency_key=step.get("idempotencyKey"),
                        )
                    )
                elif op == "scheduleCancelBackfill":
                    result = backfill_json(
                        await client.schedules.cancel_backfill(
                            namespace_id,
                            step_result(step["schedule"], "scheduleId"),
                            step_result(step["backfill"], "backfillId"),
                            idempotency_key=step.get("idempotencyKey"),
                        )
                    )
                elif op == "scheduleBackfills":
                    result = {
                        "items": [
                            backfill_json(item)
                            for item in await client.schedules.list_backfills(
                                namespace_id, step_result(step["schedule"], "scheduleId")
                            )
                        ]
                    }
                elif op == "scheduleActions":
                    actions = await client.schedules.list_actions(
                        namespace_id,
                        step_result(step["schedule"], "scheduleId"),
                        outcome=step.get("outcome"),
                        limit=step.get("limit"),
                    )
                    result = {
                        "items": [action_json(item) for item in actions.items],
                        "nextCursor": None if actions.next_cursor is None else "<cursor>",
                    }
                elif op == "awaitSchedule":
                    # The scheduler decides the times of a backfill: wait for its counters, never for a delay.
                    schedule_id = step_result(step["schedule"], "scheduleId")

                    async def counted(
                        client: SagatClient = client, schedule_id: str = schedule_id, step: Obj = step
                    ) -> bool:
                        described = await client.schedules.get(namespace_id, schedule_id, next_count=0, recent_count=0)
                        return bool(described.counters.get(step["counter"], 0) >= step.get("atLeast", 1))

                    await awaiting(f"schedule counter {step['counter']}", step.get("timeoutMs", 30_000), counted)
                elif op == "awaitBackfill":
                    # A backfill counts the times it found already acted on in its own counts, not in the schedule's.
                    backfill_schedule = step_result(step["schedule"], "scheduleId")
                    backfill_id = step_result(step["backfill"], "backfillId")

                    async def reached(
                        client: SagatClient = client,
                        schedule_id: str = backfill_schedule,
                        backfill_id: str = backfill_id,
                        step: Obj = step,
                    ) -> bool:
                        listed = await client.schedules.list_backfills(namespace_id, schedule_id)
                        return any(item.backfill_id == backfill_id and item.state == step["state"] for item in listed)

                    await awaiting(f"backfill state {step['state']}", step.get("timeoutMs", 30_000), reached)
                elif op == "businessIdGet":
                    binding = await client.business_ids.get(namespace_id, step["businessId"])
                    result = {
                        "businessId": binding.business_id,
                        "executionId": binding.execution_id,
                        "currentRunId": binding.current_run_id,
                        "open": binding.open,
                        "lastStatus": binding.last_status,
                        "workflowId": binding.workflow_id,
                        "boundAt": instant(binding.bound_at),
                        "reusableAfter": instant(binding.reusable_after),
                    }
                elif op == "signalWithStart":
                    signalled_with_start = await client.signal_with_start(
                        config["workflows"][step["workflow"]]["workflowId"]
                        if step.get("workflow")
                        else step["workflowId"],
                        step["businessId"],
                        step.get("input") or {},
                        step["signal"]["name"],
                        step["signal"].get("payload") or {},
                        idempotency_key=step.get("idempotencyKey"),
                        business_id_reuse_policy=step.get("businessIdReusePolicy"),
                    )
                    result = {
                        "runId": signalled_with_start.run_id,
                        "started": signalled_with_start.started,
                        "deliveredTo": list(signalled_with_start.delivered_to),
                    }
                elif op == "updateWithStart":
                    declared = step["update"]
                    delay_ms = step.get("startDelayMs")
                    updated_with_start = await client.update_with_start(
                        config["workflows"][step["workflow"]]["workflowId"],
                        step["businessId"],
                        step.get("input") or {},
                        declared["name"],
                        declared["updateKey"],
                        declared.get("args"),
                        expected_state_revision=declared.get("expectedStateRevision"),
                        idempotency_key=step.get("idempotencyKey"),
                        business_id_conflict_policy=step.get("businessIdConflictPolicy"),
                        business_id_reuse_policy=step.get("businessIdReusePolicy"),
                        start_delay=timedelta(milliseconds=delay_ms) if delay_ms is not None else None,
                    )
                    result = {
                        "runId": updated_with_start.run_id,
                        "started": updated_with_start.started,
                        "update": {
                            "outcome": updated_with_start.update.outcome,
                            "reason": updated_with_start.update.reason,
                            "receipt": receipt_json(updated_with_start.update.receipt),
                        },
                    }
                elif op == "batchPreview":
                    preview = await client.batch.preview(
                        namespace_id, selector_of(step["selector"]), step.get("maxItems")
                    )
                    # The sample is a set of runs in the order the server chose; the scenario compares which of its
                    # own runs it holds.
                    result = {
                        "matched": preview.matched,
                        "capped": preview.capped,
                        "sampleSize": len(preview.sample),
                        "sampleSteps": sorted(step_of_run(item) for item in preview.sample),
                    }
                elif op == "batchCreate":
                    # What does not change while the job runs (its state and counters do): the job as it was accepted.
                    created = await client.batch.create(
                        namespace_id, batch_request_of(step["request"]), idempotency_key=step.get("idempotencyKey")
                    )
                    result = {
                        "batchId": created.batch_id,
                        "action": created.action,
                        "createdBy": created.created_by,
                        "hasPayload": created.has_payload,
                        "maxItems": number(created.max_items),
                        "itemsPerSecond": number(created.items_per_second),
                    }
                elif op == "batchGet":
                    result = job_json(await client.batch.get(namespace_id, step_result(step["batch"], "batchId")))
                elif op == "batchList":
                    jobs = await client.batch.list(namespace_id, state=step.get("state"), limit=step.get("limit"))
                    result = {
                        "items": [
                            {
                                "action": job.action,
                                "state": job.state,
                                "counts": {
                                    "selected": number(job.counts.selected),
                                    "succeeded": number(job.counts.succeeded),
                                    "skipped": number(job.counts.skipped),
                                    "failed": number(job.counts.failed),
                                    "cancelled": number(job.counts.cancelled),
                                    "deferred": number(job.counts.deferred),
                                },
                            }
                            for job in jobs.items
                        ],
                        "nextCursor": None if jobs.next_cursor is None else "<cursor>",
                    }
                elif op == "batchCancel":
                    result = job_json(
                        await client.batch.cancel(
                            namespace_id,
                            step_result(step["batch"], "batchId"),
                            step["reason"],
                            idempotency_key=step.get("idempotencyKey"),
                        )
                    )
                elif op == "batchItems":
                    batch_items = await client.batch.list_items(
                        namespace_id,
                        step_result(step["batch"], "batchId"),
                        state=step.get("state"),
                        limit=step.get("limit"),
                    )
                    # The order of the items is the server's; which of the scenario's runs sits at which ordinal is not
                    # part of the contract, so the items are named by the step that started the run and sorted by it.
                    result = {
                        "items": sorted(
                            (
                                {
                                    "runStep": step_of_run(item.run_id),
                                    "state": item.state,
                                    "reason": item.reason,
                                    "errorCode": item.error_code,
                                    "attempts": item.attempts,
                                    "newRunId": item.new_run_id,
                                }
                                for item in batch_items.items
                            ),
                            key=lambda entry: str(entry["runStep"]),
                        ),
                        "nextCursor": None if batch_items.next_cursor is None else "<cursor>",
                    }
                elif op == "awaitBatch":
                    batch_id = step_result(step["batch"], "batchId")

                    async def in_state(
                        client: SagatClient = client, batch_id: str = batch_id, step: Obj = step
                    ) -> bool:
                        return bool((await client.batch.get(namespace_id, batch_id)).state == step["state"])

                    await awaiting(f"batch state {step['state']}", step.get("timeoutMs", 30_000), in_state)
                elif op == "resetPoints":
                    points = await client.reset_points(run_of(step), limit=step.get("limit"))
                    result = {
                        "items": [
                            {
                                "sequence": point.sequence,
                                "kind": point.kind,
                                "node": point.node,
                                "occurredAt": instant(point.occurred_at),
                            }
                            for point in points.items
                        ],
                        "nextCursor": None if points.next_cursor is None else "<cursor>",
                    }
                elif op == "resetPreview":
                    reset_plan = await client.preview_reset(run_of(step), step["sequence"], step.get("reapply"))
                    result = {
                        "sequence": reset_plan.sequence,
                        "restored": [{"node": item.node, "kind": item.kind} for item in reset_plan.restored],
                        "reexecuted": [
                            {
                                "node": item.node,
                                "completedInSource": item.completed_in_source,
                                "markers": list(item.markers),
                            }
                            for item in reset_plan.reexecuted
                        ],
                        "reapply": list(reset_plan.reapply),
                        "requiresAcknowledgement": reset_plan.requires_acknowledgement,
                        "prefixEvents": reset_plan.prefix_events,
                    }
                elif op == "reset":
                    result = reset_json(
                        await client.reset(
                            run_of(step),
                            sequence=step["sequence"],
                            reason=step["reason"],
                            reapply=step.get("reapply"),
                            source_policy=step.get("sourcePolicy"),
                            business_id=step.get("businessId"),
                            acknowledge_reexecution=step.get("acknowledgeReexecution"),
                            idempotency_key=step.get("idempotencyKey"),
                        )
                    )
                elif op == "resets":
                    result = {"items": [reset_json(item) for item in await client.list_resets(run_of(step))]}
                elif op == "getReset":
                    result = reset_json(await client.get_reset(run_of(step), step_result(step["reset"], "resetId")))
                elif op == "startWithBlob":
                    workflow_id = config["workflows"][step["workflow"]]["workflowId"]
                    blob_body = (
                        step["inputBlobText"].encode("utf-8")
                        if "inputBlobText" in step
                        else b"i" * step["inputBlobBytes"]
                    )
                    descriptor = await client.upload_run_input(
                        [blob_body], expected_size=len(blob_body), expected_sha256=hashlib.sha256(blob_body).hexdigest()
                    )
                    result = run_json(
                        await client.start_with_blob(
                            workflow_id,
                            descriptor,
                            idempotency_key=step.get("idempotencyKey"),
                            correlation_id=step.get("correlationId"),
                            traceparent=step.get("traceparent"),
                            deadline_in=(step["deadlineInMs"] / 1000 if step.get("deadlineInMs") is not None else None),
                        )
                    )
                elif op == "workflowByName":
                    name = config["workflows"][step["workflow"]]["name"]
                    result = run_json(
                        await client.start(name, step.get("input") or {}, idempotency_key=step.get("idempotencyKey"))
                    )
                elif op == "get":
                    result = run_json(await client.get(run_of(step)))
                elif op == "list":
                    ids: list[str] = []
                    if step.get("all"):
                        async for item in client.list_all(status=step.get("status"), limit=step.get("limit")):
                            ids.append(item.run_id)
                    else:
                        page = await client.list(status=step.get("status"), limit=step.get("limit"))
                        ids.extend(item.run_id for item in page.items)
                        # `waitForCount`: the runs of a chain are made by the scheduler, so a scenario that needs a
                        # hand-off to have happened waits for it by the count of runs in a status.
                        for _ in range(100):
                            if len(ids) >= step.get("waitForCount", 0):
                                break
                            await asyncio.sleep(0.05)
                            page = await client.list(status=step.get("status"), limit=step.get("limit"))
                            ids = [item.run_id for item in page.items]
                    # In the order the server gave (newest first): the SDK keeps it, also across pages. `countOnly`
                    # answers the number alone, for runs whose ids are derived from their parent's and so sort anywhere.
                    result = {"count": len(ids)} if step.get("countOnly") else {"items": ids}
                elif op == "listApprovals":
                    approval_page = await client.list_approvals(
                        run_of(step),
                        cursor=step.get("cursor"),
                        limit=step.get("limit"),
                        traceparent=step.get("traceparent"),
                    )
                    for _ in range(100):
                        if not step.get("waitForItem") or approval_page.items:
                            break
                        await asyncio.sleep(0.05)
                        approval_page = await client.list_approvals(
                            run_of(step),
                            cursor=step.get("cursor"),
                            limit=step.get("limit"),
                            traceparent=step.get("traceparent"),
                        )
                    result = {
                        "items": [
                            {
                                "approvalId": item.approval_id,
                                "runId": item.run_id,
                                "nodeId": item.node_id,
                                "instancePath": [
                                    {"scopeNodeId": scope.scope_node_id, "iterationIndex": scope.iteration_index}
                                    for scope in item.instance_path
                                ],
                                "status": item.status,
                                "deadline": item.deadline.isoformat(timespec="milliseconds").replace("+00:00", "Z"),
                                "settledAt": item.settled_at.isoformat(timespec="milliseconds").replace("+00:00", "Z")
                                if item.settled_at
                                else None,
                            }
                            for item in approval_page.items
                        ],
                        "nextCursor": approval_page.next_cursor,
                    }
                elif op == "cancel":
                    cancelled = await client.cancel(
                        run_of(step), step["reason"], expected_run_version=step.get("expectedRunVersion")
                    )
                    result = {"accepted": cancelled.accepted, "runStatus": cancelled.run_status}
                elif op == "operate":
                    operated = await client.operate(
                        run_of(step),
                        step["action"],
                        step["reason"],
                        step_id=step.get("stepId"),
                        outcome=step.get("outcome"),
                        error_code=step.get("errorCode"),
                    )
                    result = {"accepted": operated.accepted, "runStatus": operated.run_status}
                    # `newRunId` only for the action that starts a run, so the other operations observe as they did.
                    if operated.new_run_id is not None:
                        result["newRunId"] = operated.new_run_id
                elif op == "decideApproval":
                    receipt = await client.decide_approval(
                        approval_of(step),
                        step["decision"],
                        decision_key=step["decisionKey"],
                        reason_code=step.get("reasonCode"),
                        traceparent=step.get("traceparent"),
                    )
                    result = {
                        "receiptId": receipt.receipt_id,
                        "approvalId": receipt.approval_id,
                        "decisionKey": receipt.decision_key,
                        "decision": receipt.decision,
                        "reasonCode": receipt.reason_code,
                        "actor": receipt.actor,
                        "decidedAt": receipt.decided_at.isoformat(timespec="milliseconds").replace("+00:00", "Z"),
                        "outcomeSequence": receipt.outcome_sequence,
                        "replayed": receipt.replayed,
                    }
                elif op == "query":
                    queried = await client.query(
                        run_of(step), step["name"], step.get("args"), traceparent=step.get("traceparent")
                    )
                    result = {
                        "runVersion": queried.run_version,
                        "stateRevision": queried.state_revision,
                        "result": queried.result,
                    }
                elif op == "update":
                    updated = await client.update(
                        run_of(step),
                        step["name"],
                        step.get("args"),
                        update_key=step["updateKey"],
                        expected_state_revision=step.get("expectedStateRevision"),
                        traceparent=step.get("traceparent"),
                    )
                    result = {
                        "operation": updated.operation,
                        "updateKey": updated.update_key,
                        "status": updated.status,
                        "result": updated.result,
                        "beforeStateRevision": updated.before_state_revision,
                        "afterStateRevision": updated.after_state_revision,
                        "runVersion": updated.run_version,
                        "replayed": updated.replayed,
                    }
                elif op == "queryChain":
                    queried = await client.query_chain(
                        chain_ref(step["namespaceId"]),
                        chain_ref(step["chainId"]),
                        step["name"],
                        step.get("args"),
                        expected_run_id=(chain_ref(step["expectedRunId"]) if step.get("expectedRunId") else None),
                        traceparent=step.get("traceparent"),
                    )
                    result = {
                        "runId": queried.run_id,
                        "runVersion": queried.run_version,
                        "stateRevision": queried.state_revision,
                        "result": queried.result,
                    }
                elif op == "updateChain":
                    updated = await client.update_chain(
                        chain_ref(step["namespaceId"]),
                        chain_ref(step["chainId"]),
                        step["name"],
                        step.get("args"),
                        update_key=step["updateKey"],
                        expected_run_id=(chain_ref(step["expectedRunId"]) if step.get("expectedRunId") else None),
                        expected_state_revision=step.get("expectedStateRevision"),
                        traceparent=step.get("traceparent"),
                    )
                    result = {
                        "runId": updated.run_id,
                        "operation": updated.operation,
                        "updateKey": updated.update_key,
                        "status": updated.status,
                        "result": updated.result,
                        "beforeStateRevision": updated.before_state_revision,
                        "afterStateRevision": updated.after_state_revision,
                        "runVersion": updated.run_version,
                        "replayed": updated.replayed,
                    }
                elif op == "listChainRuns":
                    cursor = step.get("cursor")
                    if isinstance(cursor, str) and cursor.startswith("{{nextCursor:") and cursor.endswith("}}"):
                        cursor = results.get(cursor[len("{{nextCursor:") : -2], {}).get("nextCursor")
                        if not isinstance(cursor, str):
                            raise ValueError("the referenced step has no next page")
                    chain_page = await client.list_chain_runs(
                        chain_ref(step["namespaceId"]),
                        chain_ref(step["chainId"]),
                        cursor=cursor,
                        limit=step.get("limit"),
                        traceparent=step.get("traceparent"),
                    )
                    result = {
                        "items": [
                            {
                                "index": segment.index,
                                "runId": segment.run_id,
                                "status": segment.status,
                                "completionKind": segment.completion_kind,
                                "predecessorRunId": segment.predecessor_run_id,
                                "successorRunId": segment.successor_run_id,
                                "createdAt": instant(segment.created_at),
                                "finishedAt": instant(segment.finished_at),
                            }
                            for segment in chain_page.items
                        ],
                        "nextCursor": chain_page.next_cursor,
                    }
                elif op == "inspectChain":
                    chain = await client.inspect_chain(
                        chain_ref(step["namespaceId"]), chain_ref(step["chainId"]), traceparent=step.get("traceparent")
                    )
                    if step.get("waitForHeadChange"):
                        previous = chain_ref(step["waitForHeadChange"])
                        for _ in range(100):
                            if chain.current_run_id != previous:
                                break
                            await asyncio.sleep(0.05)
                            chain = await client.inspect_chain(
                                chain_ref(step["namespaceId"]),
                                chain_ref(step["chainId"]),
                                traceparent=step.get("traceparent"),
                            )
                    result = {
                        "chainId": chain.chain_id,
                        "currentRunId": chain.current_run_id,
                        "revision": chain.revision,
                        "cancelRequested": chain.cancel_requested,
                        "finalOutcome": chain.final_outcome,
                        "segments": [
                            {
                                "index": item.index,
                                "runId": item.run_id,
                                "predecessorRunId": item.predecessor_run_id,
                                "successorRunId": item.successor_run_id,
                            }
                            for item in chain.segments
                        ],
                    }
                elif op == "signalChain":
                    signalled = await client.signal_chain(
                        chain_ref(step["namespaceId"]),
                        chain_ref(step["chainId"]),
                        step["name"],
                        step.get("payload"),
                        expected_run_id=chain_ref(step["expectedRunId"]),
                        idempotency_key=step.get("idempotencyKey"),
                        traceparent=step.get("traceparent"),
                    )
                    result = {"deliveredTo": list(signalled.delivered_to)}
                elif op == "cancelChain":
                    await client.cancel_chain(
                        chain_ref(step["namespaceId"]),
                        chain_ref(step["chainId"]),
                        step["reason"],
                        expected_run_id=chain_ref(step["expectedRunId"]),
                        idempotency_key=step.get("idempotencyKey"),
                        traceparent=step.get("traceparent"),
                    )
                    result = {"accepted": True}
                elif op == "signal":
                    signalled = await client.signal(run_of(step), step["name"], step.get("payload") or {})
                    result = {"deliveredTo": list(signalled.delivered_to)}
                elif op == "publishEvent":
                    accepted = await client.publish_event(
                        step["type"],
                        step.get("data") or {},
                        event_id=step.get("eventId"),
                        correlation_key=step.get("correlationKey"),
                    )
                    result = {"eventId": accepted.event_id, "status": accepted.status}
                elif op == "waitForCompletion":
                    result = run_json(
                        await client.wait_for_completion(
                            run_of(step), timeout=step.get("timeoutMs", 60_000) / 1000, poll_interval=0.1
                        )
                    )
                elif op == "waitForChainCompletion":
                    chain = await client.wait_for_chain_completion(
                        chain_ref(step["namespaceId"]),
                        chain_ref(step["chainId"]),
                        timeout=step.get("timeoutMs", 60_000) / 1000,
                        poll_interval=0.1,
                    )
                    result = {
                        "chainId": chain.chain_id,
                        "currentRunId": chain.current_run_id,
                        "revision": chain.revision,
                        "cancelRequested": chain.cancel_requested,
                        "finalOutcome": chain.final_outcome,
                        "segments": [
                            {
                                "index": segment.index,
                                "runId": segment.run_id,
                                "predecessorRunId": segment.predecessor_run_id,
                                "successorRunId": segment.successor_run_id,
                            }
                            for segment in chain.segments
                        ],
                    }
                elif op == "events":
                    journal = await client.events(run_of(step), limit=200)
                    result = {
                        "events": [
                            {"type": event.event_type, "stepId": event.step_id, "attempt": event.attempt}
                            for event in journal.items
                        ]
                    }
                elif op == "sleep":
                    await asyncio.sleep(step["ms"] / 1000)
                elif op == "control":
                    body: Obj = {"action": step["action"]}
                    if step.get("run"):
                        body["run"] = run_of(step)
                    if step.get("node") is not None:
                        body["node"] = step["node"]
                    async with httpx.AsyncClient() as plain:
                        answered = await plain.post(f"{config['controlUrl']}/control", json=body)
                    if not answered.is_success:
                        raise ValueError(f"control {step['action']} answered {answered.status_code}")
                elif op == "startWorker":
                    result = await start_worker(step["worker"])
                elif op == "stopWorker":
                    for host in hosts[step["worker"]][1]:
                        await host.stop(step.get("timeoutMs", 5000) / 1000)
                elif op == "restartWorker":
                    host = hosts[step["worker"]][1][-1]  # the same host object, stopped and started again
                    await host.stop(step.get("timeoutMs", 5000) / 1000)
                    await host.start()
                    result = {"protocolVersion": host.protocol_version}
                elif op == "probeWorker":
                    probe = WorkerHost(
                        WorkerConfig(
                            grpc_url=config["grpcUrl"],
                            namespace=config["namespace"],
                            token=config["workerToken"],
                            worker_name=f"probe-{unique}",
                            protocol_version=step["protocolVersion"],
                            retry=RetryPolicy(max_attempts=1),
                        )
                    )
                    probe.register_activity("conformance.probe", lambda _input, _ctx: None, blocking=False)
                    try:
                        await probe.start()
                        result = {"accepted": probe.protocol_version}
                    finally:
                        await probe.stop(0.5)
                elif op == "awaitJournal":
                    run_id = run_of(step)
                    end = asyncio.get_running_loop().time() + step.get("timeoutMs", 20_000) / 1000
                    while True:
                        journal = await client.events(run_id, limit=200)
                        if any(
                            event.event_type == step["eventType"]
                            and ("stepId" not in step or event.step_id == step["stepId"])
                            for event in journal.items
                        ):
                            break
                        if asyncio.get_running_loop().time() > end:
                            raise ValueError(f"the journal of {run_id} has no {step['eventType']}")
                        await asyncio.sleep(0.025)
                elif op == "awaitWorkerLog":
                    worker_log = logs[step["worker"]]
                    end = asyncio.get_running_loop().time() + step.get("timeoutMs", 20_000) / 1000
                    while len(worker_log) < step["count"] and asyncio.get_running_loop().time() < end:
                        await asyncio.sleep(0.02)
                    if len(worker_log) < step["count"]:
                        raise ValueError(f"the worker log has {len(worker_log)} entries, not {step['count']}")
                else:
                    raise ValueError(f"unknown step op {op}")
                observation["ok"] = True
                if result is not None:
                    observation["result"] = result
                    results[step["id"]] = result
            except Exception as error:
                observation["ok"] = False
                observation["error"] = error_json(error)
            observations.append(observation)
    finally:
        Progress.step = "stopping the workers"
        for _, started in hosts.values():
            for host in started:
                try:
                    await host.stop(2.0)
                except Exception as error:
                    sys.stderr.write(f"stopping a worker failed: {type(error).__name__}\n")
        for client in clients.values():
            await client.aclose()
    return {
        "scenario": scenario["name"],
        "language": "python",
        "sdkVersion": SDK_VERSION,
        "observations": observations,
        "workerLog": logs,
        "workerMeasurements": {
            key: {"peakInflight": value["peakInflight"], "deliveries": value["deliveries"]}
            for key, value in measurements.items()
        },
        "faultsApplied": plan.applied(),
    }


# ----------------------------------------------------------------------------------------------- main


async def play(scenario: Obj, config: Obj) -> Obj:
    try:
        return await asyncio.wait_for(run(scenario, config), SCENARIO_DEADLINE)
    except asyncio.TimeoutError:
        raise SystemExit(
            f"the scenario did not finish within {SCENARIO_DEADLINE:g} s; it was at step {Progress.step}"
        ) from None


def main(argv: list[str]) -> int:
    if not argv or argv[0] not in ("build", "run") or len(argv) < 2:
        sys.stderr.write("usage: runner build <scenario> | run <scenario> <config> <out>\n")
        return 2
    mode, scenario_path = argv[0], argv[1]
    raw = read(scenario_path)
    if mode == "build":
        sys.stdout.flush()
        sys.stdout.buffer.write(
            json.dumps(build(raw), ensure_ascii=False).encode("utf-8")
        )  # UTF-8, whatever the locale
        return 0
    if len(argv) != 4:
        sys.stderr.write("usage: runner run <scenario> <config> <out>\n")
        return 2
    config = read(argv[2])
    out = asyncio.run(play(templated(raw, config["unique"]), config))
    Path(argv[3]).write_text(json.dumps(out, indent=2, ensure_ascii=False), encoding="utf-8")
    return 0


if __name__ == "__main__":
    try:
        code = main(sys.argv[1:])
    except SystemExit:
        raise
    except BaseException:
        traceback.print_exc()
        code = 1
    sys.stdout.flush()
    sys.exit(code)
