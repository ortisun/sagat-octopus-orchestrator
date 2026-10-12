import assert from 'node:assert/strict';
import test from 'node:test';
import { context, trace } from '@opentelemetry/api';
import { AsyncLocalStorageContextManager } from '@opentelemetry/context-async-hooks';
import { W3CTraceContextPropagator } from '@opentelemetry/core';
import { InMemorySpanExporter, SimpleSpanProcessor, BasicTracerProvider } from '@opentelemetry/sdk-trace-base';
import { AggregationTemporality, InMemoryMetricExporter, MeterProvider, PeriodicExportingMetricReader } from '@opentelemetry/sdk-metrics';
import { OpenTelemetryBridge } from '../src/opentelemetry.js';

test('the real OTel SDK exports a fixed safe child span and metrics', async () => {
  const exporter = new InMemorySpanExporter();
  const provider = new BasicTracerProvider({ spanProcessors: [new SimpleSpanProcessor(exporter)] });
  const manager = new AsyncLocalStorageContextManager().enable();
  context.setGlobalContextManager(manager);
  const metricExporter = new InMemoryMetricExporter(AggregationTemporality.CUMULATIVE);
  const reader = new PeriodicExportingMetricReader({ exporter: metricExporter, exportIntervalMillis: 60_000 });
  const meterProvider = new MeterProvider({ readers: [reader] });
  const bridge = new OpenTelemetryBridge({ tracerProvider: provider, meterProvider, propagator: new W3CTraceContextPropagator() });
  const parent = provider.getTracer('test').startSpan('parent');
  const parentContext = trace.setSpan(context.active(), parent);
  context.with(parentContext, () => {
    const value = bridge.traceContext();
    assert.equal(value?.split('-')[1], parent.spanContext().traceId);
    bridge.onEvent({ type: 'task.taken', taskId: 'secret-task-canary', activity: 'secret-activity-canary', attempt: 2, traceparent: value! });
    bridge.onEvent({ type: 'task.failed', taskId: 'secret-task-canary', code: 'PAYMENT_DECLINED', retryable: false, durationMs: 4 });
  });
  parent.end();
  await reader.forceFlush();
  context.disable();
  const span = exporter.getFinishedSpans().find((item) => item.name === 'sagat.worker.task');
  assert.ok(span);
  assert.equal(span.parentSpanContext?.spanId, parent.spanContext().spanId);
  assert.equal(span.attributes['sagat.outcome'], 'failed');
  assert.equal(span.attributes['error.type'], 'PAYMENT_DECLINED');
  assert.doesNotMatch(JSON.stringify(span.attributes), /secret-(task|activity)-canary/);
  const names = metricExporter.getMetrics().flatMap((resource) => resource.scopeMetrics.flatMap((scope) => scope.metrics.map((metric) => metric.descriptor.name)));
  assert.ok(names.includes('sagat.worker.events'));
  assert.ok(names.includes('sagat.worker.task.duration'));
});

test('a throwing telemetry implementation cannot escape the worker event hook', () => {
  const provider = new BasicTracerProvider({ spanProcessors: [{
    onStart() {},
    onEnd() { throw new Error('secret exporter text'); },
    forceFlush: async () => undefined,
    shutdown: async () => undefined,
  }] });
  const bridge = new OpenTelemetryBridge({ tracerProvider: provider });
  bridge.onEvent({ type: 'task.taken', taskId: 't', activity: 'a', attempt: 1, traceparent: '' });
  assert.doesNotThrow(() => bridge.onEvent({ type: 'task.completed', taskId: 't', durationMs: 1 }));
});
