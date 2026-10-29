# Notification channels, messages and decision links (draft)

DRAFT, not implemented. Channels are destinations of the ADR-0025 outbound pipeline. Their messages are
fixed-shape, platform-defined deliveries, so the engine sends them through the guarded egress client
holding only that channel's own credential (the placement rule of feature 021, ADR-0055 rule (b));
channels never reuse feature-021 resources. This document
fixes what each destination receives, how a decision link is built and verified, and how the Slack
interaction endpoint (P3) authenticates. Sources consulted are listed in [research.md](../research.md).

## Destinations

| Kind | Settings (non-secret) | Secret binding | Send | Success boundary | Edit at settlement |
|---|---|---|---|---|---|
| `SLACK` | Slack app binding, channel id, `interactive` (team id copied from the binding) | the binding's bot token (`chat:write`) | `POST https://slack.com/api/chat.postMessage` through `infra-connectors/src/guard.rs` | HTTP 200 with `"ok": true` and a `ts` (stored as `provider_ref`) | `chat.update` with `channel` + stored `ts`; without `ts`, a follow-up |
| `TEAMS` | none | Workflows webhook URL | `POST` of an Adaptive Card to the secret URL | HTTP 2xx | not possible: follow-up card |
| `EMAIL` | recipients (≤ 50, `pii`) | optional relay credential | SMTP to the deployment relay, TLS required (implicit TLS or STARTTLS with certificate verification), egress guard | relay `250` after `DATA` | not possible: follow-up mail with `In-Reply-To` |

Every other answer maps to the existing attempt outcomes (`REJECTED` for a 4xx or `"ok": false` with a
permanent error, `THROTTLED` with `Retry-After` honoured for 429, `SERVER_ERROR`, `TIMEOUT`, `NETWORK`,
`DENIED` for egress refusal, `CREDENTIAL_UNAVAILABLE`). A provider's error text is never stored or logged:
only the closed outcome and, for Slack, the closed `error` token when it is in a fixed allow-list
(`channel_not_found`, `not_in_channel`, `invalid_auth`, `ratelimited`, `msg_too_long`), as a bounded code.
Twelve failed attempts make the delivery `DEAD` and twenty consecutive dead deliveries disable the
channel (ADR-0025 decision 3). Delivery is at-least-once: Slack and Teams have no documented idempotency
key, so a lost acknowledgement produces a duplicate message (G1322).

## Templates

Templates are part of the platform build, versioned (`template-v1`) and closed. Inputs are only:
deployment base URL, namespace display name, workflow name and version, run id, node id, statuses,
closed failure codes, alert kind, subject, counts and ages, approval deadline, decision link, and the
approval summary fields allowed by the channel ceiling. No author-supplied URL, image, markup or
mention is rendered.

| Event kind | Content |
|---|---|
| `run.finished` | workflow and version, status, closed failure code (failed runs), finish time, link `…/studio/runs/{runId}` |
| `ops.alert.fired` / `ops.alert.resolved` | kind, subject, severity, counts and oldest age, runbook link |
| `approval.requested` | workflow, node, deadline, summary fields at or below `maxClassification`, decision link button/URL |
| `approval.settled` | outcome (`APPROVED`, `REJECTED`, `EXPIRED`, `CANCELLED`) and time; no actor, reason or input |
| test | fixed text naming the channel and the time |

Escaping: Slack `mrkdwn` control characters (`&`, `<`, `>`) are entity-encoded and user values are never
placed in link syntax; Adaptive Card `TextBlock` text is plain (Markdown in values is escaped); e-mail has
a `text/plain` body and an HTML alternative in which every value is HTML-escaped; subject lines have
control characters and line breaks removed (header injection). Every value is cut at 512 characters and
the rendered message is bounded (plan ceilings); a message that would exceed its bound drops summary
fields from the end and says so.

## Summary filtering

