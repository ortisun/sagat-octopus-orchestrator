# Data Model: Additional script languages (feature 023)

The tables of feature 005 (`crates/infra-postgres/migrations/20261014000000_scripts.up.sql`) remain the
model: `scripts`, `script_versions`, `script_artifacts`, `script_build_jobs`, `script_test_runs`,
`script_executions`. Every table carries `tenant_id` and composite foreign keys, so the database refuses
a reference across tenants. This feature adds one constraint change and three tables in a single additive
migration `crates/infra-postgres/migrations/<next>_script_languages.{up,down}.sql`; the
version number is allocated at implementation because parallel features share the sequence (G1213).

## Changes to existing tables

| Table | Change | Notes |
|---|---|---|
| `scripts` | `scripts_language_valid` becomes `CHECK (language IN ('typescript','python','csharp','go','java','bash','powershell','php','rust','python-data'))` | Dropped and re-added in one transaction; existing rows satisfy both; `python-data` rows can exist only once feature 024 enables it |
| `script_versions`, `script_test_runs`, `script_executions` | none | `language` is copied from the script by the store; the domain `FromStr` is the guard, as today |
| `script_artifacts` | none | Stays content-addressed and insert-once; provenance is per build, in `script_build_provenance` (D1227) |

## New table `script_build_provenance`

One signed statement per build (D1227, architecture review S2). The artifact row is shared by every
build that produces the same digest; the statement is not.

| Column | Type | Constraint |
|---|---|---|
| `tenant_id` | `uuid` | `NOT NULL`, part of PK |
| `build_key` | `text` | `NOT NULL CHECK (build_key ~ '^sha256:[0-9a-f]{64}$')`, part of PK |
| `artifact_digest` | `text` | `NOT NULL`; `FOREIGN KEY (tenant_id, artifact_digest) REFERENCES script_artifacts (tenant_id, digest)` |
| `key_id` | `text` | `NOT NULL CHECK (octet_length(key_id) BETWEEN 1 AND 64)` |
| `envelope` | `bytea` | `NOT NULL CHECK (octet_length(envelope) BETWEEN 1 AND 65536)` (DSSE JSON) |
| `created_at` | `timestamptz` | `NOT NULL DEFAULT clock_timestamp()` |

Primary key `(tenant_id, build_key)`. Inserted in the build-completion transaction with `ON CONFLICT DO
NOTHING` (a repeated completion of the same build adds nothing); an `ALWAYS` trigger refuses update and
delete. A version reaches its statement through its own `build_key`; a version finished from the
build-key cache therefore shows the statement of the build it reused, which is accurate. Builds made
before this feature have no row (G1217).

## New table `script_runtime_policies`

One current, revisioned policy document per namespace. A missing row means the deployment default
(every built language enabled, the whole catalog allowed; O1201).

| Column | Type | Constraint |
|---|---|---|
| `tenant_id` | `uuid` | `NOT NULL`, part of PK |
| `namespace_id` | `uuid` | `NOT NULL`, part of PK; `FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id)` |
| `revision` | `bigint` | `NOT NULL CHECK (revision >= 1)` |
| `document` | `jsonb` | `NOT NULL CHECK (jsonb_typeof(document) = 'object' AND octet_length(document::text) <= 16384)` |
| `updated_by` | `text` | `NOT NULL CHECK (octet_length(updated_by) BETWEEN 1 AND 255)` |
| `updated_at` | `timestamptz` | `NOT NULL DEFAULT clock_timestamp()` |

Primary key `(tenant_id, namespace_id)`. The document's closed shape (validated by the domain before the
write; the CHECK only bounds it):

```json
{ "languages": {
    "bash":       { "enabled": false },
    "rust":       { "enabled": true, "packages": { "allow": ["serde_json"] } },
    "php":        { "enabled": true, "packages": "none" },
    "powershell": { "enabled": true } } }
```

`enabled` defaults to `true`; `packages` is `"catalog"` (default), `"none"` or `{"allow": [names]}` with
at most 256 names, each present in the deployment catalog of that language at write time. A language not
built by the platform cannot appear. An update is `UPDATE … WHERE revision = $expected` (`409` with the
current revision otherwise) or an insert when no row exists and the expected revision is 0.

