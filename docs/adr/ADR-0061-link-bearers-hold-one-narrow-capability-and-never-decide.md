# ADR-0061: A link carries one narrow capability: an anonymous submitter may only submit its form, and a decision link never authorizes a decision

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26)
**Refines / supersedes:** refines ADR-0033 (adds an anonymous audience to its disclosure table) and
ADR-0012/ADR-0026 (a second kind of unauthenticated public request); follows ADR-0062 (feature 027)
for origins and CSP; preserves FR-014-008 and ADR-0031 unchanged; supersedes none.
**Enforced by:** proposed checks of feature 026 — the security battery extended to the public
operations (`tests/contract/security-multitenancy/rest/battery.rs`), the public-link and decision-link
suites under `tests/integration/invocation-human-surfaces/` and `tests/fault/invocation-human-surfaces/`
(T026-029..033, T026-039, T026-040, T026-044, T026-045, T026-047, T026-048, T026-051), a domain
unit test that the public-link principal type cannot be converted into a `Principal` or authorize
any `Permission`, a control-plane start-up test that refuses equal Studio and public-forms origins,
a header test that the Studio origin answers `/f/` and `/v1/public/*` with `404`, and review. None
exists yet.

## Context

Feature 026 adds two kinds of URL that people outside a signed-in session will hold: a **public form
link** that lets anybody start one workflow (Windmill public apps and forms), and a **decision link**
sent to Slack, Teams or e-mail with an approval (Windmill's resume URLs). Both travel through
channels the platform does not control: chat history, mail archives, forwarded messages, browser
history, screenshots.

Until now every caller is either an authenticated tenant member with a role (`Principal`:
tenant, subject, role) or a signed webhook sender whose endpoint supplies tenant and namespace
(ADR-0012, ADR-0026). Authorization fails closed, a foreign resource is a missing one, and a
human approval is decided only by a principal that is eligible **at decision time** (FR-014-008).
Windmill's resume URLs are bearer capabilities: whoever holds the URL can approve unless the step
requires approvers to be logged in. Adopting that default here would make a forwarded e-mail an
approval, break FR-014-008 and make the audit record name a link instead of a person.

The forces: anonymous intake is a real product need; a link cannot be revoked from the places it was
copied to, only made useless; the platform has no CAPTCHA or reputation service; source addresses are
personal data (LGPD) and must not become a forensic store by default; the run started by an anonymous
person still needs an accountable owner for secrets and managed operations, as triggered runs
need their publisher.

## Decision

1. **A public-link caller is its own principal kind, not a subject.** It carries tenant, namespace,
   link id and link revision, all read from the stored link, never from the request. It MUST NOT be
   convertible into a `Principal`, MUST NOT have a membership or a role and MUST NOT authorize any
   catalog `Permission`. It holds exactly one capability: submit the pinned version's form through
   this link, with the link's presets and exposed fields. It is recorded as the reserved actor
   `system:public-link:<linkId>`, which no token can authenticate as.
2. **The run is admitted on behalf of an accountable owner, checked every time, in its own pool.** The
   link records its owner. Each submission MUST take its locks in the slots of ADR-0050 decision 7 (the
   link row in slot 1, the owner's membership and the namespace row in slot 4a, the public-link pool with
   slot 6's active-run admission), after debiting the durable rate buckets in their own committed
   decision, and recheck, in the admission transaction, that the owner is still a member of the
   tenant whose role grants `run:start` in the namespace, that the feature-029 evaluator (when enabled)
   still lets the owner see and start the pinned workflow, that the link is active, unexpired and under its submission cap, that its live runs are
   under its required concurrent-run cap, and that the namespace's pool of live public-link runs
   (`public_link_active_runs`, a sub-pool of `active_runs`, counted exactly under the feature-009 lock)
   has room. Any failure refuses the submission like an unknown link (`429` for capacity). Authority for
   secrets and managed operations of the run is the publisher's, exactly as for triggered runs.
3. **Link tokens are random, hashed, revocable and kept out of logs.** A public-link token, a receipt
   token and a decision-link token MUST carry at least 256 bits that an attacker cannot predict, MUST be
   stored only as a SHA-256 (with the key version where a key is used), and MUST travel in the URL
   fragment or the response body and then a request header, never in a path or query string. A
   decision link adds an HMAC-SHA-256 tag under a rotatable platform key so that garbage is refused
   before the database. A receipt token is an HMAC of the link and the submission-key hash under the
   same key set with its own label, so that a replay returns the same token without storing it; the
   submission key carries at least 128 random bits. Token validity is infrastructure time (the
   time-class catalog of feature 028 and its ADR-0063 draft): a decision link is valid until creation plus the lesser of the approval
   timeout and 30 days, and only while the approval is `PENDING`; the approval's workflow-time deadline
   is never compared with it. Revocation, rotation, expiry and settlement take effect on the next
   request.
4. **Every unusable link looks the same, and public pages live on their own origin.** Unknown,
   expired, paused, revoked, exhausted, at-capacity and owner-unauthorized public links MUST answer the
   same `404`; an unusable decision link MUST show the same page. Public pages and the `/v1/public/*`
   routes MUST be served only on an operator-configured public-forms origin that is not the Studio
   origin (it may share a host with the feature-027 apps origin; ADR-0062 holds the combined refinement
   of ADR-0016 for the three origins and their CSP, and this decision follows it); the Studio origin answers those paths
   `404` and the control plane refuses to start when the origins are equal. The page path carries a
   non-secret embed id, so `frame-ancestors` is computed from that one link (`'none'` for an unknown id
   or an unusable link) and no tenant's approved site can frame another tenant's form. The cost of a
   caller who proved nothing MUST be bounded before the database: format and size checks, a
   per-process keyed rate guard (keyed on the peer address; a forwarded address only from configured
   trusted proxies), then the durable per-link and per-namespace token buckets of ADR-0027. Public links
   are disabled until an administrator raises the namespace count quota `public_links` above its
   default of 0.
5. **An anonymous audience sees only declared `public` values.** ADR-0033's table gains one row: an
   anonymous caller MUST see no value unless the link declares `resultDisclosure: public-fields-v1`,
   and then only result fields whose effective level is `public`, through a receipt token bound to
   that one submission. For a value a model wrote (feature 025), the effective level is at least the
   highest level of everything that entered its transcript, whatever the author's annotation says
   (feature 025's model-output floor). `internal`, `pii` and `secret` values, failure codes, run
   identifiers and step data MUST NOT be returned to an anonymous caller, through a receipt, a stream or
   an error. The same rule governs a synchronous custom route (feature 021) that declares
   `public-fields-v1`.
6. **A decision link is a pointer, never authority.** Opening a decision link MUST require the same
   sign-in as the Studio. The decision MUST be the existing feature-014 decision of the signed-in
   principal, with its eligibility, initiator exclusion, tenant check and decision key, evaluated at
   commit; the link only identifies the approval, and the server derives the attribution (`via`) from
   the verified link, never from a client assertion. When feature 029 is enabled, resolving a link also
   requires that its evaluator lets the caller see the run's workflow. A link forwarded to another eligible member lets
   that member decide as themself; a link held by anybody else decides nothing. Deciding by possession
   of a link without signing in is not allowed by this decision (owner question O1320 of feature 026).
7. **A chat identity acts for a subject only after that subject linked and confirmed it, inside the
   tenant the decision link names.** There is one Slack app model: a tenant-owned Slack app registered
   as a namespace binding whose team id the platform obtains from Slack `auth.test` (never from the
   client); an interactive Slack channel is unique by `(team, channel)` across tenants. A request MUST
   arrive at its binding's own URL and verify with that binding's signing secret (`v0`, 300-second
   window, constant time, uniform `401`), be deduplicated through the inbox, and take its tenant and
   namespace only from the decision-link row in the button value, which MUST match the binding's
   tenant, team and channel. The chat identity is looked up only inside that tenant. A link between a
   chat identity and a subject is created from a single-use code of at least 128 random bits, consumed
   atomically, valid for 10 minutes, usable only from a team the code's tenant bound, rate-limited per
   team, and becomes active only after the subject confirms the exact Slack user and workspace in a
   signed-in Studio session. An unlinked, pending or revoked identity receives the decision link and
   decides nothing.
8. **No source address is kept.** Rate guards MAY key on a source address in memory; audit, logs,
   spans, metrics and stored rows MUST NOT contain source addresses, user agents or submitted values.

## Considered options

1. **Narrow principal kind, owner rechecked, pointers for decisions (chosen).** It keeps FR-014-008,
   ADR-0031 and ADR-0033 true, makes every anonymous effect attributable to a link and an accountable
   owner, and makes a leaked decision link harmless. It costs a sign-in for every channel approver and
   a separate code path for anonymous requests.
2. **Bearer resume URLs (Windmill's default).** Simplest for external approvers, but a forwarded or
   archived message becomes an approval, the decider is unknown, and decision-time eligibility and
   initiator exclusion become meaningless. Rejected; kept as owner question O1320 for an explicitly
   restricted future mode.
3. **Model the public link as a service-account `Subject` with a restricted role.** Reuses the
   permission machinery, but a subject can be granted more later, appears in membership lists, can be
   confused with a person in approval eligibility and audit, and a token for it could be minted by the
   identity provider. Rejected.
4. **Signed, stateless link tokens (JWT-like) without a database row.** Cheap to verify, but revocation
   and settlement need a lookup anyway, and a stateless token in a query string ends up in logs.
   Rejected for authority; an HMAC tag is kept only as a cheap pre-filter for decision links.
5. **Serve the public page on the Studio origin under `/f/`.** One origin to operate, but `frame-ancestors`
   could only be `'none'` or the union of every tenant's approved sites (the token is in the fragment,
   which the server never sees), and an XSS in the anonymous page could script a Studio page of the same
   origin and read its in-memory token (security review B1; ADR-0062 rejected the same for apps).
   Rejected.
6. **Read the tenant of a Slack interaction from the stored channel matched by team and channel.**
   Two tenants could register the same team and channel and nothing proved who owns the team
   (architecture review B1). Rejected for decision 7.

## Consequences

- Every channel approver needs a platform account; external approvers without one cannot decide
  (documented; O1320).
- The operator runs a second origin (public forms, optionally sharing the host of the feature-027
  apps origin) with its own TLS certificate and ingress route; without it, public links cannot be used.
- Slack interactivity needs each tenant to create its own Slack app and register its bot token and
  signing secret; there is no platform-distributed app (G1335). A Slack channel can be interactive for
  one tenant only.
- The public surface grows: a separate minimal page entry, unauthenticated routes and their abuse
  limits must be tested as release-blocking (constitution V).
- Owner demotion or departure silently disables that owner's links; the namespace alert
  `PUBLIC_LINK_OWNER_UNAUTHORIZED` makes it visible, and an administrator transfers ownership by
  editing the link.
- Without stored addresses, abuse forensics rely on per-link counts, audit of refusals and rate limits
  (G1320).
- Rollback: disabling creation of links (quota 0) and pausing existing links stops all anonymous
  admission while receipts and runs remain readable to members; decision links can be left unused, as
  decisions keep working through the Studio and API.

## Evidence

None yet: this is a proposal. Required before acceptance: the tests named under **Enforced by**,
including cross-tenant and canary checks over the public operations, a crash at each commit point of
submission and decision, replay of submissions and decisions, owner demotion during submission, link
revocation during submission, decision-link forwarding to an ineligible member, Slack signature and
replay refusals, and the measured cost of refused requests before the database. See
[spec](../../specs/026-invocation-human-surfaces/spec.md),
[research](../../specs/026-invocation-human-surfaces/research.md) and
[plan](../../specs/026-invocation-human-surfaces/plan.md).
