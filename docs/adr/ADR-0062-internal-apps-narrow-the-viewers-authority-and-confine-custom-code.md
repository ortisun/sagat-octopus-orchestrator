# ADR-0062: An internal app narrows its viewer's own authority through a server-side session, and author code runs only in an opaque-origin frame

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26)
**Scope:** brings a bounded app builder into the product although `docs/architecture/overview.md`
("Explicit scope left for later releases") defers a "Generic low-code style application designer".
Acceptance of this ADR requires the owner's answer to O1360 (`specs/027-internal-apps/research.md`)
and the matching edit of the overview; until then the decision is a proposal only.
**Refines / supersedes:** refines ADR-0016 and is the single home of its combined refinement for
origins and content security policy (decision 8, coordinator decision C15 of 2026-10-27): the Studio
origin stays unframeable and serves no author or anonymous content; three operator-configured origins
are added (apps, component sandbox, public forms of feature 026); the Studio policy gains one
`frame-src` and the operator-approved codec origins of feature 031 in `connect-src`. Features 026
(ADR-0061) and 031 (ADR-0066) cite this decision for those parts. It also refines ADR-0018 (applies its
publication, pointer-history and approval rules to app documents); preserves ADR-0028, ADR-0031 and
ADR-0033 unchanged; supersedes none.
**Enforced by:** proposed checks of feature 027, none of which exists yet — the role × binding ×
scope matrix (`tests/integration/internal-apps/authorization.rs`, T027-024), the cross-tenant
battery extended to the new routes (`tests/contract/security-multitenancy/rest/battery.rs`), header
contract tests for the Studio, apps and sandbox surfaces on every status code (T027-039, T027-045),
a REST contract test that every key-taking route refuses an ordinary `app:` key (T027-023), the
author-level and frame-taint publication fixtures (T027-047), the canary suite (T027-030), the
malicious-bundle browser suite (T027-048), a start-up configuration test that refuses every origin
equality forbidden by decision 8 and a header test that codec origins appear only in the Studio
policy (T027-039), and a domain unit test that an app session cannot be
converted into a `Principal` with more permissions than its subject's membership.

## Context

Feature 027 adds Windmill-style internal apps: pages of tables, forms, charts, text, buttons and tabs
whose data comes from declared Queries/Updates, workflow starts, run lists and classified run
results. Four forces make this a hard-to-reverse decision:

1. **Whose authority does an app use?** Windmill's default executes an app's runnables "on behalf
   of" the publisher, so a viewer can do what the publisher can. That turns every published app into
   a standing grant that bypasses the namespace's roles and audit of the actual person, and it is the
   confused-deputy pattern the platform has refused elsewhere (ADR-0031 grants are namespace-wide and
   checked on the caller; the proposed ADR-0061 of feature 026 refuses bearer-link decisions).
2. **Where does an app's data flow?** If the browser runtime calls the general API with the viewer's
   bearer token, any defect in the runtime, or any author-controlled script, can call anything the
   viewer can. The Studio today holds its token in memory under a strict same-origin policy
   (`crates/api-http/src/studio.rs`: `frame-ancestors 'none'`, no `frame-src`).
3. **Where does author code run?** Custom components are author JavaScript executed in other
   people's browsers. The constitution forbids untrusted code in the control plane; in a browser the
   only isolation available is an opaque-origin sandboxed frame with a strict CSP. That isolation
   protects credentials and the host page; it does not stop a frame from sending out data it was
   given (WebRTC is outside `connect-src`; a frame can navigate itself).
4. **Scope.** The architecture overview defers a generic low-code designer to a later release. This
   ADR is the bounded version of that idea (closed components, declared platform bindings, no app
   backend) and stands or falls with the owner's answer to O1360.

## Decision

1. **Viewer identity, narrowed.** Every binding invocation MUST be authorized as the person viewing
   the app, with their current membership and permissions, intersected with (a) the bindings
   declared in the app version pinned by their session and (b) the app's audience. An app MUST NOT
   grant a permission its viewer lacks. There is no publisher, delegated or anonymous execution mode;
   adding one requires a new ADR.
2. **App sessions are the only runtime credential.** Opening an app MUST exchange the viewer's
   bearer token for an opaque app session token (32 random bytes, stored only as SHA-256 in
   PostgreSQL), bound to tenant, namespace, app, one version (or one draft revision for a preview),
   subject and mode, expiring no later than the exchanged token. App runtime routes MUST accept only
   app session tokens, and every other route MUST refuse them. Sessions MUST be revocable at once;
   an activation MAY revoke sessions of other versions in its own transaction.
