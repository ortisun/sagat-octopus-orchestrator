package otelbridge_test

import (
	"context"
	"fmt"
	"strings"
	"testing"
	"time"

	"go.opentelemetry.io/otel/sdk/metric"
	metricdata "go.opentelemetry.io/otel/sdk/metric/metricdata"
	"go.opentelemetry.io/otel/sdk/trace"
	"go.opentelemetry.io/otel/sdk/trace/tracetest"

	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/otelbridge"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

func TestRealSDKExportsParentedSafeSpanAndMetrics(t *testing.T) {
	exporter := tracetest.NewInMemoryExporter()
	traces := trace.NewTracerProvider(trace.WithSyncer(exporter))
	reader := metric.NewManualReader()
	meters := metric.NewMeterProvider(metric.WithReader(reader))
	bridge := otelbridge.New(traces, meters)
	ctx, parent := traces.Tracer("test").Start(context.Background(), "parent")
	tp := bridge.TraceContext(ctx)
	bridge.OnEvent(worker.Event{Type: worker.EventTaskTaken, TaskID: "secret-task", Activity: "secret-activity", Attempt: 2, Traceparent: tp})
	bridge.OnEvent(worker.Event{Type: worker.EventFailed, TaskID: "secret-task", Code: "PAYMENT_DECLINED", Duration: 4 * time.Millisecond})
	parent.End()
	var task tracetest.SpanStub
	for _, span := range exporter.GetSpans() {
		if span.Name == "sagat.worker.task" {
			task = span
		}
	}
	if task.Name == "" || task.Parent.SpanID() != parent.SpanContext().SpanID() {
		t.Fatalf("span parent: %+v", task)
	}
	if strings.Contains(fmt.Sprint(task.Attributes), "secret-") {
		t.Fatalf("secret identifier in attributes: %v", task.Attributes)
	}
	var data metricdata.ResourceMetrics
	if err := reader.Collect(context.Background(), &data); err != nil {
		t.Fatal(err)
	}
	names := map[string]bool{}
	for _, scope := range data.ScopeMetrics {
		for _, m := range scope.Metrics {
			names[m.Name] = true
		}
	}
	if !names["sagat.worker.events"] || !names["sagat.worker.task.duration"] {
		t.Fatalf("metrics: %v", names)
	}
}

type brokenExporter struct{}

func (brokenExporter) ExportSpans(context.Context, []trace.ReadOnlySpan) error {
	panic("secret exporter text")
}
func (brokenExporter) Shutdown(context.Context) error { return nil }

func TestExporterFailureCannotEscapeEventHandling(t *testing.T) {
	traces := trace.NewTracerProvider(trace.WithSyncer(brokenExporter{}))
	bridge := otelbridge.New(traces, metric.NewMeterProvider())
	bridge.OnEvent(worker.Event{Type: worker.EventTaskTaken, TaskID: "t", Attempt: 1})
	bridge.OnEvent(worker.Event{Type: worker.EventCompleted, TaskID: "t", Duration: time.Millisecond})
}
