# Telemetry and audit (design 046)

Every label is closed; tenants, namespaces, scripts, versions and packages are span attributes, never labels.

## Control plane (builder A, Task 046-25)

| Metric | Labels and values |
|---|---|
| `script_dependency_jobs_total` | `phase` (`resolve`, `install`), `ecosystem` (`npm`, `pypi`), `outcome` (`succeeded`, `refused`, `failed`, `retried`) |
| `script_dependency_job_seconds` | `phase`, `ecosystem` |
| `script_supply_chain_evaluations_total` | `outcome` (`admitted`, `refused`), `path` (`resolve`, `author_lock`, `finish`, `reevaluation`) |
| `script_supply_chain_findings_total` | `kind` (`vulnerability`, `malware`, `licence`, `age`, `install_script`, `yanked`), `severity` (`critical`, `high`, `moderate`, `low`, `unrated`, `none`) |
| `script_supply_chain_lease_refusals_total` | `point` (`lease`, `materialization`, `start`) |
| `script_policy_reevaluation_backlog` | none (rows waiting) |
| `package_advisory_import_seconds`, `package_advisory_snapshot_age_seconds` | `ecosystem` |
| `package_advisory_imports_total` | `ecosystem`, `outcome` (`switched`, `refused_older`, `refused_shrinking`, `held`, `confirmed`) |

Audit records: `script.dependencies_resolved`, `script.supply_chain_refused`, `script.supply_chain_blocked`,
`script.supply_chain_unblocked`, `script_policy.changed`, `script_package_rule.added`, `script_package_rule.removed`,
`supply_chain_waiver.added`, `supply_chain_waiver.expired`, `supply_chain_waiver.removed`, `package.reserved`,
`package.unreserved`, `package.uploaded`, `package.deleted`, `advisory_snapshot.imported`, `advisory_snapshot.held`,
`advisory_snapshot.forced`, `script_block.released`, and the cross-tenant attempt records of the private routes and
materializations.

## Gateway and runner (builder B, Task 046-21)

| Metric | Labels and values |
|---|---|
| `package_gateway_requests_total` | `ecosystem`, `kind` (`document`, `file`, `metadata`, `advisory`, `internal`), `result` (`ok`, `not_found`, `refused`, `upstream_error`) |
| `package_gateway_upstream_seconds` | `ecosystem` |
| `script_layer_cache_total` | `result` (`hit`, `miss`, `evicted`) |
| `script_layer_materialize_seconds` | `ecosystem`, `outcome` (`ok`, `failed`, `blocked`) |
| `script_resolve_forwarded_bytes` | `ecosystem` |
