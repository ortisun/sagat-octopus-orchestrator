# ADR-0074: Platform operators act through their own listener and principal, and outside data comes from allowlisted, read-only sources

**Status:** Proposed (2026-10-28); depends on the owner's answer to O1710
**Decided by:** agent proposal for the owner's review; the owner decided that the platform pulls outside data by configuration, operator-only (O1707), that tenants never define policies (O1703), and that the management interface is a REST contract and an operator CLI (O1708)
**Refines / supersedes:** none; it states how the rule of `docs/runbooks/security-governance.md` that limits are raised from a shell and not by an API call applies to scaling
**Enforced by:** `security_elastic_scaling` (a tenant token on the operator listener, a tenant declaring a source, SSRF, redirects, oversize, plain-text credentials, cross-tenant lag), `contract_elastic_scaling` (both OpenAPI documents, the configuration schema), the architecture check, which treats `infra-signals` as an adapter by its prefix

## Context

The REST API authenticates a bearer token to a tenant membership with one role. There is no platform-operator principal; today's operator actions (`quota-ceiling`, `scheduling-weight`) are `control-plane` subcommands run with the database owner role, because a tenant must not remove the limits that protect the others and the runtime role cannot write those tables. The specification asks for operator policies, sources and overrides to be manageable programmatically (FR-036-019, FR-036-023), and for the platform to query outside metric systems and queues (FR-036-056). The audit trail requires a tenant, and a platform-wide change has none.

The tenant egress policy refuses loopback and private addresses by design, which is right for a tenant's webhook and wrong for an in-cluster monitoring service.

## Decision

1. **A separate operator listener,** served only when `SAGAT_OPERATOR_ADDR` is set (off by default), with its own OpenAPI document. It verifies bearer tokens with the existing token verification but a separate audience (`sagat-operator`) and produces a `PlatformOperator` principal that only this listener's extractor can create. The tenant listener MUST NOT accept an operator token and the operator listener MUST NOT accept a tenant token. Permissions are `scaling:read`, `scaling:manage`, `scaling:override` and `scaling:publish`.
2. **Platform-scope changes are audited in `platform_audit`,** an append-only table written in the same transaction as the change, with actor, action, resource, reason, a digest of the before and after, and the trace id. Tenant-scope changes use the tenant audit.
3. **Ceilings, the destination allowlist, the database budget and the pool names are configuration only,** so a compromised operator API or runtime role cannot lift them. The network policy peers that may reach the operator listener are explicit in the chart, and an empty or global peer is refused at render.
4. **External sources are declared by operators only.** A tenant publishes nothing that makes the platform call out.
5. **A source's destination MUST be on the configuration allowlist** (exact host and port; `allowPrivate` only by name), the check is repeated at every query, and where the deployment restricts its network the destination is also allowed there. The guard implements the `infra-egress` `Guard` seam in a new `infra-signals` crate: an address literal is checked before resolving, every resolved address is checked by the connector's resolver, redirects are never followed, proxies from the environment are ignored.
6. **Queries are bounded and read-only:** the platform builds the request and the operator supplies only the query text or the queue name; a timeout (5 s), a response limit (64 KiB), a minimum interval, at most one query per source per interval however many replicas run, a backoff on failure, and a result that is not one finite number inside the declared range makes the signal unavailable (never clamped, never 0).
7. **Credentials are file names.** A declaration holds the name of a file under a directory the chart mounts from Kubernetes Secrets; the file is read at use and never stored, logged, returned or put in a record. The API cannot introduce a secret the deployment did not mount. A credential travels only over TLS.

8. **Operator tokens are short-lived and rate limited.** The operator listener MUST refuse a token whose lifetime exceeds one hour, record the subject and the token identifier on every action, and limit each token to 20 requests per second (configurable). Revocation is expiry and rotation of the verification keys of the operator audience.
9. **The evaluator re-validates every definition at each evaluation** against the configuration-only ceilings, budget, allowlist and pool names, and MUST refuse one that violates them and raise a condition, so that a row written outside the management interface by a compromised role cannot take a unit past them.
10. **"Operator" means the platform operator.** It is not the tenant role named Operator, which gains no power from this decision.

## Considered options

1. **Separate listener and principal** — a tenant listener can never create or accept an operator; the surface can be left off and reached only from explicit peers.
2. **An operator role on the tenant API** — one listener, but a cross-tenant principal on the surface tenants reach, a change to the identity pipeline of feature 009, and a misconfigured role grants platform power through the tenant door.
3. **Owner-role commands only, read-only REST** — the existing pattern and no new authority, but no programmatic write for an operator, which the specification and the owner's answer require.
4. **Reuse the tenant egress policy for sources** — it forbids private addresses, so an in-cluster monitoring system could not be reached, and loosening it would loosen it for tenants.
5. **Push only (operators publish custom signals)** — the smallest surface, and the owner chose pull (O1707); it remains available.

## Consequences

- Good: a clear authority line; sources cannot become an SSRF tool for tenants; secrets never enter the database.
- Bad: a second OpenAPI document and listener to secure and document; operators handle an audience in their identity provider; a compromised control plane can still write the scaling tables inside the configuration-only bounds (`G1707`).
- Harder: the first deployment needs the audience, the peers and the mounted secrets; the chart refuses a source whose destination is not allowed.
- Compatibility: additive; no tenant behaviour changes; the operator listener is absent unless enabled.

## Evidence

Specification FR-036-019 to FR-036-025 and FR-036-056 to FR-036-060; `research.md` D1713 to D1717, G1706, G1707, O1710; `docs/runbooks/security-governance.md` lines 99 to 101; `crates/infra-egress/src/lib.rs`; `crates/domain/src/audit.rs:512, 632`. To be added when built: the security test run.
