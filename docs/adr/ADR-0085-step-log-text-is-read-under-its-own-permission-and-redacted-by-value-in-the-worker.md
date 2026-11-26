# ADR-0085: Step-log text is read under its own permission and redacted by value in the worker, by shape on the server

**Status:** Accepted (2026-11-13, OD-2026-11-13-17: the owner's "Aceito tudo"); revised the same day after review, before any code (see Revision). It states a limit on principle V of the constitution.
**Refined by:** feature 031 (Requirement 031-038, approved draft, 2026-11-26): the set of rule 1 also holds the decoded values of the planned locations of every attempt in flight, so a decoded value never reaches a step log.
**Decided by:** owner, on the agent's proposal (OD-2026-11-13-17, accepting the text of commit 35b752b3). The owner chose, among the
options of the design, that capture is on by default (O041-001), who may read (O041-002), that reads are audited once per window
(O041-003) and that the limit of redaction is accepted as stated (O041-011) (OD-2026-11-13-14), and, after review, that classified
namespaces also need the PII reveal grant and that a script execution's log is read under the same rule (O041-012, O041-013,
OD-2026-11-13-18). The rest of the revision of 2026-11-13 is the agent's, answering the architecture, security and correctness
reviews of commit 2e90dbb8, and is put to the owner with the design (T041-2).
**Refines / supersedes:** none. It follows [ADR-0033](ADR-0033-payload-values-are-classified-per-field.md) in keeping what may hold
personal data away from `run:read` and reuses its PII reveal grant; it does not extend field classification to logs: a log line has
no schema.
**Enforced by:** nothing yet: the feature is not built. The checks that MUST exist when it is, each a task of
[design 041](../designs/041-step-logs.md): the conformance scenario `step-logs-redaction` with two concurrent attempts and
`mustNotLeak` in six SDKs (T041-14 to T041-21; rules 1 and 2); server tests of each shape of both scrubbers and each sensitive field
name (T041-6; rule 3); a canary searched for in the stored rows, the routes, `run_events`, telemetry, `audit_records` and the captured
process output (T041-8, T041-27; rules 1 to 5); `role_permission_matrix_is_exactly_as_specified` with the new permissions, a viewer
refused on the read routes and given a script execution without its log, a developer without the grant refused in a classified
namespace (T041-4, T041-11, T041-28, T041-29; rules 6 and 7); the audit tests (T041-11, T041-30; rules 8 and 9); the cross-tenant
battery on the RPC and the routes (T041-8, T041-11; rule 10). Rule 4 is review only: no test can show that a code path nobody wrote
does not resolve secrets.

## Context

Principle V says secret values MUST NOT reach workflow state, the journal, logs, audit records, telemetry or ordinary read API
responses. Feature 041 stores text that customer code writes, and customer code holds secrets.

- A worker receives the values of its attempt's declared secrets through `ResolveSecrets`, once, under its lease. The server does
  not keep them afterwards, so it cannot recognise one inside a line. A worker process runs several attempts at once, each with its
  own secrets.
- Some SDK loggers are shared: Python's activity logger `_TaskLog` adapts the worker-wide logger, so anything attached to that logger
  sees every attempt's lines.
- Two redaction mechanisms exist. `sagat_domain::scripts::redact` replaces the values handed to a script (at least 6 bytes, longest
  first) and, through `redact_credentials`, PEM private keys, `AKIA`/`ASIA` keys and `Bearer` tokens, with `[REDACTED]`.
  `sagat_telemetry::redact::scrub` hides URL user information, JWT-shaped words, `Basic` credentials and `sk-`, `ghp_` and `xox`
  tokens, with `[redacted]`, and cuts a value at 2,048 characters; `is_sensitive_name` lists field names that mark a value sensitive.
- None of the six SDKs redacts what an activity logs today.
- A log line is free text that can hold personal data. `run:read` is held by viewers. The `log` of a script execution is served
  today under `script:read`, which viewers also hold.

What cannot be done by any design: recognise a secret that the code transformed before logging it, or one it obtained outside the
platform.

## Decision

1. The SDK MUST replace, before a line leaves the worker process, every secret value held by **any attempt in flight in that
   process**, in the message, in every field key, in every string field value, in the decimal text of every number field, and in
   the exception text and stack it formats, by the rule of `sagat_domain::scripts::redact`. Context the language attaches to a line
   is captured only as fields and redacted as fields.
