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
