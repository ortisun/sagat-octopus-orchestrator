# Quickstart — feature 026 demo (happy path and failure scenarios)

Status: Proposed — not runnable. Nothing below exists yet: the new routes, CLI subcommands and fixtures
are what tasks T026-002..066 build. This is the reproducible demo the Definition of Done requires; it
is executed and its real output recorded by T026-064. Commands that exist today are marked *(exists)*.

## Prerequisites

- The local stack *(exists)*:
  `docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build`, then the
  seed profile; `export SAGAT_TOKEN=$(sagat dev token --subject alice --tenant acme)` *(exists)*.
- A second identity for decisions: `sagat dev token --subject bob --tenant acme` (operator), and a
  third tenant identity for the cross-tenant check: `--subject eve --tenant other`.
- The test stand-ins from `tests/support` (T026-003): a Slack/Teams receiver on loopback and an SMTP
  sink. The deployment points Slack at the stand-in through the development-only base-URL override; a
  production profile refuses that override.
- `SAGAT_LINK_KEYS` set to a generated development key file (never committed), and
  `SAGAT_PUBLIC_FORMS_ORIGIN` set to a loopback origin different from the Studio origin (for example
  `http://forms.localhost:8081`); the control plane refuses to start when the two are equal.

## 1. Publish a workflow with a form, a stream and a notified approval

Use the excerpt of [contracts/workflow-ir-fragment.schema.json](./contracts/workflow-ir-fragment.schema.json)
(`refund-request`), completed with the `refund` and `notify-rejected` activity nodes, and a worker that
publishes ten `progress` events while `refunds.assess` runs.

```sh
sagat slack-app register finance-bot --bot-token-secret slack-bot --signing-secret-secret slack-signing
# the stand-in answers auth.test with team T0STANDIN; the binding stores it
sagat channel create finance-approvals --slack-app finance-bot --slack-channel C0123456789 \
  --kinds approval.requested,approval.settled,run.finished --max-classification internal
# internal needs payload:read-internal (alice is admin); the default would be public
sagat channel test finance-approvals            # a delivery appears at the stand-in
sagat workflow publish refund-request.json       # (exists) now with streams/notify/summary/form
sagat workflow form refund-request --version 1   # FormModel: AVAILABLE, contactEmail level pii
```

Expected: the form lists `orderId` (internal), `amount` (internal), `reason` (pii, textarea) and
`contactEmail` (pii); the channel test message reaches the stand-in through the delivery pipeline.

## 2. Start from the form and follow the stream (US01, US03)

In the Studio: Workflows → `refund-request` → *Start run* → fill the form → *Start*. Open the run →
*Streams*: the progress bar advances to 1.0 and the list shows each event with its attempt.

```sh
sagat run stream <runId> --channel progress      # SSE; Ctrl-C at event 5, rerun: resumes at 6
```

Expected: offsets 1..10 exactly once across the reconnection; `end` after the last event once the run is
terminal; as `viewer` the `stage` field is withheld (internal), the `fraction` is shown (public).

## 3. Decide through the channel (US05)

The approval opens; the stand-in receives one message with order and amount (internal ≤ channel
ceiling) and a decision link `…/studio/approvals/decide#l=v1.…`. Open the link signed in as `bob`,
choose *Reject* without a comment (refused: `DECISION_INPUT_INVALID`), then with a comment (accepted).

Expected: one decision receipt with `via: SLACK`; the run continues at `notify-rejected`; the stand-in
receives `chat.update` with "Rejected" and the time only; `steps.approve.output.input.comment` is
readable by the next step; the link now shows the settled status only.

## 4. Call and wait (US02)

```sh
sagat run execute refund-request --input @small.json --wait 10s     # 200, completion TERMINAL
sagat run execute refund-request --input @slow.json --wait 2s       # 202, completion PENDING
sagat run outcome <runId> --wait 30s                                # 200 when it ends
```

Expected: the result is the feature-016 projection for `alice`; repeating the first command with the
same `--idempotency-key` returns the same run.

## 5. Public link (US04)

```sh
# setNamespaceQuota exists (feature 009); the resource public_links is new (default 0)
curl -fsS -X PUT "$SAGAT_API/v1/namespaces/$NS_ID/quotas/public_links" \
  -H "Authorization: Bearer $SAGAT_TOKEN" -H 'Content-Type: application/json' -d '{"limit": 5}'
sagat link create refund-intake --workflow refund-request --version 1 \
  --expose /orderId,/reason,/contactEmail --preset amount=0 --expires 7d --rate 10/min \
  --max-concurrent-runs 2
# prints https://forms.localhost:8081/f/<embedId>#t=<token> once
```

Open the URL in a private browser window (no account), submit, keep the receipt page open.

Expected: one run with `startedBy: system:public-link:<linkId>`; the receipt shows `RUNNING` then
`COMPLETED`, no run id; `sagat link list` shows one submission; audit holds `public_link.submitted`
without any address.

## Failure scenarios (each must be observed and recorded)

1. **Unusable links look the same**: `sagat link pause refund-intake` → the page says the link is not
   available; `resume`; `revoke` → the same page; an invented token → the same page; all `404`.
2. **Owner loses authority**: demote `alice` to `viewer`; a submission is refused like an unusable link;
   the namespace alert `PUBLIC_LINK_OWNER_UNAUTHORIZED` fires; audit `public_link.refused` with reason
   `OWNER` (collapsed on repeats).
3. **Rate and capacity**: eleven submissions in a minute → the eleventh is `429` with `Retry-After`; a
   third concurrent submission while two runs are live → `429`; no run admitted in either case.
4. **Wrong origin**: open `/f/<embedId>` and `/v1/public/form` on the Studio origin → `404`; frame the
   public page from an origin not listed on the link → the browser refuses it.
5. **Stale stream append**: stop the worker's heartbeat until the lease expires, then let it publish →
   `STALE_ATTEMPT`, nothing stored; the second attempt publishes with `attempt: 2` and a connected
   subscriber receives `superseded` for the first.
6. **Decision races expiry**: set `timeoutMs` to 5 s and click *Approve* at the deadline → exactly one
   outcome; the loser is an audited no-op; the link shows that outcome.
7. **Ineligible and cross-tenant**: open the decision link as `eve` (other tenant) → the same `404`; as
   the initiator → refusal `INITIATOR_CANNOT_DECIDE`; both audited without disclosure.
8. **Channel down**: stop the stand-in; a finished run's delivery retries with the same delivery id, goes
   `DEAD` at the limit and is redriven after the stand-in returns; the run itself is unaffected.
9. **Lost hint**: stop NATS during `sagat run execute … --wait 10s`; the answer still arrives (batched
   poll) within the wait.
10. **Restart during a wait**: kill the control plane during the wait; rerun the same command with the
   same key → the same run, no second run.

Record for each step the command, the HTTP status or CLI output, and the audit rows read through
`GET /v1/audit`, in `verification.md` (T026-066). A step that cannot be executed is reported as not
executed, with the reason.
