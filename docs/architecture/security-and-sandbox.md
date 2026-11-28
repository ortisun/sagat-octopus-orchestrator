# Security, isolation and script sandbox

## Trust boundaries

Studio/API browser → API auth gateway → privileged engine → queue/task descriptors → untrusted user code (sandbox) → egress broker → external systems.

Release 0.3.0 adds four boundaries (constitution 5.0.0, principle IV and Delivery Constraints); none of them is built yet:

- **Workflow code in tenant workers** (design 044). The function runs only in the tenant's SDK worker, never in a platform process. Each command it proposes is checked against the version's envelope, the current authorization and the quotas. A workflow task carries no secret. The history it receives is that run's own journal projection, sent only to an authorized session and under the classification rules. What a workflow lease discloses and what a workflow-type binding grants are in [Workflow as code: what a workflow lease discloses](#workflow-as-code-what-a-workflow-lease-discloses-design-044).
- **Internal apps and their component frames** (feature 027). A session carries only the intersection of the viewer's authority and the app's declared operations. Author code runs in an opaque-origin frame that holds no credential.
- **The hub's indexes** (feature 028, design 047). The server fetches and verifies the release-signed index and operator-pinned indexes; every item is pinned by digest and imported as a draft. The server executes nothing from them.
- **The region epoch** (feature 032). Leases, fences and writes are checked against the active region's epoch in the database. A failover that can lose a revocation keeps the revoked access refused until an operator reviews it.

- Tenant and namespace scope authorization, persistence, messages and audit. Identifiers may appear in authorized logs/spans; metrics use bounded labels and do not carry tenant/namespace IDs. Constitution V scopes tenant diagnosis to authorized records/logs/spans and excludes tenant/namespace identifiers from metric labels, consistently with Constitution VI. Namespace also defines environment and RBAC.
- OIDC/JWT tokens with JWKS verification; machine auth via OAuth2 client credentials, optional mTLS.
- Built-in role names come from [Role](../../crates/domain/src/identity.rs): `admin`, `developer`, `operator`, `viewer`, `worker`, `runner` and `database-runner`. Human and machine roles have different permissions; PII reveal uses a separate namespace grant, not a custom role.
- Secret values are referenced by name and resolved under capability. Current implementations include the sealed application vault and optional OpenBao integration; the generic AWS Secrets Manager/Vault design target is not evidence of adapters for those services. See [secret provider](../../crates/infra-secrets/src/openbao.rs) and [deployment selection](../../infra/helm/values.yaml).
- Immutable audit of publish, trigger, webhook config, secrets access, rerun, cancel, manual override.

## Sandbox

- Compile code under build isolation; `source_hash + lockfile + toolchain_digest + policies` produces an artifact identified by digest.
- Pinned/allowlisted dependencies, size limits, SBOM and vulnerability verification, package origin and signatures where supported.
- Run in an isolated process with namespaces/seccomp/cgroups on Linux, or an isolated container environment; **no language/compiler is a sandbox by itself**.
- Per-tenant policy: CPU, memory, time, temporary filesystem, processes, concurrent builds, network and egress domain allowlist. No access to the Docker socket, host filesystem or cloud metadata.
- The runner has no scheduler credentials or database access. Short-lived capability tokens per activity and scoped secret.
- Segregate pools by network permissions, languages and trust level. Long builds in their own queues so they do not block the execution plane.
- Retention of stdout/stderr is limited, with redaction, and never include secrets in errors or debug output.

## Inline C#

> **Implemented paths:** TypeScript, Python, C#, Go and Java have builder/runtime selection in the sandbox adapter. C# uses separate build/runtime images; Go additionally requires an available configured build target. `Language::is_built` includes all five. The design bullets below do not establish that every supply-chain control is delivered; use the actual runner configuration and CI for its supported toolchain. This static inventory is not a fresh sandbox test result.

- .NET/Roslyn compilation in a **build container** with NuGet lockfile and package allowlist; immutable assembly and metadata, executed by a sandboxed .NET host.
- Do not offer `Assembly.Load` in the privileged process nor treat `AssemblyLoadContext` as a security barrier.
- Cancellation and timeout via process/container supervision, not just cooperative `CancellationToken`.

## Webhook security

