// Package otelbridge adapts Sagat's existing trace and worker event hooks to OpenTelemetry.
// The application owns providers, readers, processors, and exporters.
package otelbridge

import (
	"context"
	"fmt"
	"regexp"
	"sync"

	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/codes"
	"go.opentelemetry.io/otel/metric"
	"go.opentelemetry.io/otel/propagation"
	"go.opentelemetry.io/otel/trace"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

const maxOpenTasks = 1000

var codePattern = regexp.MustCompile(`^[A-Z0-9_]{1,64}$`)

// Bridge supplies client.WithTraceContext(b.TraceContext) and worker.WithEventHandler(b.OnEvent).
type Bridge struct {
	tracer   trace.Tracer
	events   metric.Int64Counter
	duration metric.Float64Histogram
	mu       sync.Mutex
	open     map[string]trace.Span
}

// New constructs an API-only bridge. Nil providers use the OTel globals; no provider is installed.
func New(traces trace.TracerProvider, meters metric.MeterProvider) *Bridge {
	if traces == nil {
		traces = otel.GetTracerProvider()
	}
	if meters == nil {
		meters = otel.GetMeterProvider()
	}
	meter := meters.Meter("sagat-octopus-orchestrator-sdk", metric.WithInstrumentationVersion(sagat.Version))
	events, _ := meter.Int64Counter("sagat.worker.events", metric.WithUnit("{event}"), metric.WithDescription("Worker lifecycle events."))
	duration, _ := meter.Float64Histogram("sagat.worker.task.duration", metric.WithUnit("ms"), metric.WithDescription("Worker task attempt duration."))
	return &Bridge{
		tracer: traces.Tracer("sagat-octopus-orchestrator-sdk", trace.WithInstrumentationVersion(sagat.Version)),
		events: events, duration: duration, open: make(map[string]trace.Span),
	}
}

// TraceContext returns the current W3C traceparent and is safe as client.WithTraceContext.
func (b *Bridge) TraceContext(ctx context.Context) (result string) {
	defer func() {
		if recover() != nil {
			result = ""
		}
	}()
	sc := trace.SpanContextFromContext(ctx)
	if !sc.IsValid() {
		return ""
	}
	return fmt.Sprintf("00-%s-%s-%02x", sc.TraceID(), sc.SpanID(), byte(sc.TraceFlags()))
}

// OnEvent translates a worker event. Telemetry panics are contained by the host and here.
func (b *Bridge) OnEvent(event worker.Event) {
	defer func() { _ = recover() }()
	ctx := context.Background()
	attrs := []attribute.KeyValue{attribute.String("sagat.event.type", string(event.Type))}
	if event.Type == worker.EventFailed && codePattern.MatchString(event.Code) {
		attrs = append(attrs, attribute.String("error.type", event.Code))
	}
	b.events.Add(ctx, 1, metric.WithAttributes(attrs...))
	if event.Type == worker.EventTaskTaken {
		b.mu.Lock()
		defer b.mu.Unlock()
		if old := b.open[event.TaskID]; old != nil {
			old.End()
		}
		if len(b.open) >= maxOpenTasks {
			return
		}
		if event.Traceparent != "" {
			ctx = propagation.TraceContext{}.Extract(ctx, propagation.MapCarrier{"traceparent": event.Traceparent})
		}
		_, span := b.tracer.Start(ctx, "sagat.worker.task", trace.WithSpanKind(trace.SpanKindConsumer),
			trace.WithAttributes(attribute.Int("sagat.task.attempt", event.Attempt)))
		b.open[event.TaskID] = span
		return
	}
	if event.Type != worker.EventCompleted && event.Type != worker.EventFailed && event.Type != worker.EventStale && event.Type != worker.EventDropped {
		return
	}
	b.mu.Lock()
	span := b.open[event.TaskID]
	delete(b.open, event.TaskID)
	b.mu.Unlock()
	if span == nil {
		return
	}
	outcome := string(event.Type)[len("task."):]
	span.SetAttributes(attribute.String("sagat.outcome", outcome))
	if event.Type == worker.EventFailed {
		span.SetStatus(codes.Error, "")
		if codePattern.MatchString(event.Code) {
			span.SetAttributes(attribute.String("error.type", event.Code))
		}
	}
	if event.Type == worker.EventCompleted || event.Type == worker.EventFailed {
		b.duration.Record(ctx, float64(event.Duration)/1e6, metric.WithAttributes(attribute.String("sagat.outcome", outcome)))
	}
	span.End()
}
