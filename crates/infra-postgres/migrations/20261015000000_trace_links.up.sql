-- Observability (feature 008; FR-008-001, G79): the links that make one trace of a request, the run
-- it started and the attempts of the run's steps.
--
-- A run already keeps the trace id it belongs to. What it lacked is the span it hangs from: the
-- span of the request that started it (for a run started through the API), and for a run started by
-- an event, the trace of the event that started it (G79). Both are additive and nullable: a run
-- created before this migration simply has no parent span, and one with no trace is not exported.

ALTER TABLE runs ADD COLUMN trace_parent_span text;
ALTER TABLE runs ADD CONSTRAINT runs_trace_parent_span_format
    CHECK (trace_parent_span IS NULL OR trace_parent_span ~ '^[0-9a-f]{16}$');

-- The trace an event was published in, when its publisher sent a `traceparent`, and the span of the
-- request that admitted it. The run an event starts continues that trace.
ALTER TABLE inbox_events ADD COLUMN trace_id text;
ALTER TABLE inbox_events ADD COLUMN trace_parent_span text;
ALTER TABLE inbox_events ADD CONSTRAINT inbox_events_trace_id_format
    CHECK (trace_id IS NULL OR trace_id ~ '^[0-9a-f]{32}$');
ALTER TABLE inbox_events ADD CONSTRAINT inbox_events_trace_parent_span_format
    CHECK (trace_parent_span IS NULL OR trace_parent_span ~ '^[0-9a-f]{16}$');
