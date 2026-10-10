# Runbook: scripts and the sandbox runner (feature 005)

Operational notes for the part of the platform that runs code its tenants wrote. Read `control-plane.md` and `execution-engine.md` first. Architecture and the reasons: `docs/adr/ADR-0013-…`; what is not built: the gaps of `specs/005-inline-code-sandbox/research.md`, listed at the end so nobody assumes them.

## 1. What runs where

| Process | Holds | Never holds |
|---|---|---|
| **Control plane** (`control-plane serve`) | The scripts, versions, artifacts, build and test queues and execution records (PostgreSQL); the **signing key** (`SAGAT_SCRIPT_SIGNING_KEY`) | The container engine. It never starts a container or runs a script |
| **Execution engine** (`execution-engine serve`) | The worker protocol, the scheduler, the leases of script steps | Anything specific to scripts: a script step is an activity named `script.sha256-<hex>` |
| **Sandbox runner** (`sandbox-runner serve`) | The **container engine** (its socket), one tenant's `runner` token | A database connection, the signing key (only its public half, if given), the secrets of the tenant except the ones a step is leased |
| **The container** (one per build and per run) | The code, its input, its scratch area | A network, the host, the engine's socket, any secret it was not given, any file of the platform |

Without `SAGAT_SCRIPT_SIGNING_KEY` the control plane does not serve the script routes at all (`404`): an artifact is only trusted if the platform signed it.

## 2. Setting it up

1. **The signing key.** `control-plane script-key` prints a new Ed25519 key (PKCS#8, base64) to standard output and nothing else; keep it in the secret store of the control plane and set `SAGAT_SCRIPT_SIGNING_KEY`. Several control-plane replicas must share it. The key id (first 8 bytes of the SHA-256 of the public key) is on every artifact. **Rotation** is not built (G101): replacing the key makes every existing artifact unverifiable by a runner that was given the old public key and re-signs nothing. Until it is, treat the key like the database password.
2. **The ceiling.** `SAGAT_SCRIPT_CEILING` is the most any script may be given (`v1;memory=<bytes>;cpu=<millis>;wall=<secs>;pids=<n>;output=<bytes>;scratch=<bytes>;egress=none|namespace`); the default is 256 MiB, one core, 30 s, 64 processes, 64 KiB of log, 32 MiB of scratch and **no network**. A script may ask for less and is refused (`ABOVE_CEILING`) for more. Each runner has a ceiling of its own (`SAGAT_RUNNER_CEILING`); a script runs under the smaller of the two. Letting scripts use the network (`egress=namespace`) is a decision about the namespace's egress policy too: a script can reach only what that policy allows, never loopback, link-local, private or metadata space (`security-governance.md`).
3. **The runner's identity.** A membership with the role `runner` in the tenant (`INSERT INTO memberships` as the owner role, like a worker's), a token from your identity provider with that subject and the tenant claim, and a **worker binding** for the script activities of every namespace it serves:
   `PUT /v1/namespaces/{ns}/worker-bindings/{runner}` with `{"activities": ["script.*"]}`. The role holds `worker:execute`, `script:build` and `script:execute` and nothing else: it cannot read, write or publish a script through the author's API, read a secret, or touch another tenant. A runner serves **one tenant** (its token's); a pool shared by several tenants is not built (G99).
4. **The runner process.** Environment: `SAGAT_RUNNER_API_URL`, `SAGAT_RUNNER_GRPC_URL` (https outside `SAGAT_ENV=dev`), `SAGAT_RUNNER_TOKEN_FILE` (read at every call: rotate the token by rewriting the file), `SAGAT_RUNNER_CA_FILE` for a private authority, `SAGAT_RUNNER_NAMESPACES=slug=id,…` for the namespaces whose script steps it serves (a runner with none builds and runs test runs only), `SAGAT_RUNNER_MAX_CONCURRENT` containers at once, and `SAGAT_RUNNER_TRUSTED_KEYS` — the public keys of the control plane, base64, so the runner does not have to ask it for its key (without them it asks, and trusts the answer from then on: trust on first use, G102). `sandbox-runner help` lists the rest.
5. **Images.** `SAGAT_RUNNER_IMAGE_TYPESCRIPT` (default `node:24-alpine`) and `SAGAT_RUNNER_IMAGE_PYTHON` (default `python:3.13-alpine`). The toolchain identity in every build key is the engine's image id plus the hash of the runner's shims: a new image or a new runner release builds everything again, on purpose. Name an image by digest in production; a tag moves.

