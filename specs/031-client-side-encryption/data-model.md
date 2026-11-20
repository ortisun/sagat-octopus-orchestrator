# Data model: client-side payload encryption

Draft; nothing is migrated. One additive migration, `crates/infra-postgres/migrations/<timestamp>_payload_encoding.up.sql`
(timestamp chosen at implementation, later than every migration on `develop` and on the parity branches; the latest at
the base commit is `20261220000000_reveal_grants_role_change`), follows `.claude/skills/migration/SKILL.md`.

## 1. Domain types (`crates/domain/src/payload_encoding.rs`, new; no serde_json, no crypto)

| Type | Meaning | Invariants |
|---|---|---|
| `Encoding` | `Encrypted` \| `Compressed` | closed; parsed from the two strings |
| `PointerPattern` | RFC 6901 segments, `*` = every element | 0–16 segments, at most 4 `*`, segment ≤ 128 bytes, total ≤ 512 bytes; no literal `*` property |
| `CarrierRef` | `Input`, `Output`, `Node(NodeId)`, `Heartbeat(NodeId)`, `Stream(ChannelName)` | closed; the last two are the side-channel carriers (019, 026) |
| `EncodedLocation` | `{carrier, pointer, encoding, level}` | `level` is a 016 `Level` |
| `PayloadPlan` | locations of one version | ≤ 64 per carrier; no location is a prefix of another (no nesting) |
| `EncodingRules` | `V0` (inert) \| `V1` | stored with the version; `UnsupportedRules(u8)` otherwise |
| `EnvelopeContext` | `{tenant, namespace, pointer}` (`ctx`) | built from the payload plan only; equality is exact |
| `EnvelopeHeader` | parsed header: version, codec chain, level, context, key reference presence, sizes | built only by the strict adapter parser; `Debug` prints version and chain only |
| `EnvelopeVerdict` | `Accepted` \| `Refused(EncodingRefusal)` | `EncodingRefusal` ∈ {`EncodingRequired`, `EnvelopeInvalid`, `EnvelopeLevelMismatch`, `EnvelopeContextMismatch`, `CarriageMismatch`} |
| `CarriageCheck` | source plan mapped through a carriage path vs. target plan | `Ok` iff every mapped source location is declared by the target with the same encoding and pattern (same document: equal sets) |
| `CodecLevelCeiling` | `Public` \| `Internal` \| `Pii` | never `Secret` |
| `CodecAudience` | the highest level an assertion may carry | `max_level(roles, grant, reveal_requested, ceiling, ceiling_confirmed)` |
| `KeyUsageScanState` | `Queued`, `Running`, `Succeeded`, `Failed`, `Cancelled` | transitions below |
| `EncodingActivation` | `Inactive` \| `Active{since, by}` | publication with rules 1 requires `Active` |

