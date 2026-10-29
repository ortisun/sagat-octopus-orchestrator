# Quickstart — internal apps (planned demo)

**Nothing here runs today.** This is the reproducible demo the Definition of Done asks for, written
before the implementation so the tasks can be checked against it. Each step names what must be
observed; T027-061 turns it into an automated check and records the real result, including any step
that was skipped. Routes and bodies are the drafts in [contracts/](contracts/).

## Prerequisites

- A build of this feature, the local stack (`infra/docker/compose.yaml`: PostgreSQL, NATS, control
  plane with the Studio, engine, `dev-idp`) and `sagat` from `apps/cli`.
- For part D only: the Compose overlay of T027-042 (`SAGAT_APPS_ORIGIN`,
  `SAGAT_APPS_EMBED_ALLOWLIST`, `SAGAT_APPS_SANDBOX_ORIGIN`, three distinct local origins besides the
  Studio's) and a further local origin acting as the portal.
- Members of tenant `dev`, namespace `support` (provisioned by the test support, G290 of feature 016):
  `alice` (developer), `oscar` (operator), `vera` (viewer).

```sh
docker compose -f infra/docker/compose.yaml up -d
export API=http://127.0.0.1:18080/v1 NS=support   # SAGAT_API_PORT default of the compose file
export ALICE=$(sagat dev token --subject alice --tenant dev)
export OSCAR=$(sagat dev token --subject oscar --tenant dev)
export VERA=$(sagat dev token --subject vera --tenant dev)
```

The fixture workflow `refund` (T027-005, `tests/contract/internal-apps/fixtures/refund.workflow.json`)
declares state with the Query `summary` and the Update `approve` (`disclosure:
declared-business-result-v1`); `refund-reconciliation` has a classified `outputSchema` with one field
at each level. Publish both with `sagat publish` and start two `refund` runs that wait.

## A. Happy path

1. **Create and draft.** As Alice:
   `POST $API/namespaces/$NS/apps` with `{"name":"refund-desk","title":"Refund desk"}` →
   `201`, `draftRevision: 0`. `PUT …/apps/refund-desk/draft` with `expectedRevision: 0` and
   [refund-desk.app.json](contracts/examples/refund-desk.app.json) → `200`, revision 1.
2. **Validate.** `POST …/apps/refund-desk/validate` → `valid: true`; warnings may include
   `UPDATE_WITHOUT_EXPECTED_REVISION` only if the example is edited to drop it.
3. **Publish.** `POST …/apps/refund-desk/publish` (`Idempotency-Key: demo-publish-1`,
   `{"draftRevision":1}`) → `201`, version 1 with a `sha256:` digest. Repeat the same request → the
   same version, and exactly one `app.published` audit record (`GET $API/audit`).
4. **Open as Oscar.** In the Studio: Namespaces → `support` → Apps → Refund desk → Open. Or
   `POST …/apps/refund-desk/sessions` with `{"mode":"studio"}` → `201` with `sessionToken`,
   `expiresAt` no later than Oscar's token expiry, the pinned document and no frames.
5. **Read.** The table shows the two waiting runs (`listRuns`). Selecting one invokes `summary`
   (`POST $API/app-runtime/bindings/summary/invoke`, `Authorization: Bearer <sessionToken>`) and shows
   the declared result; the response carries `Cache-Control: no-store`.
6. **Write.** Approve → a confirmation written by the platform names *Update `approve` of workflow
   `refund`, run `<id>`, namespace `support`*. Confirm → one invocation with an `actionKey`; the run's
   state revision increases once; audit holds `run.updated` and `app.binding_invoked` with the same
   trace id, committed together.
7. **Start and result.** Start a reconciliation → a run of `refund-reconciliation`; when it ends, the
   result card shows `public` and `internal` fields, `pii` as *Withheld* with a Reveal action, and no
   `secret` field.
8. **Leave.** Close the app. Browser storage (localStorage, sessionStorage, IndexedDB, Cache Storage)
   is empty for the Studio origin; the session row has `ended_at`.

## B. Failure scenarios

1. **Viewer cannot act.** Open the app as Vera: the run list shows; the summary card and Approve show
   *Unavailable — needs run:query / run:update*. Calling the invoke route directly with her session →
   `403 FORBIDDEN`, an `access.denied.permission` record, no state change.
2. **Wrong run.** Invoke `summary` with the id of a run of another workflow, of another namespace and
   of another tenant → three identical `404 NOT_FOUND` answers.
3. **Lost response and duplicate.** Repeat step A6's request with the same `actionKey` → `200`, the
   same receipt, state revision unchanged. Same key with another note → `409 CONFLICT`
   (`IDEMPOTENCY_KEY_REUSED`).
4. **Revocation.** While Oscar's session is open, suspend his membership (test support) → his next
   invocation is `403`. Restore it.
5. **Rollback with revocation.** Publish version 2 (edit a label), open a session on it, then
   `PUT …/apps/refund-desk/active-version` with `{"version":1,"expectedVersion":2,"reason":"demo
   rollback","revokeSessions":true}` → `200`, `sessionsRevoked ≥ 1`; the version-2 session's next
   call is `401` (`APP_SESSION_EXPIRED`). A stale `expectedVersion` → `409 ACTIVE_VERSION_CHANGED`.
6. **Approval required.** Set the namespace publication policy to one approval; a direct publish →
   `409 APPROVAL_REQUIRED`; Alice opens a request, Alice cannot approve it, Oscar approves, Alice
   publishes; editing the draft before that supersedes the request.
7. **Promotion refused.** Promote version 1 into a namespace without `refund` → `422`
   (`MISSING_DEPENDENCY`) naming `refund`; nothing is published there.
8. **Crash.** With the fault harness, kill the control plane after the publish statement and before
   commit → no version; after commit → replay returns the version (T027-054).

## C. GitOps

Write a manifest listing `workflows` and `apps` (`manifest-apps.schema.json`), run
`sagat validate`, `sagat diff`, `sagat publish` twice: the second run reports no change. An older
`sagat` refuses the manifest because of the unknown `apps` key.

## D. Embedding and a custom component

1. Without `SAGAT_APPS_ORIGIN`: the Embedding screen says *Unavailable — the deployment has no apps
   origin*; `PUT …/embedding` → `422 EMBEDDING_UNAVAILABLE`.
2. With the overlay: allow the portal origin (in the deployment allowlist, the namespace policy and
   the app). The portal frames `<apps origin>/apps/support/refund-desk`; Sign in opens a popup; the
   callback page posts only the code to the apps origin and the control plane redeems it; after
   sign-in the app works inside the portal, and no issuer token is found in the page's memory,
   storage or responses. A page on a non-allowed origin is blocked by the browser; framing any Studio
   page is blocked (`frame-ancestors 'none'`).
3. Without `SAGAT_APPS_SANDBOX_ORIGIN`, enabling custom components is `422`
   (`COMPONENT_SANDBOX_UNAVAILABLE`). With it, enable them, upload the fixture `timeline` bundle as
   Alice, add it to the app and publish. The frame loads from the sandbox origin; reloading its URL
   gives `404` with the sandbox headers (spent capability). In the browser, the frame's `fetch` fails
   (CSP), `window.parent.document` throws, storage access throws; the malicious fixture that navigates
   itself is torn down and shown as *Component failed*. Binding a `pii` property without `allowPii`
   fails validation with `CLASSIFICATION_PASSING`.
4. Upload a component as a member without `payload:read-internal` and bind it to a `runResult`
   field that can be `internal` → publication fails with `COMPONENT_AUTHOR_LEVEL`. Feed a value the
   component sets into the run target of that `runResult` binding, or into the `approve` arguments →
   `FRAME_TAINTED_FLOW`.
5. Call `startRun` with `Idempotency-Key: app:anything` → `422` (`RESERVED_KEY_PREFIX`).

## What this demo does not prove

Throughput or latency (NFR-027-005 needs its own benchmark report), behaviour with a real enterprise
issuer (G1364), or that a component cannot exfiltrate data it was given (G1361).