The delivery worker reads the approval's stored summary (`approval_requests.summary_digest`), computes
each field's effective level against the pinned summary schema with the feature-016 walker, and includes
the field only when its level ≤ the channel's `maxClassification` (`public` by default; `public` <
`internal` < `pii`). Publication has already refused any summary field annotated below the level of
the locations its expression reads (`SUMMARY_LEVEL_BELOW_SOURCE`; a model-written value follows feature
025's floor), so the stored annotation is never looser than its sources. A
`secret` field is never rendered. An encrypted (feature 031) value is never rendered. A summary that does
not validate against its schema is omitted entirely (the approval itself is unaffected).

A `pii` field leaves only under the uniform egress rule (feature 020's O1081, coordinator decision C9):
the channel's `pii` ceiling was set by an administrator holding the namespace reveal grant
(`classification_set_by`); before each delivery that would carry a `pii` field the worker rechecks that
grant; if it no longer holds, the channel is paused (`PII_GRANT_REVOKED`, audited) and the delivery
waits; each delivery carrying a `pii` field is audited (`notification.pii_delivered`); `secret` never
leaves.

## Decision links

- Token: `v1.<secret>.<mac>` where `secret` is 32 random bytes (256 bits, base64url, 43 characters) and
  `mac` is HMAC-SHA-256 under key version `k` of `"octopus-decision-link-v1\n" ‖ secret`, truncated to
  16 bytes (base64url, 22 characters); about 69 characters in all. The MAC lets the server refuse a
  forged or mistyped token before any database access; the authority check is the row. The row stores
  only `SHA-256(token)`, `k` and its own identifier (`approval_decision_links.link_id`), so a database
  reader cannot rebuild a usable link (ADR-0061 decision 3).
- Keys: a file of versioned 32-byte keys (`SAGAT_LINK_KEYS`) mounted only into the control plane and
  shared, under distinct labels, with receipt tokens (research D1358); verification tries the current
  and the previous version; rotation adds a version; removing a version invalidates its links.
- Validity (infrastructure time, ADR-0063 draft of feature 028): `valid_until = created + min(approval
  timeout, 30 days)`, set at creation from the database clock; usable only while the row is `ACTIVE`,
  `valid_until` has not passed and the approval request is `PENDING`. The approval's workflow-time
  deadline is never compared with `valid_until`.
- URL: `{studio base}/studio/approvals/decide#l=<token>`; the page reads the fragment and clears it from
  the address bar (`history.replaceState`). When the person must sign in first, the page keeps the token
  in `sessionStorage` under one key for the duration of the OIDC redirect only and removes it on return;
  it is never put in local storage, a URL, a query or the OIDC `state`. The token grants nothing
  (ADR-0061), so this exposes no authority.
- Resolution: parse; verify the MAC (cheap refusal before the database); look up by hash; require the
  caller's tenant to equal the link's tenant (otherwise the same `404`); require `approval:read`; return
  the approval projection of the plan. The link grants nothing else.
- Decision: `decideWorkflowApproval` with the signed-in principal, a decision key generated per page
  load and kept across retries, the optional `input` and the link token. The server verifies the token
  and derives `via` (`SLACK`, `TEAMS` or `EMAIL` from the link's delivery; `API` when no valid link of
  that approval is sent); the client never asserts it. FR-014-008 checks apply at commit. Settlement
  closes every link of the request.

## Slack app bindings and interactivity (P3)

One Slack app model (ADR-0061 decision 7): each tenant creates its own Slack app and registers it as a
namespace *Slack app binding* with two secret bindings, the bot token and the signing secret. There is no
platform-distributed app and no deployment-wide signing secret.

- **Registration** (`POST /v1/namespaces/{namespaceId}/slack-apps`, `event:manage` + `secret:use`): the
  control plane resolves the bot token and calls `POST https://slack.com/api/auth.test` through the
  egress guard, outside the transaction; it stores the returned `team_id` (never a client value), then
  commits the binding, its audit and its quota count. Re-verification runs on every channel change and
  daily; a token that fails or now answers another team marks the binding `INVALID` and refuses
  interactive use.
- **Interactive channels**: a `SLACK` notification channel with `interactive: true` is unique by
  `(team, channel)` across all tenants (a global partial index); a second tenant's attempt is
  `CONFLICT` (`CHANNEL_ALREADY_BOUND`) and says nothing about the first. The Slack app's interactivity
  and slash-command request URL is `/v1/chat/slack/apps/{bindingId}/requests`.
- **Verification, before any database read beyond the binding row**: body ≤ 16 KiB,
  `application/x-www-form-urlencoded`; a per-binding rate guard; `X-Slack-Request-Timestamp` within 300 s
  of the platform clock; `X-Slack-Signature` equals `v0=` + hex HMAC-SHA256 of
  `v0:{timestamp}:{raw body}` under the binding's signing secret (current and previous during rotation),
  compared in constant time. The signing secret is cached in process for a bounded time so that unsigned
  floods do not drive provider reads. Any failure, including an unknown binding id: the same `401`.
- **Tenant resolution**: the button value carries the decision-link token. The dispatcher resolves the
  tenant and namespace **only** from that link row and requires: the link's tenant and namespace equal
  the binding's; the link's delivery went to a channel of this binding whose team and channel equal the
  payload's `team.id` and `channel.id`; the link is usable. Otherwise nothing is decided and
  `chat.refused` (`TENANT_MISMATCH`, `CHANNEL_MISMATCH`, `LINK_UNUSABLE`) is audited in the binding's
  tenant only.
- **Admission and decision**: one inbox row keyed by `(tenant, namespace, source = slack:<bindingId>,
  event id = <action ts and action id>)`; the handler answers `200` after the inbox commit. The
  dispatcher looks up the active chat identity link for `(team id, user id)` inside the link's tenant
  only and applies the feature-014 decision as that subject with `via: SLACK`; it posts an ephemeral
  reply through the outbound pipeline. Unlinked, pending, revoked, ineligible or settled: an ephemeral
  reply with the decision link; no decision.

### Identity linking

1. The signed-in subject asks the Studio for a code for one binding of its namespace. The code is 128
   random bits or more (base32, shown once), valid 10 minutes, stored as a SHA-256, at most 5 per subject
   per hour.
2. The person runs the app's slash command with the code in the bound workspace. The verified request
   (as above, per-team rate limit of 10 code attempts per minute) is admitted to the inbox; the
   dispatcher consumes the code with the `SECURITY DEFINER` function, which succeeds only once, only
   before expiry and only when the request's team equals the binding's team, and creates a `PENDING`
   link for `(team id, user id)`.
3. The subject's next signed-in Studio session shows "Link Slack user `U…` of workspace `T…`?"; only
   the subject's explicit confirmation makes the link `ACTIVE` (audited). A code used by somebody else
   therefore produces at most a pending link the victim sees and declines.
4. Revocation by the subject, by an administrator, or automatically with the membership; a revoked or
   pending link never decides.
