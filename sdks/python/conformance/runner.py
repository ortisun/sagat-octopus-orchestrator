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
    ClientConfig,
    Flow,
    RetryPolicy,
    Run,
    SagatClient,
    SagatError,
    Verdict,
    WorkerConfig,
    WorkerHost,
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
    return {
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


def error_json(error: BaseException) -> Obj:
    if isinstance(error, SagatError):
        return error.to_dict()
    sys.stderr.write("".join(traceback.format_exception(error)))
    return {"code": "UNEXPECTED", "message": type(error).__name__}


# ------------------------------------------------------------------------------------------ workers


def behavior_handler(name: str, kind: str, behavior: Obj, log: list[Obj]) -> Any:
    async def handler(run_input: Any, ctx: ActivityContext) -> Any:
        outcome = "returned"
        try:
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
                    # `reportsSecretError`: the canonical code of the refusal is the output, so the scenario can compare it.
                    if behavior.get("reportsSecretError"):
                        return {**(behavior.get("returns") or {}), "secretError": error.code}
                    raise
                return {**(behavior.get("returns") or {}), "secretLength": len(secrets.get(behavior["readsSecret"]))}
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
            log.append(
                {
                    "activity": name,
                    "kind": kind,
                    "node": ctx.node,
                    "attempt": ctx.attempt,
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


def host_of(definition: Obj, config: Obj, plan: FaultPlan, log: list[Obj]) -> WorkerHost:
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
            heartbeat_interval=heartbeat / 1000 if heartbeat else None,
            interceptors=plan.interceptors(),
            retry=RetryPolicy(initial_delay=0.02, max_delay=0.2),
        )
    )
    for name, activity in definition["activities"].items():
        kind = activity.get("as") or "activity"
        handler = behavior_handler(name, kind, activity.get("behavior") or {}, log)
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
    }
    logs: dict[str, list[Obj]] = {}
    hosts: dict[str, tuple[Obj, list[WorkerHost]]] = {}
    for key, definition in (scenario.get("workers") or {}).items():
        logs[key] = []
        hosts[key] = (definition, [])

    async def start_worker(key: str) -> Obj:
        if key not in hosts:
            raise ValueError(f"no worker {key}")
        definition, started = hosts[key]
        host = host_of(definition, config, plan, logs[key])
        started.append(host)
        await host.start()
        return {"protocolVersion": host.protocol_version}

    observations: list[Obj] = []
    results: dict[str, Obj] = {}

    def run_of(step: Obj) -> str:
        found = results.get(step["run"], {}).get("runId")
        if not isinstance(found, str):
            raise ValueError(f"step {step['run']} has no run")
        return found

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
                    # In the order the server gave (newest first): the SDK keeps it, also across pages.
                    result = {"items": ids}
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
