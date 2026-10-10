# Security, isolation and script sandbox

## Trust boundaries

Studio/API browser → API auth gateway → privileged engine → queue/task descriptors → untrusted user code (sandbox) → egress broker → external systems.

- Tenant is a mandatory dimension in credentials, SQL, messages, metrics and audit log. Namespace defines environment and RBAC.
- OIDC/JWT tokens with JWKS verification; machine auth via OAuth2 client credentials, optional mTLS.
- Minimum RBAC: `viewer`, `author`, `publisher`, `operator`, `security_admin`, `tenant_admin`; additional permission to reprocess, compensate and reveal payload.
- Secrets: references, rotated in a secret store (AWS Secrets Manager/Vault), resolved only at execution and authorized by capability; forbid secrets in the journal.
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

> **Not built** (feature 005, G96). The design below is the target; C# is refused with `LANGUAGE_NOT_BUILT` and nothing in the sandbox's path depends on it.

- .NET/Roslyn compilation in a **build container** with NuGet lockfile and package allowlist; immutable assembly and metadata, executed by a sandboxed .NET host.
- Do not offer `Assembly.Load` in the privileged process nor treat `AssemblyLoadContext` as a security barrier.
- Cancellation and timeout via process/container supervision, not just cooperative `CancellationToken`.

## Webhook security

HMAC SHA-256 per endpoint secret (when available), `timestamp` with window, nonce/event_id dedupe, mandatory TLS, rate limiting and optional allowlist. Ingress never trusts tenant/namespace sent in the payload; it resolves them from the authenticated endpoint.

## Release-blocking criteria

Cross-tenant read/write, sandbox escape, unauthorized script execution, secret exposure, quota bypass, incorrect webhook signature, replay of financial requests → critical severity.

## What exists today (feature 009)

The sections above are the target. What is built, and tested, is the part of it that does not need
untrusted code to run yet:

| Target | State |
|---|---|
| Tenant is a mandatory dimension in credentials, SQL, messages, metrics and audit | Built (features 001, 002). Every table carries `tenant_id` and every foreign key is composite; another tenant's resource is a `404` indistinguishable from a missing one, and the probe is audited. The cross-tenant battery attacks every implemented REST operation, driven by the OpenAPI document |
| OIDC/JWT, machine auth, optional mTLS | OIDC built (001). The worker API serves TLS and mutual TLS itself (009, D49); tying a client certificate to a token subject is open (G44) |
| Minimum RBAC, extra permissions to reveal payload | Five built-in roles plus resource-level control: worker bindings, a secret's access list (009). Custom roles and `reveal payload` are not built (G39) |
| Secrets: references, rotated in a store, resolved only at execution, authorized by capability, never in the journal | Built with an application-level store sealed by a rotatable key ring (009, D45-D47); the lease is the capability; no value reaches the journal, a log, an audit record or an API response. A KMS/Vault envelope is open (G43) |
| Immutable audit of publish, trigger, webhook config, secrets access, rerun, cancel, manual override | Publish, start, cancel, pause, resume, signal (001, 002), secret, quota, binding and egress changes and refusals (009), and endpoint, subscription and connector changes, publishing of events, refusals at the webhook ingress and the platform's own switch-offs of a subscription (004) are audited; rerun does not exist yet |
| Quotas: CPU, memory, time, processes, concurrent builds, network and egress allowlist | Concurrent runs, workflows, worker sessions and secrets are exact hard limits (009, D42), and a tenant's administrators can set them only up to a ceiling that the platform grants and only an operator raises (D52); the worker API also bounds how long a peer may stay silent, how long a connection lives and how many one address may hold (D55); the egress allowlist is stored and enforced by a guarded client (webhooks, connectors, and now scripts: a script reaches only what its policy and the namespace's egress policy allow, through its supervisor); CPU, memory, time, processes, output and scratch space are the policy of each script, clamped by a ceiling of the platform and of the runner, and enforced by the kernel through the container (005, D95, D108); `scripts` and `script_jobs` (builds and test runs in progress) are quotas |
| Sandbox isolation, build isolation, pools by trust level, short-lived capability tokens per activity | Built for TypeScript and Python (feature 005, ADR-0013): a container per build and per run — no network, read-only root, no capabilities, `no-new-privileges`, seccomp, unprivileged user, memory/CPU/process/file/time limits — started by a runner that holds the container engine and no database; the code and its input go in on standard input, the result and the log come out on standard output and error, and everything else a script asks for (a request, a secret) is the supervisor's decision. The runner receives only the secrets its lease allows and reaches only what the egress policy names; artifacts are signed by the control plane and verified before anything runs. Not built: C#/Go/Java (G96), dependency installation and a supply-chain policy (G97), a sandboxed runtime in the repository (G98), pools shared between tenants (G99), pools by trust level |
| Webhook HMAC, timestamp window, nonce, TLS, rate limit | Built (feature 004, ADR-0012, `docs/runbooks/events.md`): HMAC-SHA256 over the label, timestamp, event id, type, subject, correlation key and body; the timestamp must be within the endpoint's skew; the event id is the nonce and is remembered for twice the skew and a minute; one uniform `401`; the cost of an unauthenticated request is bounded before a secret is read; a per-endpoint rate in two stages. TLS is the deployment's, as for the rest of the API. The rate limiter is in memory per replica and there is no per-address limit (G78); OIDC and mTLS profiles are not built (G75) |

The release-blocking criteria below are tested for what exists: cross-tenant read/write, secret exposure
and quota bypass are in the battery and fail the build; sandbox escape, unauthorized script execution,
incorrect webhook signature and replay of financial requests join it with the features that create them.

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
