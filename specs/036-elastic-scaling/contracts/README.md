# Contracts of feature 036

| File | What it fixes | Where it lands |
|---|---|---|
| [`evaluation.md`](./evaluation.md) | the formulas, the order of limits, thirteen golden vectors, the properties, the partition targets | `crates/domain/src/scaling/` tests; the dry evaluation's contract tests |
| [`signals-catalogue.md`](./signals-catalogue.md) | the built-in signals of slice 1: name, unit, scope, freshness, meaning | the catalogue listed by `GET /scaling/catalogue`; a test refuses a signal missing any of the four |
| [`metrics.md`](./metrics.md) | the new series, their labels, the rule they obey | the metrics code; `tests/contract/observability-operations` label rule; the chart's autoscaler queries |
| [`scaling-config.schema.json`](./scaling-config.schema.json) | the configuration document named by `SAGAT_SCALING_CONFIG_FILE` | the evaluator's validation; the chart's `values.schema.json` for the `scaling` block |
| [`tenant-scaling.openapi-fragment.yaml`](./tenant-scaling.openapi-fragment.yaml) | the two tenant endpoints (lag, consumption) and their schemas | merged into `specs/001-platform-foundation/contracts/openapi.yaml` with `x-implemented: '036'` |
| [`operator-scaling.openapi.yaml`](./operator-scaling.openapi.yaml) | the operator listener: catalogue, signals, sources, policies, overrides, global control, decisions, desired | its own document; served only when `SAGAT_OPERATOR_ADDR` is set |

Checked so far (2026-10-28, mechanically): the JSON Schema and both YAML documents parse; every local `$ref` resolves; the tenant fragment's references into the existing `openapi.yaml` resolve and none of its paths, operation ids or schema names clash with it; operation ids are unique. **Not yet run:** Redocly lint (the repository's CI step), a JSON Schema validator against the sample configuration of the quickstart (none is installed here; the sample was read against the schema by hand), the contract tests that need an implementation.

Versioning: the operator document starts at `0.1.0` and is versioned like the others (additive changes are minor). The tenant fragment follows `openapi.yaml`. The configuration schema has `version: 1`; a change that is not additive is `version: 2` with the evaluator reading both.