HMAC SHA-256 per endpoint secret (when available), `timestamp` with window, nonce/event_id dedupe, mandatory TLS, rate limiting and optional allowlist. Ingress never trusts tenant/namespace sent in the payload; it resolves them from the authenticated endpoint.

## Release-blocking criteria

Cross-tenant read/write, sandbox escape, unauthorized script execution, secret exposure, quota bypass, incorrect webhook signature, replay of financial requests → critical severity.

## What exists today (feature 009)

The sections above include security targets. The inventory below describes paths present in the code; it is not a fresh runtime or release certification.

| Target | State |
|---|---|
| Tenant isolation and telemetry attribution | Tenant/namespace checks and composite persistence boundaries protect access; resource reads enforce isolation. Trace/log context can identify the tenant, but [metric labels](../../crates/telemetry/src/metrics.rs) exclude high-cardinality identifiers. This is not a claim that every table or metric carries tenant_id. Constitution V and VI now use the same metric-cardinality boundary. |
| OIDC/JWT, machine auth, optional mTLS | OIDC built (001). The worker API serves TLS and mutual TLS itself (009, D49); tying a client certificate to a token subject is open (G44) |
| Minimum RBAC, extra permissions to reveal payload | The role enum includes the six roles used by feature 016 plus feature 017’s dedicated `database-runner` machine role. Resource controls include worker bindings, secret access lists and namespace PII-reveal grants. Custom roles are not implied by reveal grants. The feature 016 text saying six roles is its feature-scoped baseline, not the complete current enum. |
| Secrets: references, rotated in a store, resolved only at execution, authorized by capability, never in the journal | Built with an application-level store sealed by a rotatable key ring (009, D45-D47); the lease is the capability; credential values must not reach the journal, logs, audit records or ordinary management responses; `ResolveSecrets` intentionally returns values to the authorized current executor under lease, binding and secret-access checks. Optional OpenBao provider integration is implemented and deployed with an OpenBao Agent sidecar; bootstrap/unseal and provider custody remain operator responsibilities. This does not imply generic KMS/Vault compatibility |
| Immutable audit of publish, trigger, webhook config, secrets access, rerun, cancel, manual override | Publish, start, cancel, pause, resume, signal (001, 002), secret, quota, binding and egress changes and refusals (009), and endpoint, subscription and connector changes, publishing of events, refusals at the webhook ingress and the platform's own switch-offs of a subscription (004) are audited; `RERUN_AS_NEW` has an implemented route and application path; it starts a new run without editing the original journal |
| Quotas: CPU, memory, time, processes, concurrent builds, network and egress allowlist | Concurrent runs, workflows, worker sessions and secrets are exact hard limits (009, D42), and a tenant's administrators can set them only up to a ceiling that the platform grants and only an operator raises (D52); the worker API also bounds how long a peer may stay silent, how long a connection lives and how many one address may hold (D55); the egress allowlist is stored and enforced by a guarded client (webhooks, connectors, and now scripts: a script reaches only what its policy and the namespace's egress policy allow, through its supervisor); CPU, memory, time, processes, output and scratch space are the policy of each script, clamped by a ceiling of the platform and of the runner, and enforced by the kernel through the container (005, D95, D108); `scripts` and `script_jobs` (builds and test runs in progress) are quotas |
| Sandbox isolation, build isolation, pools by trust level, short-lived capability tokens per activity | Builder/runtime paths exist for TypeScript, Python, C#, Go and Java (Go requires an available target); the isolation design uses a container per build and per run — no network, read-only root, no capabilities, `no-new-privileges`, seccomp, unprivileged user, memory/CPU/process/file/time limits — started by a runner that holds the container engine and no database; the code and its input go in on standard input, the result and the log come out on standard output and error, and everything else a script asks for (a request, a secret) is the supervisor's decision. The runner receives only the secrets its lease allows and reaches only what the egress policy names; artifacts are signed by the control plane and verified before anything runs. G96 no longer describes C#/Go/Java as absent. Remaining proposed controls include dependency installation and a supply-chain policy (G97), a sandboxed runtime in the repository (G98), pools shared between tenants (G99), pools by trust level |
| Webhook HMAC, timestamp window, nonce, TLS, rate limit | Feature 004 supports exclusive HMAC, deployment-approved OIDC and native mTLS profiles (ADR-0026). Identity failures are uniform 401, key-provider outages retryable; verified certificate identity never comes from forwarded headers. Every profile uses the timestamp/event-ID replay window, durable inbox and refusal audit. Native TLS revalidates certificates on keep-alive and bounds handshake/connection lifetime. Endpoint attempt throttling remains a local abuse guard, not a shared namespace rate quota. |