2. Capture MUST happen in the per-attempt logger object; an SDK MUST NOT attach a handler, sink or appender to a logger that other
   attempts share.
3. The server MUST apply, in the application layer and before any size is measured, the shapes of both existing scrubbers with one
   placeholder, `[REDACTED]`, and without the 2,048-character cut, and MUST replace the value of a field whose key the existing
   sensitive-name list matches. It reuses those functions; it MUST NOT grow a second list.
4. The server MUST NOT resolve or read a secret in order to redact a log.
5. The text of a line MUST NOT appear in the journal, a projection, telemetry, an audit record, a metric, a span, an error body or
   the platform's own logs, and the batch request MUST NOT be printed by any log, `Debug` or trace. The audit record of a read holds
   no line and not the search text.
6. Step logs MUST be read only with `run:read` and `run:logs:read`, a permission held by the admin, developer and operator roles and
   not by viewer, worker or runner. In a namespace in which a published version carries a classification annotation, the reader MUST
   also hold the namespace's PII reveal grant. Neither the read routes nor any other route MUST disclose a line, or whether a log
   exists or how large it is, to a caller without both.
7. The `log` of a script execution of a run MUST be returned only to a caller who may read that run's step logs under rule 6; any
   other caller gets the execution without it.
8. A read MUST be audited as `run.logs_read`, at most once per subject and run in 300 seconds whatever its filters; the record of a
   window commits before its first line is returned, and a failure to write it returns no line.
9. Deleting a run's step logs MUST need `run:logs:delete`, held by the admin role only, and MUST be audited as `run.logs_purged` in the
   same transaction.
10. Every key and predicate of the log tables carries the tenant. Another tenant's run is answered as missing and the attempt is
    audited, on the RPC and on the routes. A cursor or stream of another run MUST be refused.
11. Capture is on by default at level `info`, and a namespace MUST be able to turn it off; the release notes MUST state the default,
    the limit below and that viewers no longer read script logs.
12. **The limit, stated and not hidden:** a secret that the step's code transforms before logging it (encoded, split, shorter than 6
    bytes), or that it obtained outside `ResolveSecrets`, is stored as written; and a client that speaks protocol 1.7 without an
    official SDK can skip rule 1 and store what it was given. The platform MUST NOT claim otherwise in its documentation, and the
    runbook MUST say how a stream's lines are deleted and that the secret is then rotated.

## Considered options

1. **Redaction by value in the worker over the process's live secrets, by shape on the server, and a permission of its own** —
   chosen. Each side redacts what it can know; the reader's permission and, where classification is used, the PII grant bound what
   the limit of rule 12 can expose.
2. **Redaction by value on the server, by resolving the attempt's secrets for each batch** — lost. A secret read per batch on a path
   that must stay cheap, each read a disclosure to authorize and audit.
3. **The server keeps the values it handed to an attempt, or their digests** — lost. Keeping the values is the exposure the
   principle forbids; a digest matches only a whole token.
4. **Redaction by each attempt's own secrets only** — lost in review. One attempt can log a value it learned from another attempt of
   the same process, through shared state.
5. **Capture through a handler on the language's shared logger** — lost in review. It sees every attempt's lines.
6. **No redaction; rely on the permission** — lost. A value the platform itself handed out would be stored by the platform.
7. **Reading under `run:read`** — lost (O041-002). Viewers hold it.
8. **Reading only under a per-namespace grant, everywhere** — lost (O041-002); the grant applies only where classification is used
   (O041-012).
9. **Leaving the script log under `script:read`** — lost (O041-013). It contradicted rule 6.
10. **Capture off unless a namespace asks** — lost (O041-001).
11. **An audit record per request, or none** — lost (O041-003).

## Consequences

Good: a secret the platform delivered to any attempt of a process is not stored by an official SDK; a credential-shaped string is
caught even from a client that does not redact; who read and who deleted a run's log is on record; viewers, readers without the
grant in classified namespaces, and other tenants learn nothing.

Bad: principle V is met with a condition, recorded in the design's Constitution Check and Complexity Tracking. After an SDK upgrade,
text a customer wrote for their own machines is sent to the platform unless they turn capture off. A secret shorter than 6 bytes is
not redacted. Redaction by value can cut a word that equals a secret, including one of another attempt. Viewers lose the script
log they read today: a documented break of one field.

