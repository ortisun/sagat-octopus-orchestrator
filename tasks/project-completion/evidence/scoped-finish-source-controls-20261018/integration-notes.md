Before review, root removed unused fake workflow-version creation/immutable-guard deletion.
The canonical certification pin intentionally has no version-table FK; a fresh UUID is already
a schema-valid mismatched pin. No definition row or its guard is touched. Source and capacity
snapshots now compare at each corrupted/missing fixture before restoration, so cleanup cannot
mask a consumer mutation. Notes describe the original draft; integrated source governs.
No production implementation change or test result claimed by these controls before execution.

Actual first compilation failed E0521: SQLx0.9 requires a static SQL string; all literal-only denied-read probes now declare statement: &'static str. Failure logs and unchanged pre-fix source hashes retained separately.
