# Refusal transaction fault review

Fresh C1 reviewed exactly two fault tests plus their helpers. Required correction accepted: both draft workflow prefixes exceeded the 63-character slug limit when combined with UUID. Root shortened them to refusal-midpersist- and refusal-drain- before execution. No other required issue was found under that correction.

The first trigger gates the exact first scoped step.completed journal insertion, after the awaited root-only persist. The second gates actual collection.drain.started outbox insertion after the report has already committed. Both use observed pg_blocking_pids, a second-connection state snapshot while still blocked, a distinct labelled23514 failure, full rollback comparison and normal valid retry. The executed patch includes the corrected names and formatting. No old binary, process restart or provider effect evidence is inferred.
