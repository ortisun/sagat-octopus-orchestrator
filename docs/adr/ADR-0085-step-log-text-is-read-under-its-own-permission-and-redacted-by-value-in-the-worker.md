# ADR-0085: Step-log text is read under its own permission and redacted by value in the worker, by shape on the server

**Status:** Proposed (2026-11-13). Accepting it is the owner's, separately from the answers of OD-2026-11-13-14. It states a limit
on principle V of the constitution, which makes its acceptance a security decision.
**Decided by:** Agent proposal for feature 041. The owner chose, among the options of the design, that capture is on by default
(O041-001), who may read (O041-002), that reads are audited once per window (O041-003) and that the limit of redaction is accepted as
stated (O041-011), on 2026-11-13 ("Aceitar todas", OD-2026-11-13-14). Where redaction happens and what the server may not do are the
agent's proposal.
**Refines / supersedes:** none. It follows [ADR-0033](ADR-0033-payload-values-are-classified-per-field.md) in keeping what may hold
personal data away from `run:read`, and does not extend field classification to logs: a log line has no schema.
**Enforced by:** nothing yet: the feature is not built. The checks that MUST exist when it is, each a task of
[design 041](../designs/041-step-logs.md): the conformance scenario `step-logs-redaction` with `mustNotLeak` in six SDKs (T041-14
to T041-21, rule 1); server tests of each shape rule and sensitive field name (T041-6, rule 2); a canary searched for in the stored
rows, the three routes, `run_events`, telemetry, `audit_records` and the captured process output (T041-8 and T041-27, rules 1 to
4); `role_permission_matrix_is_exactly_as_specified` with the new permission, and a viewer refused on the three routes (T041-4,
T041-11, rule 5); the audit test "no audit, no lines" (T041-11, rule 6); the cross-tenant battery on the RPC and the routes
(T041-8, T041-11, rule 7). Rule 3 is review only: no test can show that a code path nobody wrote does not resolve secrets.

## Context

Principle V says secret values MUST NOT reach workflow state, the journal, logs, audit records, telemetry or ordinary read API
responses. Feature 041 stores text that customer code writes, and customer code holds secrets.

- A worker receives the values of its attempt's declared secrets through `ResolveSecrets`, once, under its lease. The server does
  not keep them afterwards, so it cannot recognise one inside a line.
- The script sandbox already redacts by value: `sagat_domain::scripts::redact` replaces the values handed to a script (at least 6
  bytes, longest first) with `[REDACTED]`, and `redact_credentials` replaces credential shapes (PEM private keys, `AKIA`/`ASIA`
  keys, `Bearer` tokens). `sagat_telemetry::redact::is_sensitive_name` lists field names that mark a value as sensitive.
- None of the six SDKs redacts what an activity logs today; `sdks/SPEC.md` §10 only requires that the SDK's own logs hold no secret.
- A log line is free text. The annotations of feature 016 classify payload schemas and, by its runbook, do not classify logs. A line
  can hold personal data, and `run:read` is held by viewers.

What cannot be done by any design: recognise a secret that the code transformed before logging it, or one it obtained outside the
platform.

## Decision

1. The SDK MUST replace, before a line leaves the worker process, every secret value the attempt received through `ResolveSecrets`,
   in the message and in every string field value, by the rule of `sagat_domain::scripts::redact`. The rule is part of
   `sdks/SPEC.md` and of the conformance suite, the same in the six SDKs.
2. The server MUST apply the existing shape rules to every stored message and string field value, and MUST replace the value of a
   field whose key the existing sensitive-name list matches. It reuses those functions; it MUST NOT grow a second list.
3. The server MUST NOT resolve or read a secret in order to redact a log.
4. The text of a line MUST NOT appear in the journal, a projection, telemetry, an audit record, a metric, a span, an error body or
   the platform's own logs. The audit record of a read holds no line and not the search text.