3. **Server dispatch of declared bindings.** The runtime MUST invoke a binding only by its id with
   argument values. The server MUST resolve the binding from the pinned version, refuse a target
   outside the declared workflow and the session's namespace as an opaque `NOT_FOUND`, validate the
   arguments, and call the existing use case in process. The audit record of an app write MUST
   commit in the same transaction as the underlying decision. The idempotency key of an app write
   MUST be derived as `app:<app-id>:<binding-id>:<action-key>`, and the prefix `app:` MUST be refused
   in every caller-chosen key of every other route, so app actions and ordinary requests never share
   a key and two bindings never share one.
4. **Closed documents.** An app version MUST be an immutable canonical document with a digest,
   published, approved, activated and promoted under ADR-0018's rules. The document MUST NOT carry
   HTML, CSS, scripts, script URLs or browser-evaluated expressions; values come from a closed
   reference language and conditions from closed predicates that control presentation only.
5. **Embedding needs its own origin.** Framing an app MUST be possible only on an operator-configured
   apps origin distinct from the Studio's, which serves only the runtime page, its sign-in callback,
   session creation and runtime routes, never component frames. The Studio MUST keep `frame-ancestors 'none'`. The allowed embedders
   MUST be exact `https` origins in the intersection of the app's list, the namespace policy and the
   deployment allowlist. Embedded sign-in MUST be a user-initiated popup to the configured issuer
   that hands back only the authorization code, by `postMessage` to the exact apps origin, accepted
   only from the opened popup with a matching single-use `state`; the control plane MUST redeem the
   code server-side so that no issuer token exists in apps-origin script. The host page MUST NOT be
   able to supply a credential.
6. **Author code only in opaque-origin frames on a sandbox origin.** A custom component MUST run
   only in an `<iframe sandbox="allow-scripts">` whose document carries the CSP `sandbox
   allow-scripts`, hash-pinned inline scripts and styles, `connect-src 'none'` and
   `form-action 'none'`, and MUST NOT receive any token or platform API access. Frames MUST be served
   only from a dedicated component sandbox origin that is never the Studio, apps or public-page
   origin and serves nothing else, with the sandbox headers on every response at every status, through
   a single-use capability that never outlives its session. Custom components MUST be disabled per
   namespace until an administrator enables them, and unavailable in a deployment without a sandbox
   origin.
7. **A component receives at most what its author may read.** Values passed to a frame MUST be
   limited to effective `public`/`internal` levels, `pii` only where the published version declares
   it, `secret` never. Publication MUST refuse a version unless its publisher and every uploader of a
   pinned component version hold the rights for the highest level the version passes to that
   component; session creation MUST re-check the uploaders. Values originating in a frame MUST NOT
   reach the target or arguments of a binding whose result can flow to a frame, nor of any write
   binding. The platform MUST NOT describe the frame as a confidentiality boundary for the values it
   receives.
