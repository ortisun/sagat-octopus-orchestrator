"""Opt-in OpenTelemetry adapters. The application owns providers and exporters."""

from __future__ import annotations

import re
import threading
from typing import Any

from opentelemetry import metrics, trace
from opentelemetry.propagators.textmap import Getter
from opentelemetry.trace import Span, SpanKind, Status, StatusCode
from opentelemetry.trace.propagation.tracecontext import TraceContextTextMapPropagator

from .config import SDK_VERSION
from .worker import WorkerEvent

_SCOPE = "sagat-octopus-orchestrator-sdk"
_MAX_OPEN_TASKS = 1000
_CODE = re.compile(r"[A-Z0-9_]{1,64}\Z")


class _Getter(Getter[dict[str, str]]):
    def get(self, carrier: dict[str, str], key: str) -> list[str] | None:
        value = carrier.get(key)
        return [value] if value is not None else None

    def keys(self, carrier: dict[str, str]) -> list[str]:
        return list(carrier)


class OpenTelemetryBridge:
    """Supplies ``ClientConfig.trace_context`` and ``WorkerConfig.on_event`` using OTel APIs only."""

    def __init__(self, tracer_provider: Any = None, meter_provider: Any = None) -> None:
        self._tracer = (tracer_provider or trace.get_tracer_provider()).get_tracer(_SCOPE, SDK_VERSION)
        meter = (meter_provider or metrics.get_meter_provider()).get_meter(_SCOPE, SDK_VERSION)
        self._events = meter.create_counter(
            "sagat.worker.events", unit="{event}", description="Worker lifecycle events."
        )
        self._duration = meter.create_histogram(
            "sagat.worker.task.duration", unit="ms", description="Worker task attempt duration."
        )
        self._open: dict[str, Span] = {}
        self._lock = threading.Lock()
        self._propagator = TraceContextTextMapPropagator()

    def trace_context(self) -> str | None:
        """Return the current W3C traceparent. Safe to pass directly to ``ClientConfig``."""
        try:
            context = trace.get_current_span().get_span_context()
            if not context.is_valid:
                return None
            return f"00-{context.trace_id:032x}-{context.span_id:016x}-{int(context.trace_flags):02x}"
        except Exception:
            return None

    def on_event(self, event: WorkerEvent) -> None:
        """Translate one worker event into bounded metrics and one task span."""
        try:
            attributes: dict[str, str] = {"sagat.event.type": event.type}
            if event.type == "task.failed" and event.code and _CODE.fullmatch(event.code):
                attributes["error.type"] = event.code
            self._events.add(1, attributes)

            if event.type == "task.taken" and event.task_id:
                with self._lock:
                    old = self._open.pop(event.task_id, None)
                    if old is not None:
                        old.end()
                    if len(self._open) >= _MAX_OPEN_TASKS:
                        return
                    parent = (
                        self._propagator.extract({"traceparent": event.traceparent}, getter=_Getter())
                        if event.traceparent
                        else None
                    )
                    self._open[event.task_id] = self._tracer.start_span(
                        "sagat.worker.task",
                        context=parent,
                        kind=SpanKind.CONSUMER,
                        attributes={"sagat.task.attempt": event.attempt or 0},
                    )
                return

            if not event.type.startswith("task.") or not event.task_id:
                return
            with self._lock:
                span = self._open.pop(event.task_id, None)
            if span is None:
                return
            outcome = event.type.removeprefix("task.")
            span.set_attribute("sagat.outcome", outcome)
            if event.type == "task.failed":
                span.set_status(Status(StatusCode.ERROR))
                if event.code and _CODE.fullmatch(event.code):
                    span.set_attribute("error.type", event.code)
            if event.type in {"task.completed", "task.failed"} and event.duration_ms is not None:
                self._duration.record(max(0, event.duration_ms), {"sagat.outcome": outcome})
            span.end()
        except Exception:
            # Telemetry is observational: a broken provider/exporter never changes worker handling.
            return
