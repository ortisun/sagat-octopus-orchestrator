"""Typed, side-effect-free builders of the workflow IR (``sdks/SPEC.md`` section 7).

A flow is an ordered list of nodes; ``build()`` links node *i* to node *i + 1*, writes the nested flows of
``parallel`` and ``saga`` after the top flow and returns the document the server validates and versions.
The same program in the five SDKs gives byte-identical canonical text.
"""

from __future__ import annotations

import re
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from typing import Any, Literal, TypeVar

from ._json import canonical_json
from .errors import invalid

_NAME = re.compile(r"[a-z][a-z0-9-]{2,63}")
_SECRET = re.compile(r"[a-z][a-z0-9._-]{0,63}")

Node = dict[str, Any]
_F = TypeVar("_F", bound="Flow")


@dataclass(frozen=True)
class RetryOptions:
    """The retry policy of a node, as ``workflow.schema.json`` names it. Delays are in milliseconds."""

    max_attempts: int | None = None
    initial_delay_ms: int | None = None
    max_delay_ms: int | None = None
    strategy: Literal["fixed", "exponential"] | None = None
    jitter: bool | None = None

    def to_ir(self) -> dict[str, Any]:
        pairs = {
            "maxAttempts": self.max_attempts,
            "initialDelayMs": self.initial_delay_ms,
            "maxDelayMs": self.max_delay_ms,
            "strategy": self.strategy,
            "jitter": self.jitter,
        }
        return {key: value for key, value in pairs.items() if value is not None}


@dataclass(frozen=True)
class SwitchCase:
    """One ordered branch of a switch decision."""

    id: str
    when: str
    target: str


@dataclass(frozen=True)
class ApprovalPolicy:
    """Subjects and built-in roles eligible to decide an approval."""

    subjects: Sequence[str] | None = None
    roles: Sequence[Literal["admin", "developer", "operator"]] | None = None
    exclude_initiator: bool | None = None


RetrySpec = RetryOptions | Mapping[str, Any]
"""A ``RetryOptions``, or a mapping already in the IR's own keys (for a field a newer schema adds)."""


def _retry(spec: RetrySpec | None) -> dict[str, Any] | None:
    if spec is None:
        return None
    return spec.to_ir() if isinstance(spec, RetryOptions) else dict(spec)


def _add(node: Node, **fields: Any) -> Node:
    """Sets the fields that are present; an absent option is omitted, never ``null``."""
    for key, value in fields.items():
        if value is not None:
            node[key] = value
    return node


