# Native leaf certification observations review

Fresh-context Astra/xhigh, bounded3cycles, read-only reviewers ran no tests and made no edits.
Artifact: operational native wrapper, closed telemetry vocabulary, actual observation integration
unit and source/collector helpers. Contract: preserved native transaction/error/replay semantics,
returned-attempt-only metrics, acknowledged-first-commit count only, inherited qualified source
span, no error/authority/payload/digest/audit detail; filtered child cannot change caller fields.

1. /root/review_leaf_observations_c1: actionable oracle gaps. Collector dropped OTLP status.message;
   preserve it and assert empty exported descriptions (also no span events/links). Log assertions
   allowed extra successful result detail; enforce allowed event fields.
2. /root/review_leaf_observations_c2: text log splitting discarded message prefixes, allowing
   forbidden identifiers before fixed text. Replaced text parsing with actual unredacted JSON
   event capture: exact native target,7DEBUG events, fields exactly message+closed outcome and
   complete message equality. No prefix/suffix parsing remains.
3. /root/review_leaf_observations_c3: no concrete remaining contract violations in reviewed changes.
   Stop at this bounded limit, no fourth cycle.

A real source output payload canary additionally covers accidental payload export. A failed run
captured synthetic fixture CREATE FUNCTION SQL, whose literal contained the driver marker; move
only fixture DDL before capture, retaining actual failed native COMMIT inside the redaction boundary.
SQLx0.9 logger.rs records SQL text for debug query events. This fixture correction changes no native
redaction behavior. External review previously declined by user; no external CLI invoked.