## 3. The runner is a high-value host

Whoever can reach the container engine's socket can start a privileged container that mounts the host, so **whoever controls the runner controls its machine**. The sandbox is as strong as the engine's runtime under it (`runc` here); a kernel or runtime bug is an escape. Therefore:

- Run the runner on machines that hold nothing else of value, in their own network segment with no route to the database, the control plane's admin port or the cloud metadata service (the containers have none, but the runner itself does). Block `169.254.169.254` for the host.
- Prefer a rootless engine or a sandboxed runtime (gVisor, Kata) on shared deployments: it is a setting of the engine (`--runtime`), not of the code (G98). The code asks for nothing the default runtime lacks.
- The runner's token is worth exactly one tenant's script sources and artifacts. Keep it out of the environment (`SAGAT_RUNNER_TOKEN_FILE`) and rotate it.
- Containers are labelled `sagat.sandbox=1`; the runner sweeps the ones older than `SAGAT_RUNNER_SWEEP_AFTER_SECS` (default an hour) every five minutes, which is what a runner that was killed mid-run leaves behind. `docker ps --filter label=sagat.sandbox=1` shows what is running.
- A container is created with: no network (`none`), read-only root, a `/tmp` of the policy's scratch size mounted `noexec,nosuid,nodev`, all capabilities dropped, `no-new-privileges`, the engine's default seccomp profile, user `65534`, memory with swap off, a CPU quota and a CPU-time limit, a process limit, a file-size limit, 256 open files, no core dumps, no log driver and nothing mounted. If an operator changes any of these in the engine's configuration, the guarantees change with it.

## 4. Operating

**Queues.** Builds and test runs are leased for 60 s, heartbeat every 20 s, and taken again by another runner when the lease runs out (three tries for a build, three for a test run). A test run nobody takes within two minutes fails as `SCRIPT_SANDBOX_UNAVAILABLE`. Script **steps** are the engine's: the runner polls (`SAGAT_RUNNER_POLL_MS`), asks the control plane which script activities have a step ready, advertises them in a worker session (at most 256 per session, eight sessions per namespace, renewed every 30 minutes) and runs what it is leased up to `SAGAT_RUNNER_MAX_CONCURRENT`.

**Quotas** (per namespace, `PUT /v1/namespaces/{ns}/quotas/{resource}`): `scripts` (500) and `script_jobs` (8 builds and test runs in progress). A `429 QUOTA_EXCEEDED` on publish or on a test run means the namespace has 8 in progress: a runner is down or slow.

**Retention.** Execution records and test runs are kept for the script's `retentionDays` (1–365, default 14) after they finish and purged by the scheduler's housekeeping. Artifacts and versions are kept for as long as the script exists (a script with a version cannot be deleted).

**Metrics.** On the runner's admin listener (`SAGAT_ADMIN_ADDR`, `/metrics`): `script_builds_total{outcome}`, `script_build_seconds{language}`, `script_start_seconds{language,temperature}` (cold is the first run of a language since the process began, warm the rest), `script_run_seconds{language}`, `script_memory_peak_bytes{language}`, `script_runs_total{language,outcome}` (`outcome` is `succeeded` or the `SCRIPT_*` code), `script_denied_total{kind}` (`fetch` or `secret`), `script_records_lost_total` (an execution record that could not be kept after three tries). On the control plane: `script_builds_total{outcome=cached|completed}`, `script_executions_total{status}`, plus the existing request and quota metrics. Nothing carries a tenant, a script name, a digest or a secret as a label.

**Audit** (`audit_records`): `script.created`, `script.changed`, `script.deleted`, `script.published`, `script.built` (the runner is the actor), `script.tested`, and **`script.violation`**: what a script asked for and was refused (`fetch <scheme>://<host>` or `secret <name>`; never a path, a query or a value), once per actor, resource and reason however often it asks. A rise in violations from one script is a script that is probing; the execution record (`GET /v1/namespaces/{ns}/script-executions?digest=…`) has the log.

## 5. Failure codes