8. **Origins and content security policy (combined refinement of ADR-0016).** A deployment has at
   most these browser origins, each configured by the operator, compared exactly
   (scheme, host, port) and selected by the `Host` header without trusting forwarded headers:

   | Origin | Setting | Serves | MUST NOT |
   |---|---|---|---|
   | Studio | existing | The Studio and its API, as ADR-0016 | be framed (`frame-ancestors 'none'`); serve author-supplied or anonymous content (no app runtime page, component frame or public form page; feature 026's public paths answer `404` there) |
   | Apps | `SAGAT_APPS_ORIGIN` (027) | App runtime page, its sign-in callback, app session creation and the app runtime routes | serve component frames or any other API route |
   | Component sandbox | `SAGAT_APPS_SANDBOX_ORIGIN` (027) | Component frame documents only, with the sandbox headers on every response | equal any other origin of this table |
   | Public forms | `SAGAT_PUBLIC_FORMS_ORIGIN` (026, ADR-0061) | Feature 026's anonymous public form pages and their routes, with per-link `frame-ancestors` | equal the Studio or sandbox origin |

   The public-forms origin MAY share its host with the apps origin (coordinator decision C15); when
   scheme and port also match they are one browser origin, so a deployment SHOULD give them distinct
   hosts (G1645). Start-up MUST refuse any configuration that breaks the "MUST NOT" column.
   The Studio policy keeps every directive ADR-0016 set and gains exactly two things: `frame-src`
   limited to the sandbox origin's frame path (027), and in `connect-src` the codec-server origins the
   operator lists in `SAGAT_STUDIO_CODEC_ORIGINS` (feature 031, ADR-0066), deployment-wide and never per
   tenant; this ADR accepts that addition. Codec origins MUST be exact `https` origins (plain `http` only
   on loopback in a development deployment, as feature 031 states) and MUST NOT be added to the apps,
   sandbox or public-forms policies: apps do not decrypt in this feature (feature 027 D1394).

## Considered options

1. **Viewer identity through server-side sessions and declared bindings (chosen).** Keeps the
   platform's authorization model and audit truthful; an app is a lens, never a grant. Cost: a person
   needs the underlying permission, so read-only viewers can only read (G1362 of feature 027); a
   per-app grant waits for feature 029 or a later decision.
2. **Publisher (or app-principal) identity with an enumerated policy, Windmill's default.** Lets
   people without permissions act through an app. Lost: it creates standing grants outside the role
   model, makes the audited actor ambiguous, and needs its own approval and revocation lifecycle; kept
   as owner question O1361.
3. **Runtime calls the public API with the viewer's bearer token.** Simplest, no new routes. Lost:
   the browser holds a general credential, the server cannot know which app made a call, and nothing
   enforces "only what the version declares".
4. **Signed JWT app tokens instead of stored sessions.** No database read per call. Lost: needs a new
   signing key and cannot be revoked before expiry, which activation-time revocation requires.
5. **Embedding the Studio origin with a per-route `frame-ancestors`.** No ingress change. Lost:
   privileged Studio pages and framed pages would share one origin; a mistake in one route's policy
   exposes the other.
6. **Custom components in the trusted page (Windmill low-code) or none at all.** In-page code would
   hold the session; no components would leave a parity gap. Off-by-default frames are the bounded
   middle.

7. **Frames on the Studio or apps origin.** No third host. Lost (architecture review, 2026-10-27):
   isolation would rest on one header; any response path that lost it would run author code
   same-origin with the Studio token or an app session.
8. **Viewer-level data rule only.** Simpler publication. Lost (security review, 2026-10-27): a
   component author without a right could read through an administrator's session what the author may
   not read, by steering a binding from frame state.

## Consequences

- New contracts: app document schema, app/runtime REST paths, an `appSession` security scheme,
  additive error reasons and manifest keys. No workflow IR, worker-protocol or AsyncAPI change.
- Each invocation costs one indexed session read beyond the underlying operation; it is measured,
  not promised (NFR-027-005).
- Deployments that want embedding must add an apps host to ingress and register its redirect URI
  with the issuer; deployments that do not keep today's surface.
- Rolling back: disable the `apps` capability (no session creation, no publication); tables,
  versions and audit stay; no run is affected because apps own no execution state.
- Data passed to a custom component must be treated as disclosed to its author (G1361); decision 7
  bounds that author to what they may already read (G1375), and the Studio says so when the namespace
  policy is enabled.
- Custom components need a third host (the sandbox origin) in ingress and DNS; deployments without
  it keep built-in components only.
- Reserving the `app:` key prefix refuses new ordinary keys with that prefix, a behaviour change for
  any client already using it (G1373); exact replays of older records stay valid.
- Embedded sign-in needs the control plane to reach the issuer's token endpoint and the issuer to
  accept the apps-origin callback for the platform's client (G1364).
- One table now says which origin may serve what (decision 8); a new surface that needs its own
  origin or a CSP change refines this ADR instead of ADR-0016 directly.

## Evidence

None yet: this is a proposal. The design is in `specs/027-internal-apps/` (spec, plan, research
D1361–D1375 and D1397–D1399, contracts). The independent architecture and security reviews of
2026-10-27 (frames on the Studio origin, key collision, scope citation, embedded token exposure,
component data at the viewer's level) led to decisions 3, 5, 6 and 7 as written here; the cross-feature coordinator decision C15 added decision 8. External
semantics consulted on 2026-10-26: Windmill's app editor and app
`Policy` documentation, MDN on CSP `sandbox` and `frame-ancestors`, and the CSP `webrtc` directive
status. Acceptance requires the checks named under **Enforced by** to exist and pass.
