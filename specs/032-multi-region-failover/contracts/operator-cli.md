# Operator commands — feature 032 (draft, not implemented)

`control-plane <command>`, following `apps/control-plane/src/cli.rs`: one hyphenated subcommand, no
flags, every input an explicit environment variable, `DATABASE_URL` = the **owner** role of the cell's
database in the region where the command acts (like `quota-ceiling`; feature 010 G260). Output: one line
of JSON on stdout `{"outcome": "...", "reason": "...", ...}`; logs on stderr carry no tenant data,
attestation or reason text. Exit codes as `apps/control-plane/src/lib.rs`: **0** done, **1** database
or I/O failure or unknown outcome (re-run `region-status` before repeating), **2** configuration error or
refused precondition (`reason` says which).

Common inputs: `SAGAT_REGION_ACTOR` (1–255, the person; recorded), `SAGAT_REGION_REASON` (8–2,000;
recorded), `SAGAT_REGION_EXPECTED_EPOCH` (integer; compare-and-set) where noted.

| Command | Inputs | Preconditions (refusal reason) | Effect (one transaction) |
|---|---|---|---|
| `region-init` | `SAGAT_CELL_ID`, `SAGAT_REGION_REGIONS` (2–3, comma-separated), `SAGAT_REGION_ACTIVE`, `SAGAT_REGION_MODE` (`sync_quorum`/`async`) | no authority row (`ALREADY_INITIALIZED`); not in recovery (`IN_RECOVERY`) | authority epoch 1 `ACTIVE`; history `INIT`; `ALTER ROLE sagat_app SET default_transaction_read_only = on` (from now on only region-aware processes write; D1593); prints a reminder that every process must already run this version |
| `region-status` | — | — (read-only; works on a standby) | prints role, epoch, regions, state, mode, `pg_is_in_recovery()`, replay LSN/time, protection if a monitor connection is configured |
| `region-drain` | expected epoch | state `ACTIVE` (`NOT_ACTIVE`); not in recovery | state `DRAINING`; history `DRAIN`; then waits up to `SAGAT_REGION_DRAIN_SECS` (60, ≤ 600) for zero live shard and singleton leases and prints what remains |
| `region-undrain` | expected epoch | state `DRAINING` (`NOT_DRAINING`); not in recovery | state `ACTIVE`; history `UNDRAIN` |
| `region-activate` | expected epoch, `SAGAT_REGION_TARGET`, `SAGAT_REGION_KIND` (`planned`/`unplanned`); planned: `SAGAT_REGION_FINAL_LSN`; unplanned: `SAGAT_REGION_ATTESTATION` (8–2,000); the target region's key configuration (same files/variables its processes mount) for the fingerprint checks | not in recovery (`IN_RECOVERY`); epoch matches (`EPOCH_CONFLICT`); target in the cell and not active (`TARGET_NOT_IN_CELL`, `TARGET_IS_ACTIVE`); ≥ 60 s since the last activation (`TOO_SOON`); planned: state `DRAINING` (`NOT_DRAINING`) and `pg_last_wal_replay_lsn()` ≥ final LSN (`CATCH_UP_UNVERIFIED`, also when NULL); unplanned: attestation present (`ATTESTATION_REQUIRED`) and blocking preflight passes (`PREFLIGHT_BLOCKED`) | epoch N+1, active := target, `ACTIVE`; history with cut point, preflight, attestation; shard rows generation+1/expired/epoch; singleton leases, members and connector leases removed; `region_fanout` row |
| `region-mode` | expected epoch, `SAGAT_REGION_MODE` | not in recovery | mode changed; history `MODE_CHANGE` |
| `region-topology` | expected epoch, `SAGAT_REGION_REGIONS` | active region kept (`ACTIVE_REMOVED`); every namespace placement allows every region (`RESIDENCY_CONFLICT`, with the count of namespaces that forbid it); ≤ 100,000 namespaces (`TOO_MANY_NAMESPACES`) | regions changed; history `TOPOLOGY_CHANGE` |
| `region-divergence-report` | `SAGAT_REGION_FORMER_DATABASE_URL` (read-only role on the former primary), `SAGAT_REGION_REPORT_LIMIT` (10,000, ≤ 100,000 per table), `SAGAT_REGION_REPORT_PATH` (file, created `0600`, must not exist) | former database is not this database (system identifier differs) (`SAME_DATABASE`); current authority epoch > former's (`NOT_DIVERGED`) | read-only on both; writes the report file (identifiers and counts; schema below) and one history row `DIVERGENCE_REPORT` with counts only |
| `region-reingest-inbox` | `SAGAT_REGION_FORMER_DATABASE_URL`, `SAGAT_REGION_REPORT_PATH` (a report produced above), `SAGAT_REGION_REINGEST_LIMIT` (1,000, ≤ 10,000) | report belongs to this cell and current epoch (`REPORT_MISMATCH`) | per event: normal inbox admission (dedupe key, quotas) + tenant audit `event.reingested`; then one history row `REINGEST` with counts; refusals per event are counted by closed reason, never dropped silently |
| `region-emit-tombstones` | `SAGAT_REGION_REPORT_PATH` (a divergence report) | report belongs to this cell and current epoch (`REPORT_MISMATCH`); 020's sink pipeline present (`NO_SINK_PIPELINE`) | per run listed only in the former primary: one whole-run `run_tombstone` request through 020's pipeline (idempotent by `(run, epoch)`); history row `TOMBSTONES` with counts |
| `region-routing-manifest` | — | authority present | prints `{cell, endpoints: {api, worker, webhooks}, tenants: [slug…]}` from `SAGAT_CELL_ENDPOINTS`; read-only |

