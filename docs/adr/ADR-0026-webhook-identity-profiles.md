# ADR-0026: Authenticate webhook identities at the transport boundary

Status: accepted and implemented

## Context

FR-004-004 and G75 require OIDC and mutual TLS profiles in addition to the existing
HMAC envelope. All profiles must share the durable inbox, deduplication, bounded
cost, timestamp window, tenant ownership and refusal audit.

## Decision

An endpoint selects exactly one profile: HMAC with a namespace signing secret,
OIDC with a deployment-approved verifier name and an exact subject, or mutual TLS
with the SHA-256 fingerprint of a verified leaf certificate. The endpoint management
API cannot supply discovery/JWKS URLs, trust roots, certificate headers or a token
algorithm. Existing endpoints and omitted profiles remain HMAC.

OIDC verifier settings are deployment configuration: exact issuer, audience and
HTTPS JWKS URI, bounded cache/refresh and token lifetime. Verification reuses the
existing signature/claim checks. Webhook identities do not need a platform tenant
claim or membership; the persisted endpoint supplies the tenant and namespace.
Platform API token verification keeps its mandatory tenant claim.

Native HTTP TLS verifies client certificates against configured trust roots and
inserts the leaf fingerprint as an internal request extension. A missing certificate
is allowed at the handshake for routes that do not require it, but cannot authenticate
an mTLS endpoint. Forwarded certificate headers never establish identity. TLS
handshakes, connections, headers and stalled writes retain bounded deadlines.
The listener owns its connection tasks and cancels remaining handlers and sockets
after shutdown grace; returning from the listener never detaches them. Certificates
are revalidated on every keep-alive request; an expired identity follows ordinary
profile refusal and auditing. TLS session resumption is disabled.

All profiles require a valid event ID and timestamp within the configured skew.
The inbox deduplication window exceeds twice that skew plus the existing margin.
HMAC authenticates the envelope bytes; OIDC/mTLS authenticate the sender over TLS.
They do not claim a detached body signature. Exact duplicate admission returns 202;
reuse of an event ID with different content returns the existing typed conflict.
An invalid credential, profile or subject yields the same public 401. An unavailable
JWKS provider fails closed with a retryable server error. Credentials and certificates
are never written to the journal, audit, logs or error response.

## Validation required before acceptance

Real TLS handshakes with missing, untrusted and mismatching client certificates;
OIDC wrong subject/issuer/audience, expired tokens, rotating keys and unavailable
JWKS; spoofed forwarded headers; unchanged HMAC compatibility; replay and mutated
event IDs; tenant management boundaries, database rollback, typed errors and metrics.

## Verification

Independent review found an endpoint-change race after asynchronous identity verification.
Admission now locks and compares the complete trusted endpoint configuration in the inbox
transaction. Missing/changed identities receive 401; paused endpoints retain their 503 behavior.
Deterministic pause, identity replacement and deletion races pass (3/3), and the complete
PostgreSQL ingress suite passes 29/29. Other checkpoints: migration 5/5, HTTP contract 2/2,
platform authentication 27/27, real TLS/OIDC 8/8, control-plane configuration 32/32 and
HTTP connection-fault suite 9/9. Scoped Clippy and all workspace target compilation pass.