class Flow:
    """A chain of nodes: the body of a workflow, of a parallel branch or of a saga."""

    def __init__(self) -> None:
        self._nodes: list[Node] = []
        self._embedded: list[Flow] = []
        self._ids: set[str] = set()  # the ids of this flow and of the flows joined to it when they were joined

    @property
    def nodes(self) -> tuple[Node, ...]:
        return tuple(self._nodes)

    def _all_ids(self) -> set[str]:
        """The ids of this flow and of every flow joined to it, as they are now (a flow is read when it is built)."""
        found = {node["id"] for node in self._nodes}
        for inner in self._embedded:
            found |= inner._all_ids()
        return found

    def _claim(self, ids: Iterable[str]) -> None:
        """Takes ids for this flow; one that is taken already is ``used twice`` (checked again by ``build()``)."""
        taken = set(self._ids)
        for node_id in ids:
            if node_id in taken:
                raise invalid(f"the node id {node_id!r} is used twice")
            taken.add(node_id)
        self._ids = taken

    def _push(self, node: Node) -> None:
        node_id = node.get("id")
        if not isinstance(node_id, str) or node_id == "":
            raise invalid("a node needs an id")
        self._claim([node_id])
        self._nodes.append(node)

    def _join(self, node: Node, inner: Sequence[Flow]) -> None:
        """Adds a node that owns other flows (a parallel, a saga): the ids of all of them must be free."""
        node_id = node.get("id")
        if not isinstance(node_id, str) or node_id == "":
            raise invalid("a node needs an id")
        ids = [node_id]
        for flow in inner:
            ids.extend(sorted(flow._all_ids()))
        self._claim(ids)
        self._nodes.append(node)
        self._embedded.extend(inner)

    def step(
        self: _F,
        node_id: str,
        activity: str,
        *,
        retry: RetrySpec | None = None,
        timeout_ms: int | None = None,
        compensate_with: str | None = None,
        reconcile_with: str | None = None,
        idempotency_template: str | None = None,
        secrets: Sequence[str] | None = None,
        compensation_retry: RetrySpec | None = None,
        compensation_timeout_ms: int | None = None,
        irreversible: bool | None = None,
        high_risk: bool | None = None,
        on_error: str | None = None,
    ) -> _F:
        """An activity executed by a worker. ``compensate_with`` names the activity that undoes it (idempotent);
        ``reconcile_with`` the one that finds out whether it happened."""
        if not activity:
            raise invalid(f"step {node_id} needs an activity name")
        if irreversible and compensate_with:
            raise invalid(f"step {node_id} cannot be irreversible and have a compensation")
        for secret in secrets or ():
            if not _SECRET.fullmatch(secret):
                raise invalid(f"step {node_id} names a secret that is not a name: {secret!r}")
        node: Node = {"id": node_id, "type": "activity", "activity": activity}
        _add(
            node,
            retry=_retry(retry),
            timeoutMs=timeout_ms,
            compensateWith=compensate_with,
            reconcileWith=reconcile_with,
            idempotencyTemplate=idempotency_template,
            secrets=list(secrets) if secrets is not None else None,
            compensationRetry=_retry(compensation_retry),
            compensationTimeoutMs=compensation_timeout_ms,
            irreversible=irreversible,
            highRisk=high_risk,
            onError=on_error,
        )
        self._push(node)
        return self

    def sleep(self: _F, node_id: str, duration_ms: int) -> _F:
        """Waits for a duration. A timer, not a process: nothing is kept alive."""
        if isinstance(duration_ms, bool) or not isinstance(duration_ms, int) or duration_ms < 1:
            raise invalid(f"sleep {node_id} needs a duration of at least 1 ms")
        self._push({"id": node_id, "type": "timer", "durationMs": duration_ms})
        return self

    def wait_for_event(
        self: _F,
        node_id: str,
        *,
        event_type: str,
        correlation_expr: str,
        timeout_ms: int,
        on_timeout: str | None = None,
        lookback_ms: int | None = None,
    ) -> _F:
        """Waits for an event of a type with a correlation key (an expression over the run's data) or the timeout."""
        if not event_type or not correlation_expr:
            raise invalid(f"waitForEvent {node_id} needs an event_type and a correlation_expr")
        if isinstance(timeout_ms, bool) or not isinstance(timeout_ms, int) or timeout_ms < 1:
            raise invalid(f"waitForEvent {node_id} needs a timeout of at least 1 ms")
        node: Node = {
            "id": node_id,
            "type": "waitEvent",
            "eventType": event_type,
            "correlationExpr": correlation_expr,
            "timeoutMs": timeout_ms,
        }
        self._push(_add(node, onTimeout=on_timeout, lookbackMs=lookback_ms))
        return self

    def invoke_child(
        self: _F,
        node_id: str,
        workflow: str,
        *,
        version: int | None = None,
        input: str | None = None,
        cancel_policy: Literal["propagate", "detach", "request_cancel"] | None = None,
    ) -> _F:
        """Starts another workflow of the namespace and waits for it."""
        if not workflow:
            raise invalid(f"invokeChild {node_id} needs the name of a workflow")
        node: Node = {"id": node_id, "type": "child", "workflowId": workflow}
        self._push(_add(node, version=version, input=input, cancelPolicy=cancel_policy))
        return self

    def condition(self: _F, node_id: str, expression: str, if_true: str, if_false: str) -> _F:
        """A decision over the run's data; it names both of its branches."""
        if not expression:
            raise invalid(f"condition {node_id} needs an expression")
        self._push(
            {"id": node_id, "type": "condition", "expression": expression, "ifTrue": if_true, "ifFalse": if_false}
        )
        return self

    def switch(self: _F, node_id: str, cases: Sequence[SwitchCase], default_target: str) -> _F:
        """Tries cases in order and follows the first matching target, or the default."""
        if not 1 <= len(cases) <= 32:
            raise invalid(f"switch {node_id} needs between 1 and 32 cases")
        if not default_target:
            raise invalid(f"switch {node_id} needs a default target")
        case_ids: set[str] = set()
        copied: list[Node] = []
        for item in cases:
            if not item.id:
                raise invalid(f"switch {node_id} has a case without an id")
            if item.id in case_ids:
                raise invalid(f"switch {node_id} uses case id {item.id!r} twice")
            if not item.when:
                raise invalid(f"switch {node_id} case {item.id!r} needs a when expression")
            if not item.target:
                raise invalid(f"switch {node_id} case {item.id!r} needs a target")
            case_ids.add(item.id)
            copied.append({"id": item.id, "when": item.when, "target": item.target})
        self._push({"id": node_id, "type": "switch", "cases": copied, "default": default_target})
        return self

    def for_each(
        self: _F,
        node_id: str,
        items: str,
        body: str,
        result: str,
        *,
        max_items: int,
        max_concurrency: int,
    ) -> _F:
        """Invokes a named body once for every item captured from ``items``."""
        if not items or not body or not result:
            raise invalid(f"forEach {node_id} needs items, body and result expressions")
        if isinstance(max_items, bool) or not isinstance(max_items, int) or not 1 <= max_items <= 10_000:
            raise invalid(f"forEach {node_id} maxItems must be between 1 and 10000")
        if (
            isinstance(max_concurrency, bool)
            or not isinstance(max_concurrency, int)
            or not 1 <= max_concurrency <= 256
        ):
            raise invalid(f"forEach {node_id} maxConcurrency must be between 1 and 256")
        self._push(
            {
                "id": node_id,
                "type": "forEach",
                "items": items,
                "body": body,
                "result": result,
                "maxItems": max_items,
                "maxConcurrency": max_concurrency,
            }
        )
        return self

    def map(
        self: _F,
        node_id: str,
        items: str,
        body: str,
        result: str,
        *,
        max_items: int,
        max_concurrency: int,
    ) -> _F:
        """Value-returning collection authoring; emits the same durable ``forEach`` node."""
        return self.for_each(
            node_id, items, body, result, max_items=max_items, max_concurrency=max_concurrency
        )

    def while_loop(
        self: _F,
        node_id: str,
        condition: str,
        body: str,
        initial: str,
        carry: str,
        *,
        max_iterations: int,
        timeout_ms: int,
        delay_ms: int | None = None,
        on_error: str | None = None,
    ) -> _F:
        """Runs a named body sequentially while a pre-test condition is true."""
        return self._loop(
            "while", node_id, condition, body, initial, carry,
            max_iterations=max_iterations, timeout_ms=timeout_ms, delay_ms=delay_ms, on_error=on_error,
        )

    def repeat_until(
        self: _F,
        node_id: str,
        condition: str,
        body: str,
        initial: str,
        carry: str,
        *,
        max_iterations: int,
        timeout_ms: int,
        delay_ms: int | None = None,
        on_error: str | None = None,
    ) -> _F:
        """Runs a named body once, then repeats it until a post-test condition is true."""
        return self._loop(
            "repeatUntil", node_id, condition, body, initial, carry,
            max_iterations=max_iterations, timeout_ms=timeout_ms, delay_ms=delay_ms, on_error=on_error,
        )

    def _loop(
        self: _F,
        kind: Literal["while", "repeatUntil"],
        node_id: str,
        condition: str,
        body: str,
        initial: str,
        carry: str,
        *,
        max_iterations: int,
        timeout_ms: int,
        delay_ms: int | None,
        on_error: str | None,
    ) -> _F:
        if not condition or not body or not initial or not carry:
            raise invalid(f"{kind} {node_id} needs condition, body, initial and carry expressions")
        if isinstance(max_iterations, bool) or not isinstance(max_iterations, int) or not 1 <= max_iterations <= 10_000:
            raise invalid(f"{kind} {node_id} maxIterations must be between 1 and 10000")
        if isinstance(timeout_ms, bool) or not isinstance(timeout_ms, int) or not 1 <= timeout_ms <= 31_622_400_000:
            raise invalid(f"{kind} {node_id} timeoutMs must be between 1 and 31622400000")
        if delay_ms is not None and (
            isinstance(delay_ms, bool) or not isinstance(delay_ms, int) or not 1 <= delay_ms <= 31_622_400_000
        ):
            raise invalid(f"{kind} {node_id} delayMs must be between 1 and 31622400000")
        node: Node = {
            "id": node_id,
            "type": kind,
            "condition": condition,
            "body": body,
            "initial": initial,
            "carry": carry,
            "maxIterations": max_iterations,
            "timeoutMs": timeout_ms,
        }
        if delay_ms is not None:
            node["delayMs"] = delay_ms
        if on_error is not None:
            node["onError"] = on_error
        self._push(node)
        return self

    def approval(
        self: _F,
        node_id: str,
        policy: ApprovalPolicy,
        *,
        timeout_ms: int,
        on_approve: str,
        on_reject: str,
        on_expire: str,
    ) -> _F:
        """Waits for one authorized approval decision or for its deadline."""
        subjects = list(policy.subjects or ())
        roles = list(policy.roles or ())
        if not 1 <= len(subjects) + len(roles) <= 32:
            raise invalid(f"approval {node_id} policy needs between 1 and 32 subjects and roles")
        if len(set(subjects)) != len(subjects):
            raise invalid(f"approval {node_id} policy has a duplicate subject")
        if len(set(roles)) != len(roles):
            raise invalid(f"approval {node_id} policy has a duplicate role")
        for subject in subjects:
            if (
                not subject.strip()
                or len(subject.encode("utf-8")) > 255
                or any(ord(char) < 32 or 127 <= ord(char) <= 159 for char in subject)
                or subject[:7].lower() == "system:"
            ):
                raise invalid(f"approval {node_id} policy has an invalid subject")
        if any(role not in ("admin", "developer", "operator") for role in roles):
            raise invalid(f"approval {node_id} policy has an unsupported role")
        if isinstance(timeout_ms, bool) or not isinstance(timeout_ms, int) or not 1 <= timeout_ms <= 31_622_400_000:
            raise invalid(f"approval {node_id} timeoutMs must be between 1 and 31622400000")
        if not on_approve:
            raise invalid(f"approval {node_id} needs onApprove")
        if not on_reject:
            raise invalid(f"approval {node_id} needs onReject")
        if not on_expire:
            raise invalid(f"approval {node_id} needs onExpire")
        policy_node: Node = {}
        if policy.subjects is not None:
            policy_node["subjects"] = subjects
        if policy.roles is not None:
            policy_node["roles"] = roles
        if policy.exclude_initiator is not None:
            policy_node["excludeInitiator"] = policy.exclude_initiator
        self._push(
            {
                "id": node_id,
                "type": "approval",
                "policy": policy_node,
                "timeoutMs": timeout_ms,
                "onApprove": on_approve,
                "onReject": on_reject,
                "onExpire": on_expire,
            }
        )
        return self

    def parallel(self: _F, node_id: str, join: Literal["all", "any"], branches: Sequence[Flow]) -> _F:
        """Runs branches at once; ``all`` waits for every one, ``any`` for the first."""
        if len(branches) < 2:
            raise invalid(f"parallel {node_id} needs at least two branches")
        if any(not branch._nodes for branch in branches):
            raise invalid(f"parallel {node_id} has an empty branch")
        self._join(
            {
                "id": node_id,
                "type": "parallel",
                "branches": [branch._nodes[0]["id"] for branch in branches],
                "join": join,
            },
            branches,
        )
        return self

    def saga(
        self: _F,
        node_id: str,
        body: Flow,
        *,
        on_compensation_failure: Literal["manual_review", "fail"] | None = None,
        cancel_policy: Literal["compensate_completed", "abort_no_compensation", "wait_in_flight_then_compensate"]
        | None = None,
    ) -> _F:
        """The body is compensated, in reverse order of completion, when it fails or the run is cancelled;
        a step in it names its undo with ``compensate_with``."""
        if not body._nodes:
            raise invalid(f"saga {node_id} has an empty body")
        if any(node["type"] == "end" for node in body._nodes):
            raise invalid(f"saga {node_id} cannot hold an end node")
        node: Node = {"id": node_id, "type": "saga", "begin": body._nodes[0]["id"]}
        self._join(_add(node, onCompensationFailure=on_compensation_failure, cancelPolicy=cancel_policy), [body])
        return self

    def end(self: _F, node_id: str, *, output_expr: str | None = None) -> _F:
        """An explicit end of the flow, optionally with the run's output expression."""
        self._push(_add({"id": node_id, "type": "end"}, outputExpr=output_expr))
        return self

    def _flatten(self) -> list[Node]:
        """Every node of the flow and of the flows inside it, in order, with ``next`` set."""
        out: list[Node] = []
        for index, node in enumerate(self._nodes):
            linked = dict(node)
            # Branching nodes name their routes and an end ends: neither has a next node.
            branching = ("condition", "switch", "approval", "end", "return")
            if index + 1 < len(self._nodes) and linked["type"] not in branching:
                linked["next"] = self._nodes[index + 1]["id"]
            out.append(linked)
        for inner in self._embedded:
            out.extend(inner._flatten())
        return out


