# Review: feature 022, release 0.3.0 slice — security

- **Doc:** `specs/022-flow-control-step-policies/` (spec, plan, tasks, data-model, contracts, research) with ADR-0056 and ADR-0088
- **Reviewed-commit:** `1a21cbdcf9afc80a21b29c90f768df256dd66c64`
- **Reviewer:** `security-reviewer` (independent read-only agent, run by the coordinator on 2026-11-13)
- **Recorded by:** the slice's author, from the coordinator's relay of the findings; the reviewer's full text was not handed to the author.
- **Answered in:** the revision commit that adds this file.

| Finding | Severity | How the revision answers it |
|---|---|---|
| M1 — a flood on one key or tenant starves others; the pass is per waiter and slow | Major | key-driven pass with per-tenant fair selection and bounded visits (FR-022-047, FR-022-052, D022-021); SC-022-017 revised; SC-022-029; T022-097, T022-089 |
| M2 — one key can fill `queued_runs` | Major | per-key run waiters default 100, always below `queued_runs` (FR-022-042, D022-024) |
| M3 — key cardinality and retention of grants | Major | identity-only keys refused (`FLOW_CONTROL_KEY_UNBOUNDED`), purge by `max_period_ms` + 1 min (FR-022-056, D022-025); T022-103 |
| M4 — digest key rotation blocked by live rows | Major | per-key rotation with both digests (FR-022-056); `flow_control_key_versions`; T022-095 |
| M5 — a failing step key must fail closed | Major | FR-022-054; `FLOW_CONTROL_KEY_INVALID`, `FLOW_CONTROL_LIMIT`; T022-101 |
| M6 — classification of keys and label disclosure | Major | FR-022-057; `FLOW_CONTROL_KEY_SECRET` / `_PII`; `keyLabel` nullable; G1185 (online oracle); T022-104 |
| Nit — position and estimate in listings | Nit | single-run read only (FR-022-048, OpenAPI fragment) |
| Nit — `group` filter pattern rejects derived groups | Nit | pattern widened in both list routes |
| Nit — foreign keys without the namespace | Nit | `(tenant_id, namespace_id, run_id)` → `runs_tenant_namespace_id_uniq` (data-model revision) |
| Nit — runtime-role grants and `SECURITY DEFINER` for counters | Nit | definer triggers, `SELECT`-only counters, column-limited `UPDATE` (FR-022-058, T022-105) |
| Nit — does a grant re-authorize the initiator | Nit | no, stated and tested (FR-022-060, T022-106) |
| Nit — paused steps hold slots | Nit | FR-022-046, T022-100 |
| Three further nits — not relayed to the author | Nit | not addressed; G1186 |

Counts: B0 M6 N9
