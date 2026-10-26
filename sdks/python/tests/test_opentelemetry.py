from __future__ import annotations

from opentelemetry import trace
from opentelemetry.sdk.metrics import MeterProvider
from opentelemetry.sdk.metrics.export import InMemoryMetricReader
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor, SpanExporter, SpanExportResult
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter

from sagat_octopus_orchestrator.opentelemetry import OpenTelemetryBridge
from sagat_octopus_orchestrator.worker import WorkerEvent


def test_real_otel_sdk_exports_parented_safe_span_and_metrics() -> None:
    exporter = InMemorySpanExporter()
    traces = TracerProvider()
    traces.add_span_processor(SimpleSpanProcessor(exporter))
    reader = InMemoryMetricReader()
    meters = MeterProvider(metric_readers=[reader])
    bridge = OpenTelemetryBridge(traces, meters)
    with trace.use_span(traces.get_tracer("test").start_span("parent"), end_on_exit=True) as parent:
        traceparent = bridge.trace_context()
        assert traceparent and parent.get_span_context().trace_id == int(traceparent.split("-")[1], 16)
        bridge.on_event(
            WorkerEvent(
                "task.taken", task_id="secret-task", activity="secret-activity", attempt=2, traceparent=traceparent
            )
        )
        bridge.on_event(
            WorkerEvent("task.failed", task_id="secret-task", code="PAYMENT_DECLINED", retryable=False, duration_ms=4)
        )
    task = next(span for span in exporter.get_finished_spans() if span.name == "sagat.worker.task")
    assert task.parent and task.parent.span_id == parent.get_span_context().span_id
    assert task.attributes == {"sagat.task.attempt": 2, "sagat.outcome": "failed", "error.type": "PAYMENT_DECLINED"}
    assert "secret" not in repr(task.attributes)
    data = reader.get_metrics_data()
    assert data is not None
    names = {
        metric.name
        for resource in data.resource_metrics
        for scope in resource.scope_metrics
        for metric in scope.metrics
    }
    assert {"sagat.worker.events", "sagat.worker.task.duration"} <= names


class _ThrowingExporter(SpanExporter):
    def export(self, spans: object) -> SpanExportResult:
        raise RuntimeError("secret exporter failure")


def test_exporter_failure_does_not_escape_event_handling() -> None:
    traces = TracerProvider()
    traces.add_span_processor(SimpleSpanProcessor(_ThrowingExporter()))
    bridge = OpenTelemetryBridge(traces, MeterProvider())
    bridge.on_event(WorkerEvent("task.taken", task_id="t", attempt=1))
    bridge.on_event(WorkerEvent("task.completed", task_id="t", duration_ms=1))
