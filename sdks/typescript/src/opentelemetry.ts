import {
  context,
  metrics,
  propagation,
  trace,
  SpanKind,
  SpanStatusCode,
  defaultTextMapGetter,
  type MeterProvider,
  type TextMapPropagator,
  type TracerProvider,
  type Span,
} from '@opentelemetry/api';
import { SDK_VERSION } from './config.js';
import type { WorkerEvent } from './worker/host.js';

const SCOPE = '@sagat-octopus-orchestrator/sdk';
const MAX_OPEN_TASKS = 1000;
const CODE = /^[A-Z0-9_]{1,64}$/;

/** Options for the opt-in OpenTelemetry bridge. The application still owns providers and exporters. */
export interface OpenTelemetryOptions {
  tracerProvider?: TracerProvider;
  meterProvider?: MeterProvider;
  propagator?: TextMapPropagator;
}

/**
 * Adapts the SDK's ambient trace and worker event hooks to the OpenTelemetry API. Creating this
 * object does not install a provider, context manager or exporter.
 */
export class OpenTelemetryBridge {
  readonly #tracer;
  readonly #events;
  readonly #duration;
  readonly #propagator: TextMapPropagator;
  readonly #open = new Map<string, Span>();

  constructor(options: OpenTelemetryOptions = {}) {
    this.#tracer = (options.tracerProvider ?? trace).getTracer(SCOPE, SDK_VERSION);
    const meter = (options.meterProvider ?? metrics).getMeter(SCOPE, SDK_VERSION);
    this.#events = meter.createCounter('sagat.worker.events', { unit: '{event}', description: 'Worker lifecycle events.' });
    this.#duration = meter.createHistogram('sagat.worker.task.duration', { unit: 'ms', description: 'Worker task attempt duration.' });
    this.#propagator = options.propagator ?? propagation;
  }

  /** Pass this function as `ClientConfig.traceContext`. */
  readonly traceContext = (): string | undefined => {
    try {
      const value = trace.getSpanContext(context.active());
      return value && trace.isSpanContextValid(value)
        ? `00-${value.traceId}-${value.spanId}-${value.traceFlags.toString(16).padStart(2, '0')}`
        : undefined;
    } catch {
      return undefined;
    }
  };

  /** Pass this function as `WorkerConfig.onEvent`. It is safe to call concurrently. */
  readonly onEvent = (event: WorkerEvent): void => {
    try {
      const attributes: Record<string, string> = { 'sagat.event.type': event.type };
      if (event.type === 'task.failed' && CODE.test(event.code)) attributes['error.type'] = event.code;
      this.#events.add(1, attributes);

      if (event.type === 'task.taken') {
        const old = this.#open.get(event.taskId);
        old?.end();
        if (this.#open.size >= MAX_OPEN_TASKS) return;
        const parent = event.traceparent
          ? this.#propagator.extract(context.active(), { traceparent: event.traceparent }, defaultTextMapGetter)
          : context.active();
        this.#open.set(event.taskId, this.#tracer.startSpan('sagat.worker.task', {
          kind: SpanKind.CONSUMER,
          attributes: { 'sagat.task.attempt': event.attempt },
        }, parent));
        return;
      }

      if (!event.type.startsWith('task.') || !('taskId' in event)) return;
      const span = this.#open.get(event.taskId);
      if (!span) return;
      this.#open.delete(event.taskId);
      const outcome = event.type.slice('task.'.length);
      span.setAttribute('sagat.outcome', outcome);
      if (event.type === 'task.failed') {
        span.setStatus({ code: SpanStatusCode.ERROR });
        if (CODE.test(event.code)) span.setAttribute('error.type', event.code);
      }
      if ((event.type === 'task.completed' || event.type === 'task.failed') && Number.isFinite(event.durationMs)) {
        this.#duration.record(Math.max(0, event.durationMs), { 'sagat.outcome': outcome });
      }
      span.end();
    } catch {
      // Telemetry is observational: a broken provider/exporter never changes worker handling.
    }
  };
}