Pure rules: `EnvelopeHeader::verdict(&EncodedLocation, &EnvelopeContext) -> EnvelopeVerdict` (cipher present for
`Encrypted`, `lvl == level`, `ctx` equal to the run's context and the location pattern); `CarriageCheck::evaluate`;
`PayloadPlan::codec_need(node) -> Option<CodecNeed>` (`Cipher` if any `Encrypted` location in the node's decode, output,
checkpoint or stream plan, `AnyCodec` if only `Compressed`, `None` otherwise). New system failure codes in
`crates/domain/src/execution/event.rs`: `ENCODING_REQUIRED`, `ENVELOPE_INVALID`, `ENVELOPE_LEVEL_MISMATCH`,
`ENVELOPE_CONTEXT_MISMATCH`, `ENCODING_CARRIAGE_MISMATCH`. New permission `CodecManage` (`codec:manage`, admin only)
in `crates/domain/src/identity.rs`; new audit actions in `crates/domain/src/audit.rs`: `codec.endpoint_set`,
`codec.endpoint_removed`, `codec.assertion_issued`, `codec.assertion_denied`, `payload.key_usage_scan_started`,
`payload.key_usage_scan_finished`, `run.encoding_refused`, `payload.encoding_activated`, `payload.encoding_deactivated`.

The strict JSON parser of envelopes, the undeclared-envelope walk, the schema walk that derives plans and the
expression access-path analysis are adapter code in `crates/infra-ir` behind application ports, like the 016 walker.

## 2. State transitions

```text
Key-usage scan:  QUEUED --claim(lease)--> RUNNING --batch commit--> RUNNING ... --> SUCCEEDED
                 RUNNING --lease expiry--> RUNNING (reclaimed by another engine; cursor unchanged)
                 RUNNING --scanned limit--> FAILED(SCAN_LIMIT_REACHED)      QUEUED|RUNNING --cancel--> CANCELLED
Codec endpoint:  absent --PUT--> revision 1 --PUT(expectedRevision)--> revision n+1 --DELETE--> absent
                 (levelCeiling = pii and its setter loses the grant: ceilingConfirmed = false until a new PUT)
Activation:      Inactive --activate (018 registry: every live instance capable)--> Active --deactivate--> Inactive
```

No run or step state machine changes: refusals reuse the existing failed-step, failed-run and refused-admission paths.

## 3. PostgreSQL

### 3.1 `workflow_versions.encoding_rules` (column)

`smallint NOT NULL DEFAULT 0 CHECK (encoding_rules IN (0, 1))`. Existing rows stay `0`; no document, digest or row is
rewritten. Written only by the publish transaction (ADR-0018), which also reads the activation row (§3.7).

### 3.2 `workflow_version_encodings` and `workflow_version_codec_nodes` (new, immutable)

Encodings: `tenant_id`, `workflow_version_id` (FK `(tenant_id, workflow_version_id)` → `workflow_versions (tenant_id,
id)`), `carrier` (`CHECK IN ('input','output','node','heartbeat','stream')`), `carrier_key text` (`''` for workflow
carriers, else node id or channel name, ≤ 64 bytes), `ordinal smallint` (0–63), `pointer text` (`octet_length ≤ 512`),
`encoding` (`'encrypted'|'compressed'`), `level` (`'public'|'internal'|'pii'|'secret'`). PK `(tenant_id,
workflow_version_id, carrier, carrier_key, ordinal)`.

Codec nodes: `tenant_id`, `workflow_version_id`, `node_id text`, `needs_cipher boolean`; PK `(tenant_id,
workflow_version_id, node_id)`; one row per node whose decode, output, checkpoint or stream plan is non-empty (with an
encoded input, every task-producing node). The claim filter joins it on the run's version (FR-031-015).

Both tables: `BEFORE UPDATE OR DELETE` trigger raises (`ENABLE ALWAYS`); versions are never deleted. Grants: `sagat_app`
SELECT, INSERT. Classification: `pointer`, `carrier_key` and `node_id` are `content` (author-derived text, a second
copy of document bytes, ADR-0027); enums and booleans are `protected_system`.

### 3.3 `worker_sessions` (columns)

`codec_cipher boolean NOT NULL DEFAULT false`, `codec_compression boolean NOT NULL DEFAULT false`, set at registration
from `WorkerCapabilities.payload_codec` only when the negotiated minor is at least the codec minor. Fixed size.

### 3.4 `namespace_codec_endpoints` (new)

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `namespace_id` | uuid | PK; FK `(tenant_id, namespace_id)` → `namespaces (tenant_id, id)` |
| `url` | text | ≤ 512 bytes, https (http only for loopback) |
| `origin` | text | ≤ 256 bytes, derived server-side from `url` |
| `level_ceiling` | text | `CHECK IN ('public','internal','pii')` (coordinator C9) |
| `ceiling_set_by` | text | the subject who set a `pii` ceiling (grant rechecked at each `pii` assertion); null otherwise |
| `revision` | bigint | ≥ 1, incremented per change |
| `updated_by` | text | bounded subject, as other actor columns |
| `updated_at` | timestamptz | database clock |

Grants: SELECT, INSERT, UPDATE, DELETE. Classification: `url`, `origin` `content` (tenant configuration); subjects as
existing actor columns.

### 3.5 `payload_key_usage_scans` and `payload_key_usage_results` (new)

Scans: `tenant_id, namespace_id, id` (PK `(tenant_id, id)`), `state` (closed CHECK), `requested_by`, `requested_at`,
`started_at`, `finished_at`, `cursor_run_id uuid NULL`, `scanned_runs bigint`, `scanned_payloads bigint`,
`blob_envelopes bigint` (root envelopes in blobs are counted, their headers are not re-read), `results_truncated bool`,
`failure_code text NULL CHECK IN ('SCAN_LIMIT_REACHED','UNAVAILABLE')`, `lease_owner text NULL`,
`lease_expires_at timestamptz NULL`. Unique partial index `(tenant_id, namespace_id) WHERE state IN
('QUEUED','RUNNING')` enforces one active scan. At most 50 scans are kept per namespace: starting a scan deletes the
oldest finished ones beyond 49 in the same transaction.

Results: `tenant_id, scan_id, kid text NOT NULL DEFAULT '', kv text NOT NULL DEFAULT '', algorithm text CHECK IN
('aes-256-gcm','none'), encoding text CHECK IN ('encrypted','compressed'), envelopes bigint, oldest_payload_at,
newest_payload_at`; PK `(tenant_id, scan_id, kid, kv, algorithm, encoding)`; FK to the scan `ON DELETE CASCADE`; at most
1,024 rows per scan (the batch transaction sets `results_truncated` and stops adding keys beyond it). Counting is per
reference: a payload referenced by two runs (a rerun) counts twice; the API description says so.

Grants: SELECT, INSERT, UPDATE on scans (lease and progress), SELECT, INSERT, UPDATE, DELETE on results.
Classification: `kid`, `kv` `content` (customer-chosen text); the rest `protected_system`.

### 3.6 `event_deliveries.refusal_code` (column)

`text NULL CHECK (refusal_code IN ('ENCODING_REQUIRED','ENVELOPE_INVALID','ENVELOPE_LEVEL_MISMATCH',
'ENVELOPE_CONTEXT_MISMATCH','ENCODING_CARRIAGE_MISMATCH'))`. A refused match inserts the delivery row with the code, so
the dispatcher does not retry that consumer (ADR-0012 point 2) and the wait keeps waiting. Closed enum.

### 3.7 Fleet capability and activation (FR-031-036)

031 defines no member table. It uses the instance-capability registry owned by feature 018 (`serving_instances`,
`platform_requirements`, `platform_requirement_epoch`; FR-018-052, ADR-0050 decision 9, "Instance-capability registry"
in `specs/018-schedules-bulk-operations/data-model.md`):

- Every control-plane and execution-engine binary that implements this feature lists the capability
  `payload-encoding-v1` in its `serving_instances.capabilities`.
- `payload_encoding_activation` (new, singleton, the feature's own activation row that 018's rule 4 writes):
  `state` (`'INACTIVE'|'ACTIVE'`), `changed_by`, `changed_at`. Changed only by the audited operator command
  `control-plane payload-encoding activate|deactivate`, which runs 018's activation transaction: lock
  `platform_requirement_epoch` `FOR UPDATE`, verify that every live `CONTROL_PLANE` and `EXECUTION_ENGINE` instance lists
  `payload-encoding-v1`, insert the requirement (kinds both) and bump the epoch, write this row, audit. Deactivation
  removes the requirement only when no version with `encoding_rules = 1` is still publishable, and never relaxes the
  validation of already published rules-1 versions.

Grants: the runtime role reads `payload_encoding_activation`; only the operator command changes it. Registry grants
are 018's. Closed values: `protected_system`.

### 3.8 Unchanged

`payloads`, `run_events`, `runs`, `steps`, `blobs`, the audit table and the outbox keep their schema. Envelopes are
stored as ordinary payload bytes; their size is charged by the existing triggers.

## 4. Down migration

Refuses (`RAISE EXCEPTION ... ERRCODE = 'object_not_in_prerequisite_state'`) when any version has `encoding_rules = 1`,
the activation row is `ACTIVE`, any endpoint or scan row exists, or any delivery has a `refusal_code`: older code would
ignore declared encodings and accept plaintext. Otherwise drops the new objects; 018's registry tables are not touched.

## 5. Retention

Plans live as long as their version (forever today). Endpoints live until deleted. Scans: last 50 per namespace.
Assertions are never stored (only their audit record). Envelopes follow payload retention, which is undecided (G69);
operators must keep the customer key versions while data under them may exist (G1534). 019 checkpoints and 026 stream
events holding envelopes follow those features' stores in 020's content-store registry (coordinator C8).

## 6. Tenant boundary

Every table has `tenant_id` first in its key and tenant-composite foreign keys; every query filters by the authenticated
tenant and the resolved namespace; a foreign row is indistinguishable from a missing one. The claim filter joins
`workflow_version_codec_nodes` on `(tenant_id, workflow_version_id)` of the run being claimed. Every envelope's `ctx`
names exactly one tenant and namespace, checked at every write.