What becomes harder: the redaction rule lives in six languages and must stay identical; `Permission::ALL` grows by two; the
classified-namespace fact is recorded at publication and must be backfilled.

Compatibility: additive except for the script log, which viewers lose. Rollback: the capture switch per namespace,
`SAGAT_STEP_LOGS_ENABLED=false` for the deployment, `SAGAT_STEP_LOGS_READ_ENABLED=false` for the read routes.

## Revision (2026-11-13, after review, before any code)

Revision accepted 2026-11-15, OD-2026-11-15-4.

The owner accepted the text of commit 35b752b3. The architecture, security and correctness reviews of commit 2e90dbb8 found that a
Python capture handler would sit on the worker-wide logger and see every attempt, that redaction by an attempt's own secrets left
another attempt's values in view, that keys, numbers, exceptions and language context were not redacted, that the server used one
of the two existing scrubbers and measured before scrubbing, that the batch could be printed by a generated `Debug`, that there was
no deletion path for an incident, and that the claim of rule 6 was false while the script log stayed readable by viewers. The owner
then decided O041-012 and O041-013 (OD-2026-11-13-18). Decisions 1, 3, 5, 6, 10 and 11 were rewritten and decisions 2, 7 and 9
added; options 4, 5 and 9 were added. The chosen option is unchanged. The reports are in
`docs/designs/041-step-logs.reviews/2026-11-13/`.

## Revision (2026-11-15), accepted 2026-11-15 (OD-2026-11-15-4)

Revision accepted 2026-11-15, OD-2026-11-15-4.

Note (2026-11-15, OD-2026-11-15-7): O041-017 was answered (a), so the secret set of decision 1 lives for the process,
bounded, and the parenthesis about answer (b) in decision 1 and the matching clause of decision 12 do not apply.

The Status line above is unchanged; the decisions below replace those of the same numbers above. The egm-review and the second round of the
reviewers (`docs/designs/041-step-logs.reviews/2026-11-15/`) found that a secret leaves the process's set when its
attempt ends although a later attempt can log it through shared state; that each language's own decimal text of a number
does not contain a numeric secret; that the server's scrubbers cannot be reused as written (`scrub_shapes` and
`redact_credentials` are private, `scrub` cuts at 2,048 characters) and never hide a non-string value; that the withheld
script log still disclosed its size class; that the classified-namespace flag failed open during a rolling upgrade or after a rollback; and
that "purged" named both the delete and the purge. The revision replaces these decisions:

- **1.** The SDK MUST replace, before a line leaves the worker process, every value in the process's secret set, in the
  message, in every field key, in every string field value, in the canonical decimal text of every number field (the
  form of ECMAScript `Number::toString`, defined in `sdks/SPEC.md`; a number whose text holds a value becomes the string
  `[REDACTED]`), and in the exception text and stack it formats, by the rule of `sagat_domain::scripts::redact`. The set
  holds every value `ResolveSecrets` returned in the process for the life of the process, bounded; when it is full,
  capture MUST stop for the process (pending O041-017: if the owner answers (b), the set holds a value only while an
  attempt that holds it is in flight, and rule 12 names the shared-state case). Context the language attaches to a line
  is captured only as fields and redacted as fields.
- **3.** The server MUST apply, in the application layer and before any size is measured, the shapes of both existing
  scrubbers through public entry points that take the placeholder `[REDACTED]` and do not cut, and MUST replace with
  `[REDACTED]` the value, of any type, of a field whose key the existing sensitive-name list matches. It reuses those
  functions; it MUST NOT grow a second list.
- **6.** Step logs MUST be read only with `run:read` and `run:logs:read`, a permission held by the admin, developer and
  operator roles and not by viewer, worker or runner. In a namespace that is classified (a published version carries a
  classification annotation, recorded by a database trigger on every version insert into an insert-only table that
  nothing clears, with a test that may over-match and so errs closed) the reader MUST also hold the namespace's PII
  reveal grant. Neither the read routes nor any other route MUST disclose a line, or whether a run or a stream has a log
  or how large it is, to a caller without both; the tenant's total retained log bytes stay readable under `quota:read`
  as today.