## New table `script_runner_presence`

Advisory evidence of what a runner of the tenant advertised recently (D1219). Never read by admission.

| Column | Type | Constraint |
|---|---|---|
| `tenant_id` | `uuid` | `NOT NULL REFERENCES tenants (id)`, part of PK |
| `runner_subject` | `text` | `NOT NULL CHECK (octet_length(runner_subject) BETWEEN 1 AND 255)`, part of PK |
| `languages` | `text[]` | `NOT NULL CHECK (cardinality(languages) <= 10)`; each element one of the ten ids |
| `native_target` | `text` | `NULL CHECK (native_target IN ('linux-amd64','linux-arm64'))` |
| `toolchains` | `jsonb` | `NOT NULL CHECK (octet_length(toolchains::text) <= 8192)` (language → toolchain identity, each ≤ 512 bytes) |
| `seen_at` | `timestamptz` | `NOT NULL DEFAULT clock_timestamp()` |

Primary key `(tenant_id, runner_subject)`; index `(tenant_id, seen_at DESC)`. The claim handler upserts
with `ON CONFLICT (tenant_id, runner_subject) DO UPDATE … WHERE script_runner_presence.seen_at <
clock_timestamp() - interval '60 seconds'`; an insert is refused when the tenant already has 256 rows
(the statement counts under the tenant's presence advisory lock). Scheduler housekeeping deletes rows
older than 24 hours. The database clock decides freshness: served means `seen_at` younger than 120 s.

## Grants and classification

- `sagat_app`: `SELECT, INSERT` on `script_build_provenance`; `SELECT, INSERT, UPDATE` on `script_runtime_policies` (no `DELETE`: the row is current
  state, a reset is an update); `SELECT, INSERT, UPDATE, DELETE` on `script_runner_presence`.
- `crates/infra-postgres/storage-classification.json` gains: `script_runtime_policies.document` and
  `updated_by` as `content` (tenant-authored configuration, ADR-0027); `script_build_provenance.envelope`
  as `content` (like the SBOM), `build_key`, `artifact_digest` and `key_id` as `protected_system`; `script_runner_presence.languages`, `native_target`, `toolchains` and
  `runner_subject` as `protected_system` with their CHECK bounds as evidence. The manifest test against
  `pg_catalog` fails if any of them is missing.

## Entities that are not rows

- **Language** (`crates/domain/src/scripts.rs`): id, file extension, artifact family, default runtime
  policy, build profile. `is_built()` is true only once its builder, runtime, probe and validator ship.
- **Package catalog** (`runners/{rust,php}/catalog/catalog.json`, embedded in `crates/infra-schema`):
  entries per [catalog.schema.json](contracts/catalog.schema.json); its SHA-256 is the catalog digest in
  the toolchain identity. Changing an entry is an image release, never a database write.
- **Lockfile**: the existing `scripts.lockfile` text (≤ 256 KiB). For Rust, a Cargo.lock version 4; for
  PHP, a `composer.lock`; for `python-data`, a hash-pinned requirements file (`name==version
  --hash=sha256:<hex>` per line); for Bash and PowerShell it must be empty. Validation results are not stored;
  the version's build key and SBOM are the record.
- **Provenance statement**: the DSSE envelope stored in `script_build_provenance`
  ([provenance.md](contracts/provenance.md)).
- **Image-catalog runtime** (`python-data`, D1229): a runtime image whose catalog digest an artifact
  binds in `runtimeCatalog`; the catalog file and image belong to feature 024.

## Artifact bundles

Bash (v1, unchanged format, new language value):

```json
{"entry":"main.sh","files":{"main.sh":"…"},"inputSchema":null,"language":"bash","outputSchema":null,"policy":"v1;memory=268435456;cpu=1000;wall=30;pids=64;output=65536;scratch=33554432;egress=none","v":1}
```

Rust (v3, unchanged format, new language value): `{"abi":"sagat-inline-v1","encoding":"base64+gzip",
"entry":"script.elf","files":{"script.elf":"<base64 gzip>"},"language":"rust","target":"linux-arm64",…,"v":3}`.

PHP (v4, new; [schema](contracts/artifact-bundle-v4.schema.json)):

```json
{"entry":"main.php","files":{"main.php":"…","vendor/autoload.php":"…","vendor/composer/autoload_classmap.php":"…","vendor/acme/pkg/src/Thing.php":"…"},
 "inputSchema":null,"language":"php","lockDigest":"sha256:…","outputSchema":null,"policy":"v1;…","runtimeCatalog":null,"v":4}
```

`runtimeCatalog` is `null` for `php`. `python-data` (reserved, D1229) uses the same v4 shape with entry
`main.py` and a non-null `runtimeCatalog` (`sha256:…` of the runtime image's catalog). Keys sorted, no
whitespace, `digest = sha256(bytes)`, signed as in 005 (`tenant ‖ digest ‖ language`).

## Build key and toolchain identity

The 005 build key is unchanged: SHA-256 over length-prefixed `sagat/build/v1`, language, source,
lockfile, toolchain, canonical policy and schemas, scoped to the tenant. Example toolchain identities:

- `build=sha256:…;runtime=sha256:…;protocol=artifact-v1;lang=bash;version=bash-5.x;shim=sha256:…`
- `build=sha256:…;runtime=sha256:…;protocol=artifact-v3;lang=rust;version=rustc-1.98.0;target=linux-arm64;abi=sagat-inline-v1;compression=gzip-canonical-v1;shim=sha256:…;catalog=sha256:…`
- `build=sha256:…;runtime=sha256:…;protocol=artifact-v4;lang=php;version=php-8.x;composer=2.x;extensions=sha256:…;shim=sha256:…;catalog=sha256:…`

## Data this feature does not store

The reserved host calls of FR-023-028 add no table, column or record here. A checkpoint's details are
held only in the runner's memory until the next heartbeat carries them, and are stored by feature 019
(`step_checkpoints` in its draft); stream events are stored by feature 026 in its own log (ADR-0068).
Retention and erasure of both are registered by their owners in 020's content-store registry (C8).
Neither payload enters `script_executions`, the script log, audit or telemetry; only closed-label counts
do (plan.md, Observability).

## State and transitions

No new state machine. Versions keep `BUILDING → READY | FAILED`; build jobs and test runs keep
`QUEUED → RUNNING → SUCCEEDED | FAILED` with leases; executions are once per `(run, node, attempt)`.
A policy has revisions `1, 2, …`; each update replaces the document. Presence is fresh (< 120 s) or stale.

## Invariants

- What runs is exactly what was signed: the runner verifies digest and signature, then the bundle
  version's own rules (v1/v3/v4), before starting a container; a Rust target mismatch never runs.
- An artifact's SBOM package set equals its version's lockfile package set; otherwise it was not signed.
- A provenance row names exactly its build's artifact digest as subject and verifies with its `key_id`.
- A `python-data` artifact never runs on a runtime whose catalog digest differs from its `runtimeCatalog`.
- A script step whose artifact language is disabled in its namespace is never leased (D1226).
- A policy never names a language the platform does not build or a package outside the catalog.
- Admission and leasing never read presence; publication of a workflow never depends on a runner being up.
- No row, log, audit entry, metric label or error holds a secret value, a source, an input or a result.

## Retention

Artifacts, SBOMs and provenance rows are never deleted (as 005 artifacts). Policy rows are current state with
history in `script.policy_changed` audit entries. Presence rows live at most 24 hours. Existing script
retention (1–365 days, namespace ceiling) governs test runs and executions of every language.

## Migration

Up: drop and re-add the language constraint, create the three tables with grants and classification
entries. Down: refuses (raises) if any `scripts` row has a new language, any provenance row or any policy
row exists; otherwise drops the tables and restores the five-language constraint. Forward and refused
rollback are tested with data present.

Rollback is therefore binary-only (D1230). Every build after the upgrade writes a provenance row, so in
practice the schema is not rolled back: 0.2.0 binaries run against the migrated schema (they ignore the
new tables) only while no new-language script exists; once one exists, a 0.2.0 control plane fails any
script listing that contains it, because `language_of` in `crates/infra-postgres/src/scripts.rs` maps an
unknown language to an internal store error (G1220). T023-063 records the observed behaviour of both
cases.