## Preflight of an unplanned activation

| Check | Kind | Pass when |
|---|---|---|
| `database_writable` | blocking | `pg_is_in_recovery()` is false and a write in the activation transaction succeeds |
| `epoch_cas` | blocking | expected epoch equals the authority's |
| `key_ring_coverage` | blocking | every key id referenced by sealed secret versions is in `SAGAT_SECRETS_KEYS` of the command's environment (ids only, nothing decrypted) |
| `key_fingerprints_blocking` | blocking | for `secrets_key_ring`, `script_signing`, `intent_binding`, `flow_control_digest`: the fingerprints of the keys in the command's environment equal `cell_key_fingerprints` (a key recorded there but absent here also blocks) |
| `key_fingerprints_warning` | warning | the same comparison for `push_signing`, `fairness_digest`, `decision_link`, `codec_assertion`, `template_index_pins` |
| `no_standby_follows_former` | blocking when a monitor connection is configured | no standby listed in the cell topology is visible in the promoted primary's `pg_stat_replication` as missing *and* in the former primary's (when reachable) as streaming |
| `openbao_reachable` | warning | the configured OpenBao answers health within 2 s |
| `blob_replica_lag` | warning | the replication monitor's last sample for objects is within the grace |
| `broker_reachable` | warning | the region's NATS answers within 2 s |

Warnings are printed and recorded in `region_history.preflight`; they never block. A blocking check that
cannot be evaluated is `blocked`, not `ok`.

## Divergence report file (`application/json`, classification `internal`, operator-only)

```json
{"cell": "c1", "formerEpoch": 1, "currentEpoch": 2, "cutLsn": "0/5A0012F8", "generatedAt": "…",
 "tables": {"runs": {"onlyInFormer": 12, "headDiffers": 3, "truncated": false,
                     "items": [{"tenantId": "…", "namespaceId": "…", "id": "…"}]},
            "inbox_events": {"onlyInFormer": 40, "truncated": false, "items": [{"tenantId": "…", "namespaceId": "…", "source": "…", "eventId": "…"}]},
            "webhook_deliveries": {…}, "command_dedupe": {…}, "audit_records": {…}}}
```

Identifiers only: no payload, no event body, no secret, no token, no SQL text. `source` and `eventId` are
tenant-provided identifiers already stored as `protected_system`/`content` per the storage manifest; the
file is an operator artifact with the same handling as a database backup.
