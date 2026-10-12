package dev.sagatoctopusorchestrator.sdk.opentelemetry;

import static org.junit.jupiter.api.Assertions.*;

import dev.sagatoctopusorchestrator.sdk.worker.WorkerEvent;
import io.opentelemetry.api.trace.Span;
import io.opentelemetry.context.propagation.ContextPropagators;
import io.opentelemetry.api.trace.propagation.W3CTraceContextPropagator;
import io.opentelemetry.sdk.OpenTelemetrySdk;
import io.opentelemetry.sdk.metrics.SdkMeterProvider;
import io.opentelemetry.sdk.testing.exporter.InMemoryMetricReader;
import io.opentelemetry.sdk.testing.exporter.InMemorySpanExporter;
import io.opentelemetry.sdk.trace.SdkTracerProvider;
import io.opentelemetry.sdk.trace.ReadWriteSpan;
import io.opentelemetry.sdk.trace.ReadableSpan;
import io.opentelemetry.sdk.trace.SpanProcessor;
import io.opentelemetry.sdk.trace.export.SimpleSpanProcessor;
import io.opentelemetry.context.Context;
import io.opentelemetry.sdk.common.CompletableResultCode;
import java.time.Duration;
import org.junit.jupiter.api.Test;

class OpenTelemetryBridgeTest {
    @Test
    void realSdkExportsParentedSafeSpanAndMetrics() {
        var spans = InMemorySpanExporter.create();
        var traces = SdkTracerProvider.builder().addSpanProcessor(SimpleSpanProcessor.create(spans)).build();
        var reader = InMemoryMetricReader.create();
        var meters = SdkMeterProvider.builder().registerMetricReader(reader).build();
        var telemetry = OpenTelemetrySdk.builder().setTracerProvider(traces).setMeterProvider(meters)
                .setPropagators(ContextPropagators.create(W3CTraceContextPropagator.getInstance())).build();
        var bridge = new OpenTelemetryBridge(telemetry);
        Span parent = traces.get("test").spanBuilder("parent").startSpan();
        String traceparent;
        try (var ignored = parent.makeCurrent()) {
            traceparent = bridge.traceContext().get();
            bridge.onEvent().accept(new WorkerEvent.TaskTaken("secret-task", "secret-activity", 2, traceparent));
            bridge.onEvent().accept(new WorkerEvent.TaskFailed("secret-task", "PAYMENT_DECLINED", false, Duration.ofMillis(4)));
        }
        parent.end();
        var task = spans.getFinishedSpanItems().stream().filter(s -> s.getName().equals("sagat.worker.task")).findFirst().orElseThrow();
        assertEquals(parent.getSpanContext().getSpanId(), task.getParentSpanId());
        assertEquals("failed", task.getAttributes().get(io.opentelemetry.api.common.AttributeKey.stringKey("sagat.outcome")));
        assertFalse(task.getAttributes().toString().contains("secret-"));
        var names = reader.collectAllMetrics().stream().map(m -> m.getName()).toList();
        assertTrue(names.contains("sagat.worker.events"));
        assertTrue(names.contains("sagat.worker.task.duration"));
        traces.close();
        meters.close();
    }

    @Test
    void throwingProcessorCannotEscapeEventHandling() {
        SpanProcessor broken = new SpanProcessor() {
            @Override public void onStart(Context parent, ReadWriteSpan span) {}
            @Override public boolean isStartRequired() { return true; }
            @Override public void onEnd(ReadableSpan span) { throw new IllegalStateException("secret exporter text"); }
            @Override public boolean isEndRequired() { return true; }
            @Override public CompletableResultCode shutdown() { return CompletableResultCode.ofSuccess(); }
            @Override public CompletableResultCode forceFlush() { return CompletableResultCode.ofSuccess(); }
        };
        var traces = SdkTracerProvider.builder().addSpanProcessor(broken).build();
        var telemetry = OpenTelemetrySdk.builder().setTracerProvider(traces).build();
        var bridge = new OpenTelemetryBridge(telemetry);
        bridge.onEvent().accept(new WorkerEvent.TaskTaken("t", "a", 1, ""));
        assertDoesNotThrow(() -> bridge.onEvent().accept(new WorkerEvent.TaskCompleted("t", Duration.ofMillis(1))));
        traces.close();
    }
}