- **7.** The `log` of a script execution of a run MUST be returned only to a caller who may read that run's step logs
  under rule 6; any other caller gets the execution with `log` empty, `logTruncated` false and `logWithheld` true, and a
  listing shows `logTruncated` false to them.
- **9.** Deleting a run's step logs MUST need `run:logs:delete`, held by the admin role only, and MUST be audited as
  `run.logs_deleted` in the same transaction; "purge" names only the later removal of the rows.
- **12.** **The limit, stated and not hidden:** a secret that the step's code transforms before logging it (encoded,
  split, shorter than 6 bytes), or that it obtained outside `ResolveSecrets`, is stored as written; a client that speaks
  protocol 1.7 without an official SDK can skip rule 1 and store what it was given; and, if O041-017 is answered (b), a
  value of an attempt that has ended is not redacted from a later attempt's lines. The platform MUST NOT claim otherwise
  in its documentation, and the runbook MUST say how a stream's lines are deleted and that the secret is then rotated.

"Enforced by" gains, when built: `step-logs-redaction` with an attempt that starts after another ended and with numbers
12345678 and 1e15 (rule 1, T041-14 to T041-21); tests that the public scrubber entry points equal what they replace and
that a sensitive field of each type is hidden (rule 3, T041-6); the trigger tests, an old binary's insert included (rule 6,
T041-28); the withheld `logTruncated` in the viewer test (rule 7, T041-29).

Note (2026-11-16, OD-2026-11-16-7 and OD-2026-11-16-8): reading step logs needs `run:logs:read` and the namespace's PII
reveal grant in every namespace ("Sim, sempre", OD-2026-11-16-8, which changes OD-2026-11-13-18): an unannotated payload
field is `pii` under ADR-0033 and feature 016, and a log line can hold payload values. So revised decision 6 reads "in
every namespace", and no classified-namespace record is kept: neither the trigger and table of the 2026-11-15 revision
nor a backfill nor the partial index that briefly replaced them. The Consequences' sentences on readers without the grant
"in classified namespaces" and on recording and backfilling the classified fact read "in every namespace" and no
longer apply, respectively. Rollback has one switch, `SAGAT_STEP_LOGS_ENABLED`; the read switch
`SAGAT_STEP_LOGS_READ_ENABLED` was removed (design 041, D041-069). In "Enforced by", the SDK scenarios search with
`mustNotStore` (stored lines, route bodies, captured output), not `mustNotLeak`; a unit test beside each generated
module's `Debug` impl checks that a printed `AppendStepLogsRequest` holds no message text (rule 5); and the tests of the
classified-namespace trigger are replaced by tests that a reader without the grant is refused in a namespace whose
schemas carry no annotation (design 041, T041-11). The owner accepted G041-017 in writing (OD-2026-11-16-7): because the
secret set lives for the process, a caller who can steer logged text can learn that a guessed value is a secret the
process resolved.

Note (2026-11-19, design 041 T041-3): rule 5's check on the server and the Rust SDK is built: the step-log messages
skip their generated `Debug` in `crates/api-grpc`, `sdks/rust` and `sdks/rust/conformance`, the replacements print
counts only, and `crates/api-grpc/tests/step_log_debug.rs` and a unit test beside each SDK module show that no message,
exception, field or token prints; the domain's `FieldValue` prints its variant only. The other five SDKs are covered by
their capture tasks (T041-15 to T041-20).

## Evidence

None measured yet; nothing is built. Read in the code at `2e90dbb8`: `crates/domain/src/scripts.rs` (`redact`,
`redact_credentials`, `MIN_REDACTED_LEN`), `crates/telemetry/src/redact.rs` (`scrub`, `is_credential_word`, `is_sensitive_name`,
`MAX_VALUE_CHARS`), `sdks/python/sagat_octopus_orchestrator/worker/host.py` (`_TaskLog`), `crates/domain/src/identity.rs` (the role
lists), `crates/domain/src/audit.rs`, `crates/infra-postgres/src/reveal_grants.rs` (`has_effective_payload_reveal_grant`),
`crates/api-http/src/routes/scripts.rs` and `dto_scripts.rs` (`get_execution`, `ExecutionBody.log`). The design is
[docs/designs/041-step-logs.md](../designs/041-step-logs.md); decisions D041-008, D041-014, D041-016, D041-025 and D041-026 and gap
G041-001 are in [specs/041-step-logs/research.md](../../specs/041-step-logs/research.md).
