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
| Immutable audit of publish, trigger, webhook config, secrets access, rerun, cancel, manual override | Publish, start, cancel, pause, resume, signal (001, 002) and secret, quota, binding and egress changes and refusals (009) are audited; webhooks and rerun do not exist yet |
| Quotas: CPU, memory, time, processes, concurrent builds, network and egress allowlist | Concurrent runs, workflows, worker sessions and secrets are exact hard limits (009, D42), and a tenant's administrators can set them only up to a ceiling that the platform grants and only an operator raises (D52); the worker API also bounds how long a peer may stay silent, how long a connection lives and how many one address may hold (D55); the egress allowlist is stored and enforced by a guarded client that nothing consumes yet (G35, G36); CPU, memory, processes and builds belong to the sandbox (005) |
| Sandbox isolation, build isolation, pools by trust level, short-lived capability tokens per activity | Not built (feature 005, G35). The platform side that the runner will rely on exists: the runner would hold no scheduler credentials, receive only the secrets its lease allows, and reach only what the egress policy names |
| Webhook HMAC, timestamp window, nonce, TLS, rate limit | Not built (feature 004, G36) |

The release-blocking criteria below are tested for what exists: cross-tenant read/write, secret exposure
and quota bypass are in the battery and fail the build; sandbox escape, unauthorized script execution,
incorrect webhook signature and replay of financial requests join it with the features that create them.
