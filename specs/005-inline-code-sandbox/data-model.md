# Data Model: Script registry, code editor and inline execution

## Context

Allow writing and publishing code directly on the platform without manual application build/deploy.

Data ownership follows the bounded contexts of `docs/architecture/overview.md`, with the reference physical schema in `specs/001-platform-foundation/data-model.md`. The tables of this feature are the migration `crates/infra-postgres/migrations/20261014000000_scripts.up.sql`; the rules are `crates/domain/src/scripts.rs`, the services and ports `crates/application/src/{scripts,sandbox,script_signing}.rs`, the SQL `crates/infra-postgres/src/scripts.rs`. Every table carries `tenant_id` and every foreign key is composite, so the database itself refuses a version of another tenant's script.

## Entities and observable state

- **Script** (`scripts`): a name in a namespace, a language and the **draft** an author edits (source, lockfile, input and output JSON Schemas, policy, retention). The draft carries a `revision`; a change is `UPDATE … WHERE revision = $n`, so two editors never overwrite each other (`409` with the current revision). `latest_version` is the number of the last version published. The only mutable entity of the feature.
- **Script version** (`script_versions`): the draft frozen at the moment of publishing, numbered 1, 2, 3 within its script and never reused. Status moves forward once: `BUILDING` → `READY` (an `artifact_digest`, the `key_id` and `toolchain` that made it, the `build_key`, the log) or `FAILED` (a `failure`: `rejected:`, `lockfile:`, `limit:`, `unsupported:` or `unavailable:` and the first message). `content_digest` is the SHA-256 of everything that defines the version; publishing a draft with the same one as the latest version that has not failed is not another version. A trigger refuses every update that changes more than how the build ended, and every delete.
- **Artifact** (`script_artifacts`): what a build made, once per digest and tenant: the bytes (a canonical JSON bundle of the code, the policy it was built under and the schemas), the Ed25519 signature the control plane made of `tenant ‖ digest ‖ language`, the id of the key, the software bill of materials (CycloneDX) and the toolchain (`<image id>;shim=<hash>`). Immutable by trigger. At most 2 MiB.
- **Build job** (`script_build_jobs`): one per version. `QUEUED` → `RUNNING` (a lease: the hash of a token only the holder knows, and a time) → `SUCCEEDED` or `FAILED`. A lease that ran out lets another runner take the job (attempts counted, three at most); everything the holder reports is checked against the token.
- **Test run** (`script_test_runs`): the draft as it was when the run was asked for, an input (≤ 240 KiB), and what came of it: `QUEUED` → `RUNNING` → `SUCCEEDED` or `FAILED`, with the result, a `SCRIPT_*` failure code, the redacted log, what the script was refused, the numbers of the run (wall, start, peak memory) and the schemas inferred from the input and the result. It has a `deadline_at`: a run no runner takes by then is failed as unavailable (lazily, when read or when the namespace's quota is counted). Kept for the script's retention after it finishes.
- **Execution** (`script_executions`): what a published script did in a step, once per `(run, node, attempt)`; the redacted log (≤ 1 MiB), the failure code, what it was refused, the numbers, the trace id; `expires_at` is the creation time plus the script's retention (1 to 365 days, 14 by default). Purged by the scheduler's housekeeping.
- **Audit** (existing table): `script.created`, `script.changed`, `script.deleted`, `script.published`, `script.built`, `script.tested`, and `script.violation` (what a script was refused: `fetch <scheme>://<host>` or `secret <name>`; written once per actor, resource and reason, however often it asks).
- **Quotas** (existing tables, two new resources): `scripts` (500 per namespace) and `script_jobs` (builds and test runs in progress, 8 per namespace), counted under a lock in the transaction that creates the thing counted.
- **Membership role** `runner` joins `admin`, `developer`, `operator`, `viewer` and `worker`.

## The artifact bundle

```json
{ "v": 1, "language": "typescript", "policy": "v1;memory=268435456;cpu=1000;wall=30;pids=64;output=65536;scratch=33554432;egress=none",
  "inputSchema": null, "outputSchema": null, "entry": "main.mjs", "files": { "main.mjs": "…" } }
```

Keys are sorted and there is no whitespace; `digest = sha256(bytes)`. File names are `[A-Za-z0-9._-]{1,64}`, not starting with a dot; at most 32 files. The runner refuses a bundle that does not parse, whose language is not its row's, or whose policy does not hold (`Policy::parse_canonical` re-checks every floor and ceiling).

## The build key

`sha256(len ‖ part)` over: `sagat/build/v1`, language, source, lockfile, toolchain, policy (canonical text), input schema, output schema. Each part is length-prefixed, so no two tuples hash alike. A READY version of the tenant with the same key is the cache: the claim finishes a build from it and never hands it to a runner. A change of any part (including a new image or a new shim) builds again.

## Invariants

- Every tenant-scoped record validates the security context; another tenant's script, version, test run or artifact is a `404` indistinguishable from a missing one, and the probe is audited.
- A published version never changes (trigger); an artifact never changes (trigger); neither is ever deleted. A script with a version cannot be deleted.
- What runs is exactly what was signed: the runner checks the digest of the bytes and the signature (tenant, digest, language) before it starts a container.
- A lease is held by one runner at a time, and a report is accepted only with the token of the current holder.
- The policy a script runs under is in its artifact, clamped by the ceiling of the platform and of the runner; it is never what a request says at run time.
- No log, record, audit entry, error or metric label holds a secret value, a signature or the path or query of a URL.
- Large payloads: results are at most 240 KiB (the engine's inline limit); bigger ones need the blob store (feature 002's gap).

## Migrations

Additive: new tables, new quota resources, a new role in the membership constraint. Rollback (`…scripts.down.sql`) drops the tables and the guards in dependency order, deletes `runner` memberships, and is tested with data present. The quota constraint lists are replaced, so a branch that adds a resource of its own (`bugfix/spec-alignment` does not) must merge the lists.