The release-blocking criteria below are tested for what exists: cross-tenant read/write, secret exposure
and quota bypass are in the battery and fail the build; sandbox escape, unauthorized script execution,
incorrect webhook signature and replay of financial requests join it with the features that create them.

## Disclosure boundaries

Credential delivery to a leased executor is distinct from payload disclosure to a reader.
The worker protocol explicitly returns credentials through `ResolveSecrets` (and the dedicated
managed-database capability returns its selected credential). These are privileged disclosures,
not public management reads. Constitution V bounds ordinary delivery to the immutable task's
declared secret set, all or nothing, with current worker binding, pool-delivery and publisher-access
checks. The managed database capability delivers only its pinned connection credential to the
authorized lease-owning database-runner. Ordinary delivery is token-capability plus current caller
authority, not an additional original-worker-session ownership check. Since constitution 5.1.0 a third declared set
exists (features 021 and 047, not built): a declared resource may disclose to the executor of a step that declares it
the non-secret settings of its pinned revision and, for an OAuth credential, only its current access token, never a
refresh token, a client secret or a private or signing key. That access token is a secret value (it never reaches the
journal, logs, audit or telemetry), and a token with no expiry goes only to a managed integration runner. Any other
secret a slot binds (an API key, a password) reaches an executor only as a declared secret of the first set, bound like
managed delivery when the executor is a managed integration runner. A managed integration runner is bound like the
database-runner, an ordinary worker or sandbox runner like ordinary delivery, and a workflow task of design 044 is never
such an executor. Every path revalidates identity/lease and selected
input/revision after provider I/O and commits a value-free audit before returning values. These
disclosure exceptions authorize neither database execution nor general/classified-secret reads. See the
[amendment impact record](../governance/constitution-3.0.0-sync-impact.md) and the
[5.1.0 record](../governance/constitution-5.1.0-sync-impact.md).

Feature 016 adds audited namespace grants for PII disclosure. Its owner-approved event-inbox and
dead-letter exception allows authorized whole raw values without per-field classification carriers;
it does not authorize disclosure of fields classified `secret` in classified payloads. Raw values
without an event schema are not proof that every embedded credential can be detected or removed.
See [owner decisions](../owner-decisions.md), [classification specification](../../specs/016-payload-classification/spec.md)
and [event payload handling](../../crates/application/src/event_payloads.rs).

### Workflow as code: what a workflow lease discloses (design 044)

Approved in [design 044](../designs/044-workflow-as-code.md) and being built (the tables by the migration of Task 044-6,
`run_initiators` among them, the server lane by Tasks 044-8, 044-13 and 044-24, the security tests by Task 044-24);
"Technical Plan" followed by a number is a section of that design.

- **No workflow code runs in a platform process.** The function runs only in the tenant's SDK worker: never in the
  control plane, the scheduler, the engine or the API, replay checks and queries included (Non-functional requirement
  044-007, constitution principle V). The determinism runtimes of the SDKs isolate for determinism, not for security:
  a TypeScript `vm` context is not a security boundary (Gap 044-008), and the SDK's determinism guard is not a security
  control.
- **A workflow task holds no secret** and no capability to resolve one: `ResolveSecrets`, `ResolveDatabase` and every
  other activity RPC refuse its lease token. An envelope's secrets are released only to the activities whose exact
  entry lists them, at the activity's own lease, with the publisher's right checked again at every resolution.
- **The seed is not for secrets.** Each code run has a 32-byte seed from which `ctx.random`, `ctx.randomInt` and
  `ctx.uuid` are drawn. Its values are predictable to every holder of the seed (the workers bound to the type and the
  holders of `history:export`); no REST read but the history export returns it. The SDKs say so, and the analyzers of
  Go and C# flag a random value or identifier used where a token, key or secret is named (Technical Plan 1.8).
