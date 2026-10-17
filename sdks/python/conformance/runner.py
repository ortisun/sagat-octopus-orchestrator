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
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

import httpx

from sagat_octopus_orchestrator import (
    SDK_VERSION,
    ActivityContext,
    ActivityFailure,
    ApprovalPolicy,
    BlobDescriptor,
    BlobOutput,
    Body,
    ClientConfig,
    Flow,
    QueryDefinition,
    RetryPolicy,
    Run,
    SagatClient,
    SagatError,
    StateDefinition,
    SwitchCase,
    UpdateDefinition,
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
    return observed


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
        found = results.get(step["run"], {}).get("runId")
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

    try:
        for key, (definition, _) in hosts.items():
            if definition.get("autostart") is not False:
                Progress.step = f"autostart of worker {key}"
                await start_worker(key)

        for step in scenario["steps"]:
            Progress.step = f"{step['id']} ({step['op']})"
            client = clients[step.get("token") or "default"]
            observation: Obj = {"id": step["id"], "op": step["op"]}
            op = step["op"]
            try:
                result: Obj | None = None
                if op == "start":
                    workflow_id = config["workflows"][step["workflow"]]["workflowId"]
                    deadline_in = step.get("deadlineInMs")
                    result = run_json(
                        await client.start(
                            workflow_id,
                            step.get("input") or {},
                            idempotency_key=step.get("idempotencyKey"),
                            correlation_id=step.get("correlationId"),
                            traceparent=step.get("traceparent"),
                            deadline_in=deadline_in / 1000 if deadline_in is not None else None,
                        )
                    )
                elif op == "startWithBlob":
                    workflow_id = config["workflows"][step["workflow"]]["workflowId"]
                    blob_body = b"i" * step["inputBlobBytes"]
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
                    page = await client.list_chain_runs(
                        chain_ref(step["namespaceId"]),
                        chain_ref(step["chainId"]),
                        cursor=cursor,
                        limit=step.get("limit"),
                        traceparent=step.get("traceparent"),
                    )
                    result = {
                        "items": [
                            {
                                "index": item.index,
                                "runId": item.run_id,
                                "status": item.status,
                                "completionKind": item.completion_kind,
                                "predecessorRunId": item.predecessor_run_id,
                                "successorRunId": item.successor_run_id,
                                "createdAt": instant(item.created_at),
                                "finishedAt": instant(item.finished_at),
                            }
                            for item in page.items
                        ],
                        "nextCursor": page.next_cursor,
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
                        chain_ref(step["namespaceId"]), chain_ref(step["chainId"]),
                        timeout=step.get("timeoutMs", 60_000) / 1000, poll_interval=0.1,
                    )
                    result = {
                        "chainId": chain.chain_id, "currentRunId": chain.current_run_id, "revision": chain.revision,
                        "cancelRequested": chain.cancel_requested, "finalOutcome": chain.final_outcome,
                        "segments": [
                            {"index": segment.index, "runId": segment.run_id,
                             "predecessorRunId": segment.predecessor_run_id, "successorRunId": segment.successor_run_id}
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