class Body(Flow):
    """A closed, named collection or loop body with one explicit return boundary."""

    def return_value(self, node_id: str, result: str) -> Body:
        if not result:
            raise invalid(f"return {node_id} needs a result expression")
        self._push({"id": node_id, "type": "return", "result": result})
        return self


class WorkflowBuilder(Flow):
    """Builds the workflow IR (``workflow.schema.json``) without any side effect."""

    def __init__(self, name: str) -> None:
        super().__init__()
        if not _NAME.fullmatch(name):
            raise invalid(f"a workflow name is lower case letters, digits and dashes, 3 to 64: {name!r}")
        self._name = name
        self._description: str | None = None
        self._labels: dict[str, str] = {}
        self._triggers: list[dict[str, Any]] = []
        self._input_schema: dict[str, Any] | None = None
        self._output_schema: dict[str, Any] | None = None
        self._bodies: dict[str, Body] = {}

    def description(self, text: str) -> WorkflowBuilder:
        self._description = text
        return self

    def label(self, key: str, value: str) -> WorkflowBuilder:
        self._labels[key] = value
        return self

    def trigger(
        self,
        kind: Literal["manual", "event", "webhook", "message", "cron"],
        *,
        event_type: str | None = None,
        source: str | None = None,
        endpoint: str | None = None,
        connector: str | None = None,
        filter: str | None = None,
        cron: str | None = None,
        timezone: str | None = None,
    ) -> WorkflowBuilder:
        """A trigger that starts a run: a person (``manual``), an event, a webhook, a message or a schedule."""
        entry: dict[str, Any] = {"kind": kind}
        _add(
            entry,
            eventType=event_type,
            source=source,
            endpoint=endpoint,
            connector=connector,
            filter=filter,
            cron=cron,
            timezone=timezone,
        )
        self._triggers.append(entry)
        return self

    def input_schema(self, schema: Mapping[str, Any]) -> WorkflowBuilder:
        self._input_schema = dict(schema)
        return self

    def output_schema(self, schema: Mapping[str, Any]) -> WorkflowBuilder:
        self._output_schema = dict(schema)
        return self

    def body(self, name: str, definition: Body) -> WorkflowBuilder:
        """Registers a closed body by name. The body is read when ``build()`` runs."""
        if not name:
            raise invalid("a body needs a name")
        if name in self._bodies:
            raise invalid(f"the body {name!r} is defined twice")
        self._bodies[name] = definition
        return self

    def build(self) -> dict[str, Any]:
        """The document. Raises ``VALIDATION_FAILED`` for what the server would refuse and the builder can see."""
        if not self._nodes:
            raise invalid("a workflow needs at least one node")
        nodes = self._flatten()
        bodies: dict[str, list[Node]] = {}
        for name, definition in self._bodies.items():
            nested = definition._flatten()
            if not nested:
                raise invalid(f"body {name!r} is empty")
            if any(node["type"] == "end" for node in nested):
                raise invalid(f"body {name!r} cannot hold an end node")
            if sum(node["type"] == "return" for node in nested) != 1:
                raise invalid(f"body {name!r} needs exactly one return node")
            bodies[name] = nested
        def validate_scope(scope: list[Node]) -> None:
            local_ids: set[str] = set()
            for node in scope:
                if node["id"] in local_ids:
                    raise invalid(f"the node id {node['id']!r} is used twice")
                local_ids.add(node["id"])

            def reference(source: str, field: str, target: Any) -> None:
                if isinstance(target, str) and target not in local_ids:
                    raise invalid(f"{source}.{field} points at {target!r}, which is not a node")

            for node in scope:
                for field in (
                    "next", "onError", "onTimeout", "ifTrue", "ifFalse", "begin",
                    "onApprove", "onReject", "onExpire",
                ):
                    reference(node["id"], field, node.get(field))
                for branch in node.get("branches", ()):
                    reference(node["id"], "branches", branch)
                for item in node.get("cases", ()):
                    reference(node["id"], "cases", item.get("target"))
                reference(node["id"], "default", node.get("default"))
                if node["type"] in ("forEach", "while", "repeatUntil"):
                    kind = node["type"]
                    if node["body"] not in self._bodies:
                        raise invalid(f"{kind} {node['id']} points at {node['body']!r}, which is not a body")
                    if "next" not in node:
                        raise invalid(f"{kind} {node['id']} needs a next node")

        validate_scope(nodes)
        for nested in bodies.values():
            validate_scope(nested)
        metadata: dict[str, Any] = {"name": self._name}
        if self._description is not None:
            metadata["description"] = self._description
        if self._labels:
            metadata["labels"] = dict(self._labels)
        document: dict[str, Any] = {
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": metadata,
            "entrypoint": self._nodes[0]["id"],
            "nodes": nodes,
        }
        if self._triggers:
            document["triggers"] = [dict(trigger) for trigger in self._triggers]
        if self._input_schema is not None:
            document["inputSchema"] = self._input_schema
        if self._output_schema is not None:
            document["outputSchema"] = self._output_schema
        if bodies:
            document["bodies"] = {
                name: {"entrypoint": nested[0]["id"], "nodes": nested} for name, nested in bodies.items()
            }
        return document

    def to_json(self) -> str:
        """The canonical text of ``build()``: keys sorted, no whitespace."""
        return canonical_json(self.build())


def workflow(name: str) -> WorkflowBuilder:
    """Starts a workflow definition."""
    return WorkflowBuilder(name)


def flow() -> Flow:
    """A flow for a parallel branch or a saga body."""
    return Flow()


def body() -> Body:
    """A closed body for a runtime-sized collection or loop."""
    return Body()