- **The disclosure of a workflow lease.** A workflow lease receives the run's whole history **unredacted**, because the
  function needs its data: a binding that lists a workflow type grants that read for every run of the type, whoever
  started it. The one exception is a field classified `secret`, which reaches a workflow lease, and the read token of a
  worker-query or replay-check item, only as the export's typed placeholder, never its value; a blob whose carrier
  schema classifies any field `secret` is never delivered to them (its payload is `EXTERNAL_CONTENT_NOT_INSPECTED`,
  `ReadBlob` refuses it and no command may name it). A history carries no command body and no actor subject. Each
  lease's `ReadHistory` is audited once, value-free, as are the history reads of query and replay-check tasks, and the
  binding screen and API state the disclosure (Technical Plan 7.4, last paragraph). The binding and the tenant
  membership are checked again on every page.
- **The history export is narrower.** `GET /runs/{runId}/history` needs `run:read` and `history:export`, which only the
  admin role holds by default; every payload goes through the read path of feature 016 (classified by its carrier
  schema, or as `pii` when nothing declares it, and redacted unless the reader holds the reveal rights), a field
  classified `secret` is never exported, and every export is audited (Requirement 044-025).
- **A workflow-type binding grants authorship.** The worker of a bound type writes the commands of every run of the
  type, so setting a binding's `workflowTypes` needs `definition:publish` as well as `worker:bind`; workflow types are
  exact names (an activity pattern, `*` or prefix, never covers one), only an ordinary binding of an identity with the
  `worker` role may list them, and a binding records its binder (`bound_by`). The claim of a workflow task writes that
  binder into the run's initiators (`run_initiators`, created by the migration of Task 044-6), so an approval policy
  with `excludeInitiator` refuses the binder of the issuing session's binding, and approvers are shown the issuing
  identity and its binder (Technical Plan 4.1 and 5.1).
- **What `excludeInitiator` cannot see.** A person who holds a worker's credential without being its binder can make
  that worker issue commands, and no record names that person, so `excludeInitiator` cannot exclude them from approving
  what the worker issued. Keep a workflow worker's credential with its binder, or treat every holder of it as an
  initiator when assigning approvers.

## Feature 005: what a script can and cannot do

| A script can | A script cannot |
|---|---|
| Compute, read its input and the outputs of earlier steps, write a bounded log, write to a small non-executable scratch area | Open a socket: the container has no network interface but loopback |
| Ask its supervisor for an HTTP request, if its policy says `namespace` and the namespace's egress policy allows the host | Reach loopback, link-local, private, metadata, multicast or reserved space, even if a policy lists it |
| Ask for the secrets its step declared, by name | Read any other secret, any secret in a test run, or a value the platform did not hand it |
| Use up to its memory, CPU, processes and time | Exceed them: the kernel ends it, and the step fails with a code |
| Write a result up to 240 KiB | Forge another tenant's result, read the host's files, the engine's socket, the runner's environment, or run as root |
| Run again after a lost lease (at least once) | Make an external effect exactly once: it is given a key that is the same on every attempt |
| Ask its supervisor for things and be refused, any number of times | Hold the supervisor: every write to it is bounded by the run's deadline, the channel is cut after 400 lines, a hundred asks are served and the rest refused, and a run is given its wall clock plus a minute whatever the engine does |
| Choose the words of a refusal (a secret name, a host) | Make them long, carry a credential in them, or grow the audit trail: 200 characters, 32 a run, 100 different ones a script, and the address is `scheme://host` |
| Set headers on an allowed request | Set `Host`, framing or connection headers: routing is by the host of the URL |

What the control plane does with what a runner says: it signs a build only if the bundle says what its version says (language, entry point, file names, the policy it was clamped to, its schemas) under the key it handed out at the claim. It cannot compare the built code with the source, so a stolen runner token can get arbitrary code signed for **its own tenant** (G118); it cannot get anything signed for another, change a policy, or read a secret.

What stops it is the container, in the order in which a request for each of these meets a wall: the network namespace, the read-only mounts, the dropped capabilities and seccomp profile, the cgroup, the supervisor. A kernel or runtime vulnerability is an escape (G98); the mitigation is the runtime of the host, and that the runner is a host that holds nothing else.
