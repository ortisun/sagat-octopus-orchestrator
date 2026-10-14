package dev.sagatoctopusorchestrator.sdk.opentelemetry;

import dev.sagatoctopusorchestrator.sdk.Hooks;
import dev.sagatoctopusorchestrator.sdk.SagatSdk;
import dev.sagatoctopusorchestrator.sdk.worker.WorkerEvent;
import io.opentelemetry.api.GlobalOpenTelemetry;
import io.opentelemetry.api.OpenTelemetry;
import io.opentelemetry.api.common.Attributes;
import io.opentelemetry.api.metrics.DoubleHistogram;
import io.opentelemetry.api.metrics.LongCounter;
import io.opentelemetry.api.trace.Span;
import io.opentelemetry.api.trace.SpanKind;
import io.opentelemetry.api.trace.StatusCode;
import io.opentelemetry.context.Context;
import io.opentelemetry.context.propagation.TextMapGetter;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Consumer;
import java.util.function.Supplier;
import java.util.regex.Pattern;

/** Opt-in adapter for the SDK hooks. The application owns the OpenTelemetry SDK and exporters. */
public final class OpenTelemetryBridge {
    private static final String SCOPE = "sagat-octopus-orchestrator-sdk";
    private static final Pattern CODE = Pattern.compile("[A-Z0-9_]{1,64}");
    private static final TextMapGetter<Map<String, String>> GETTER = new TextMapGetter<>() {
        @Override public Iterable<String> keys(Map<String, String> carrier) { return carrier.keySet(); }
        @Override public String get(Map<String, String> carrier, String key) { return carrier.get(key); }
    };

    private final OpenTelemetry openTelemetry;
    private final io.opentelemetry.api.trace.Tracer tracer;
    private final LongCounter events;
    private final DoubleHistogram duration;
    private final ConcurrentHashMap<String, Span> open = new ConcurrentHashMap<>();

    /** Uses the global API registered by the application or Java agent. */
    public OpenTelemetryBridge() { this(GlobalOpenTelemetry.get()); }

    /** Uses an application-supplied API instance; it is never modified. */
    public OpenTelemetryBridge(OpenTelemetry openTelemetry) {
        this.openTelemetry = openTelemetry;
        tracer = openTelemetry.getTracer(SCOPE, SagatSdk.VERSION);
        var meter = openTelemetry.getMeter(SCOPE);
        events = meter.counterBuilder("sagat.worker.events").setUnit("{event}").setDescription("Worker lifecycle events.").build();
        duration = meter.histogramBuilder("sagat.worker.task.duration").setUnit("ms").setDescription("Worker task attempt duration.").build();
    }

    /** Supplier for {@code ClientConfig.Builder.traceContext}. */
    public Supplier<String> traceContext() { return this::currentTraceparent; }

    /** Consumer for {@code WorkerConfig.Builder.onEvent}. */
    public Consumer<WorkerEvent> onEvent() { return this::accept; }

    private String currentTraceparent() {
        try {
            var context = Span.current().getSpanContext();
            if (!context.isValid()) return null;
            return "00-" + context.getTraceId() + "-" + context.getSpanId() + "-" + context.getTraceFlags().asHex();
        } catch (Throwable t) {
            Hooks.rethrowIfFatal(t);
            return null;
        }
    }

    private void accept(WorkerEvent event) {
        try {
            var attributes = Attributes.builder().put("sagat.event.type", eventName(event));
            if (event instanceof WorkerEvent.TaskFailed failed && CODE.matcher(failed.code()).matches()) {
                attributes.put("error.type", failed.code());
            }
            events.add(1, attributes.build());
            if (event instanceof WorkerEvent.TaskTaken taken) {
                var old = open.remove(taken.taskId());
                if (old != null) old.end();
                if (open.size() >= 1000) return;
                Context parent = taken.traceparent() == null || taken.traceparent().isEmpty()
                        ? Context.current()
                        : openTelemetry.getPropagators().getTextMapPropagator().extract(
                                Context.current(), Map.of("traceparent", taken.traceparent()), GETTER);
                Span span = tracer.spanBuilder("sagat.worker.task").setSpanKind(SpanKind.CONSUMER).setParent(parent)
                        .setAttribute("sagat.task.attempt", taken.attempt()).startSpan();
                open.put(taken.taskId(), span);
                return;
            }
            String taskId = taskId(event);
            if (taskId == null) return;
            Span span = open.remove(taskId);
            if (span == null) return;
            String outcome = eventName(event).substring("task.".length());
            span.setAttribute("sagat.outcome", outcome);
            if (event instanceof WorkerEvent.TaskFailed failed) {
                span.setStatus(StatusCode.ERROR);
                if (CODE.matcher(failed.code()).matches()) span.setAttribute("error.type", failed.code());
                duration.record(Math.max(0, failed.duration().toNanos() / 1_000_000d), Attributes.of(io.opentelemetry.api.common.AttributeKey.stringKey("sagat.outcome"), outcome));
            } else if (event instanceof WorkerEvent.TaskCompleted completed) {
                duration.record(Math.max(0, completed.duration().toNanos() / 1_000_000d), Attributes.of(io.opentelemetry.api.common.AttributeKey.stringKey("sagat.outcome"), outcome));
            }
            span.end();
        } catch (Throwable t) {
            Hooks.rethrowIfFatal(t);
        }
    }

    private static String taskId(WorkerEvent event) {
        if (event instanceof WorkerEvent.TaskCompleted e) return e.taskId();
        if (event instanceof WorkerEvent.TaskFailed e) return e.taskId();
        if (event instanceof WorkerEvent.TaskStale e) return e.taskId();
        if (event instanceof WorkerEvent.TaskDropped e) return e.taskId();
        return null;
    }

    private static String eventName(WorkerEvent event) {
        if (event instanceof WorkerEvent.Registered) return "registered";
        if (event instanceof WorkerEvent.Poll) return "poll";
        if (event instanceof WorkerEvent.TaskTaken) return "task.taken";
        if (event instanceof WorkerEvent.TaskCompleted) return "task.completed";
        if (event instanceof WorkerEvent.TaskFailed) return "task.failed";
        if (event instanceof WorkerEvent.TaskStale) return "task.stale";
        if (event instanceof WorkerEvent.TaskDropped) return "task.dropped";
        if (event instanceof WorkerEvent.Heartbeat) return "heartbeat";
        return "reconnect";
    }
}
