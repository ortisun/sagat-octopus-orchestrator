ALTER TABLE inbox_events DROP CONSTRAINT inbox_events_trace_parent_span_format;
ALTER TABLE inbox_events DROP CONSTRAINT inbox_events_trace_id_format;
ALTER TABLE inbox_events DROP COLUMN trace_parent_span;
ALTER TABLE inbox_events DROP COLUMN trace_id;
ALTER TABLE runs DROP CONSTRAINT runs_trace_parent_span_format;
ALTER TABLE runs DROP COLUMN trace_parent_span;