5. Step logs MUST be read only with `run:logs:read`, a permission of its own, held by the admin, developer and operator roles and
   not by viewer, worker or runner. `run:read` MUST NOT disclose a line, nor whether a log exists or how large it is.
6. A read MUST be audited as `run.logs_read`, at most once per subject and run in 300 seconds; the record of a window commits
   before its first line is returned, and a failure to write it returns no line.
7. Every key and predicate of the log tables carries the tenant. Another tenant's run is answered as missing and the attempt is
   audited, on the RPC and on the routes.
8. Capture is on by default at level `info`, and a namespace MUST be able to turn it off; the release notes MUST state the default
   and the limit below.
9. **The limit, stated and not hidden:** a secret that the step's code transforms before logging it (encoded, split, shorter than 6
   bytes), or that it obtained outside `ResolveSecrets`, is stored as written; and a client that speaks protocol 1.7 without an
   official SDK can skip rule 1 and store what it was given. The platform MUST NOT claim otherwise in its documentation, and the
   runbook MUST say how a stream's lines are deleted and that the secret is then rotated.

## Considered options

1. **Redaction by value in the worker, by shape on the server, and a permission of its own** — chosen. Each side redacts what it can
   know; the reader's permission bounds what the limit of rule 9 can expose.
2. **Redaction by value on the server, by resolving the attempt's secrets for each batch** — lost. A secret read per batch on a path
   that must stay cheap, each read a disclosure that principle V requires to be authorized and audited.
3. **The server keeps the values it handed to an attempt, or their digests, to match against lines** — lost. Keeping the values is
   the exposure the principle forbids; a digest matches only a whole token, not a value inside a message.
4. **No redaction; rely on the permission** — lost. A value the platform itself handed out would be stored by the platform.
5. **Reading under `run:read`** — lost (O041-002). Viewers hold it.
6. **Reading only under a per-namespace grant, as `payload:reveal-pii`** — lost (O041-002). Nobody could read a log until someone
   granted it.
7. **Capture off unless a namespace asks** — lost (O041-001). It removes the surprise of rule 8 and the feature with it; the switch
   stays for the namespace that needs it.
8. **An audit record per request, or none** — lost (O041-003). One per page of a tail, or no evidence of who read.

## Consequences

Good: a secret the platform delivered is not stored by any official SDK; a credential-shaped string is caught even from a client
that does not redact; who read a run's log is on record; viewers and other tenants learn nothing.

Bad: principle V is met with a condition, recorded in the design's Constitution Check and Complexity Tracking. After an SDK
upgrade, text a customer wrote for their own machines is sent to the platform unless they turn capture off. A secret shorter than 6
bytes is not redacted, as in the script sandbox. Redaction by value can cut a word that happens to equal a secret.

What becomes harder: the redaction rule now lives in six languages and must stay identical; a change of the rule is a change of
`sdks/SPEC.md`, of the scenario's golden and of six SDKs. `Permission::ALL` grows to 54 and the role matrix test changes with it.

Compatibility: additive (a permission, an audit action, three operations). No existing token gains or loses access to anything
that exists today. Rollback: the capture switch per namespace, and `SAGAT_STEP_LOGS_ENABLED=false` for the deployment.

## Evidence

None measured yet; this ADR is proposed with the design and nothing is built. Read in the code at `a4f6e9db`:
`crates/domain/src/scripts.rs` (`redact`, `redact_credentials`, `MIN_REDACTED_LEN`), `crates/telemetry/src/redact.rs`,
`crates/domain/src/identity.rs` (53 permissions, the role lists), `crates/domain/src/audit.rs` (`payload.inspected`, one per 300
seconds), `docs/runbooks/payload-classification.md`. The design is [docs/designs/041-step-logs.md](../designs/041-step-logs.md);
decision D041-008 and gap G041-001 are in [specs/041-step-logs/research.md](../../specs/041-step-logs/research.md); the owner's
answers are OD-2026-11-13-14.