| `failure` | Meaning | Retried by the engine? |
|---|---|---|
| `SCRIPT_ERROR` | The script raised an error or exited non-zero. The detail is in its execution record, not in the step's message | yes (the node's policy) |
| `SCRIPT_TIMEOUT` | Still running when the policy's time ran out; the container was killed | yes |
| `SCRIPT_OUT_OF_MEMORY` | The kernel killed it at the memory limit | no |
| `SCRIPT_OUTPUT_LIMIT` | A result over 240 KiB, or a flood of output | no |
| `SCRIPT_DENIED` | It did not catch a refusal (a request or a secret it may not have) | no |
| `SCRIPT_SCHEMA_VIOLATION` | The input or the result does not match the schema; the message names the rule and the path, never the value | no |
| `SCRIPT_NO_RESULT` | It ended without a result | no |
| `SCRIPT_SANDBOX_UNAVAILABLE` | The platform could not run it (engine down, image missing, artifact could not be fetched, no runner took a test run) | yes |
| `SCRIPT_UNTRUSTED_ARTIFACT` | The artifact does not match its digest or its signature does not verify; **nothing ran**. Treat as a security event | no |
| `SCRIPT_BUILD_REJECTED` | (Test runs) the source does not build | — |

## 6. Troubleshooting

| Symptom | Likely cause | What to do |
|---|---|---|
| Script routes answer `404` | The control plane has no `SAGAT_SCRIPT_SIGNING_KEY` | Make one with `control-plane script-key` and restart |
| Versions stay `BUILDING`; test runs `QUEUED` then `SCRIPT_SANDBOX_UNAVAILABLE` | No runner for the tenant, or none has the language's image (`no toolchain here` in its log) | Start a runner with a token of that tenant; `docker pull` the images |
| Runner log `no language can be built here` at start | The container engine is not reachable (`DOCKER_HOST`, socket permissions) | Fix access; the process exits rather than serve nothing |
| `SCRIPT_UNTRUSTED_ARTIFACT` on a step | The row changed after it was signed, or the runner trusts another key than the one that signed | Compare `keyId` with `SAGAT_RUNNER_TRUSTED_KEYS`; if rows were edited by hand, find out who |
| Steps of `script.*` activities never leave READY | The runner is not bound to `script.*` in that namespace (`worker.denied` in the audit), or the namespace is missing from `SAGAT_RUNNER_NAMESPACES` | Bind it; add `slug=id` |
| `PUT` of a script: `422 ABOVE_CEILING` | The request asks for more than `SAGAT_SCRIPT_CEILING` | Ask for less, or raise the ceiling on purpose |
| `422 UNKNOWN_SCRIPT` publishing a workflow | The digest is not a ready artifact of this tenant (typo, build failed, another tenant's) | Read the version: `status`, `failure`, `artifactDigest` |
| Version `FAILED` with `lockfile:` | The lockfile pins a package; installing packages is not built | Remove the dependencies (G97) |
| The runner's log shows `429` (rate limited) from the control plane | The runner is one identity and its calls (claims, heartbeats, artifacts, records: about four per script step, a handful per second when idle) count against the per-subject budget of the API (`SAGAT_SUBJECT_RATE_LIMIT_RPS`, 20 by default) | Raise `SAGAT_SUBJECT_RATE_LIMIT_RPS` and `_BURST` on the control plane (the worker protocol has its own, larger, budget); a runner is not a person |
| Container count grows on the runner | A runner died mid-run | The sweeper removes labelled containers after an hour; `docker rm -f` earlier if you must |

## 7. Not covered (read before relying on this)

- **C#, Go and Java** are named by the workflow schema and refused (`LANGUAGE_NOT_BUILT`). Roslyn and the isolated .NET host are not built (G96).
- **Dependencies.** A lockfile that pins a package is refused; there is no registry access, no allowlist, no vulnerability check. The SBOM lists the toolchain only (G97).
- **A hardened runtime** is the deployment's job (G98). **Multi-tenant runners**, **client-credentials auth for the runner**, **key rotation**, **namespace discovery** and **object storage for artifacts** are open (G99–G102, G109).
- **Studio** (the Monaco editor, the test panel) is feature 007; this delivers the API it will use.
- **Exactly-once.** A script step runs at least once; a lease lost mid-run runs it again elsewhere. A script that makes an external effect must put `octopus.idempotencyKey` (Python: `octopus.idempotency_key`) in the request: it is the same on every attempt of the step.
