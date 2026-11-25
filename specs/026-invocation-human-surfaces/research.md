# Feature 026 — Baseline, external semantics, decisions, gaps and owner questions

Baseline investigation: 2026-10-26, worktree `sagat-octopus-parity-specs` at `70f09435` (release
0.2.0 on `develop`). Every path below was read or listed at that revision. Nothing in this feature is
implemented. Identifiers: decisions D1320–D1359, gaps G1320–G1359, owner questions O1320–O1359
(allocated range; numbers below 1000 belong to other branches).
Revised on 2026-10-26 after the independent architecture and security reviews: decisions were
revised in place where noted, D1357–D1359 and G1334–G1337 were added. Revised again for the
coordinator decisions of 2026-10-27 (C1–C22): D1640–D1641 come from the overflow range D1640–D1644.
Re-baselined on 2026-11-24 against `develop` at `2c94064f`: the stale items, the decisions D026-001 to D026-017, the
gaps G1338 to G1347 and the answers to the three owner questions are in
[Re-baseline against `develop`](#re-baseline-against-develop-2026-11-24) and [Owner questions](#owner-questions);
the outcomes of Architecture decisions 0061 and 0068 are in their status blocks, those of the requirements checklist
in `checklists/requirements.md`, and the build slices in `plan.md` ("Build slices (2026-11-24)").

## Repository evidence

| Area | What exists | Where |
|---|---|---|
| Principals | `Principal` = tenant + `Subject` + one built-in `Role`; 48 catalog permissions; reserved `system:` actors (`provisioning`, `operator`, `engine`, `gateway`) that `Subject::parse` refuses. No anonymous principal kind. | `crates/domain/src/identity.rs` |
| Run start | `startRun` (`POST /runs`, `202 Run`, required `Idempotency-Key`), `RunService::start` authorizes `run:start`; event- and cron-started runs record `system:engine`. | `specs/001-platform-foundation/contracts/openapi.yaml`, `crates/application/src/runs.rs`, `crates/scheduler/src/events.rs` |
| Waiting for a result | No server-side wait. SDKs poll `getRun` (`waitForCompletion`, 500 ms ± 20 %). | `sdks/SPEC.md` §3 |
| Run result | `runs.result_digest` from the `end` node (G21, owner decision 2026-10-16); disclosed by `discloseRunPayload` with `source: result` under feature 016. | `crates/infra-postgres/migrations/20261110000000_run_result.up.sql`, `crates/application/src/payload_disclosure.rs` |
| Live views | `GET /runs/{runId}/stream` (graph changes, payload-free, `Last-Event-ID`), bounded by `StreamLimits` (2,000 per process, 200 per tenant, 20 per subject, poll 0.5–2 s, ping 15 s, 15-minute lifetime, membership recheck 30 s). | `crates/api-http/src/streams.rs`, ADR-0016 |
| Start UI | *Start run…* takes a raw JSON object up to 256 KiB; one idempotency key per dialog; no schema-driven form. | `frontend/apps/studio/src/features/runs/StartRun.tsx` |
| Studio security | CSP with `frame-ancestors 'none'`; token in memory only. | `crates/api-http/src/studio.rs`, ADR-0016 |
| JSON Schema | `sagat-infra-schema` (jsonschema 0.58) and the feature-016 classification walker. | `crates/infra-schema/` |
| IR | `inputSchema`/`outputSchema` free-form objects; `approval` node (`policy`, `timeoutMs`, `onApprove`, `onReject`, `onExpire`, `onError`); `approvalPolicy` (≤ 32 subjects, roles `admin`/`developer`/`operator`, `excludeInitiator` default true); `state.queries`/`state.updates` with `disclosure: declared-business-result-v1`; `waitEvent.outputSchema`. | `specs/001-platform-foundation/contracts/workflow.schema.json` |
| Approvals | `listRunApprovals`, `decideWorkflowApproval` (`decisionKey`, `APPROVE`/`REJECT`, closed `reasonCode`); the opening commits request, routes, audit and an outbox id together; the store re-reads membership before each decision. No notification, no decision UI in the Studio. | OpenAPI, `crates/application/src/approval_progress.rs`, `frontend/apps/studio/src/api/schema.d.ts` |
| Outbound | `OutboundDestination::{Webhook, Broker}`; kinds `run.finished`, `ops.alert.fired`, `ops.alert.resolved`; destination discriminator `WEBHOOK`/`NATS`/`KAFKA`/`SQS` on the historical `webhook_subscriptions` tables. | `crates/domain/src/outbound.rs`, `crates/infra-postgres/migrations/20261021000000_outbound_brokers.up.sql`, ADR-0025 |
| Rate and quotas | Local token buckets per key (`crates/api-http/src/limits.rs`); durable namespace buckets `api_requests_per_second` and `events_per_second`; count quotas where zero refuses. | `crates/domain/src/quota.rs`, ADR-0027 |
| Worker protocol | rpcs `RegisterWorker`, `PollTasks`, `StreamTasks`, `Heartbeat`, `CompleteStep`, `FailStep`, `ResolveSecrets`, `ResolveDatabase`, `WriteBlob`, `ReadBlob`; minor 1.5 scoped identity, 1.6 managed database (D600). | `specs/001-platform-foundation/contracts/worker.proto`, `docs/architecture/worker-runtime-protocol.md` |
| Errors | REST codes `BAD_REQUEST … INTERNAL`; worker codes `STALE_ATTEMPT`, `IDEMPOTENCY_KEY_REUSED`, `NOT_BOUND`, `SECRET_NOT_USABLE`, `UNSUPPORTED_PROTOCOL`. | `sdks/error-catalog.json` |
| Tests | Per-feature dirs under `tests/{contract,integration,fault,e2e}/<slug>/`, targets in `tests/Cargo.toml`, the cross-tenant battery, the five-language suite with 60 scenarios. | `tests/contract/security-multitenancy/rest/battery.rs`, `tests/contract/sdk-conformance/` |

## External semantics consulted (2026-10-26)

- **Temporal Workflow Streams** — [concepts](https://docs.temporal.io/workflow-streams),
  [Python guide](https://docs.temporal.io/develop/python/workflows/workflow-streams),
  [Replay 2026 announcement](https://temporal.io/blog/replay-2026-product-announcements). Public
  Preview; built on Signal and Update; the log lives in workflow state; a monotonically increasing
  global offset; publisher id + sequence deduplicate retried batches; "a retried Activity attempt is a
  new publisher", so subscribers may see partial output of failed attempts; a batch dropped after
  `max_retry_duration` is at-most-once; the log rolls over with Continue-As-New carrying its offset;
  poll responses about 1 MB. Adopted: offsets, publisher sequences, attempt-visible partial output.
  Not adopted: state-carried log, at-most-once drop, offset carried across Continue-As-New (D1328–D1331).
- **Windmill approvals** — [suspend and approval steps](https://www.windmill.dev/docs/flows/flow_approval):
  "secret" resume/cancel URLs usable by their recipients, optional "Require approvers to be logged
  in", group restriction, self-approval prevention, an approval form whose values later steps read,
  Slack/Teams helpers, e-mail by sending the URLs. Adopted: the form, notifications, links. Not
  adopted: bearer approval without sign-in (ADR-0061, O1320).
- **Windmill webhooks** — [webhooks](https://www.windmill.dev/docs/core_concepts/webhooks): synchronous
  `run_wait_result` with an instance-level maximum timeout, asynchronous endpoint returning a job id;
  OpenAPI 3.1 generation for HTTP routes and webhooks. **Windmill auto-generated UIs** —
  [auto-generated UIs](https://www.windmill.dev/docs/core_concepts/auto_generated_uis): forms from the
  JSON Schema of arguments, with title/placeholder/description customization. Adopted: bounded wait
  with handle continuation, schema-driven forms with a small hint vocabulary, per-workflow OpenAPI.
- **Slack** — [verifying requests](https://docs.slack.dev/authentication/verifying-requests-from-slack)
  (`X-Slack-Signature` `v0=` HMAC-SHA256 over `v0:{timestamp}:{body}`, five-minute window, constant
  time), [`chat.postMessage`](https://docs.slack.dev/reference/methods/chat.postMessage) (`chat:write`,
  about one message per second per channel, returns `ts`, no documented idempotency key) and
  [`chat.update`](https://docs.slack.dev/reference/methods/chat.update) (needs `channel` and `ts`).
- **Microsoft Teams** — [Workflows incoming webhooks](https://support.microsoft.com/en-us/office/create-incoming-webhooks-with-workflows-for-microsoft-teams-8ae491c7-0394-4861-ba59-055e33f75498)
  (Adaptive Card and Message Card payloads) and the Message Center notice MC1181996 as mirrored at
  [mc.merill.net](https://mc.merill.net/message/MC1181996) (Office 365 Connectors disabled between
  18 and 22 May 2026; Workflows webhooks lack interactive MessageCard buttons). The mirror is a
  third-party copy; the date must be rechecked against Microsoft's admin center at implementation.

## Decisions

- **D1320 — The server computes the form model.** One Rust walker in `crates/infra-schema` (next to the
  classification walker of feature 016) turns a carrier schema into the form model; the Studio, the
  public page and the SDKs render it. A TypeScript interpretation of JSON Schema would drift from the
  server's validation (ADR-0016's reasoning).
- **D1321 — No partial forms.** A supported keyword subset (FR-026-002); an unsupported optional field
  is shown as unsupported and omitted from submission; an unsupported required field makes the form
  `UNAVAILABLE`. A form that silently drops a required constraint would admit inputs the author did
  not intend or fail late.
- **D1322 — `x-sagat-form` is a closed annotation.** Three keys (`widget`, `order`, `placeholder`),
  additive inside IR v1 (ADR-0032), validated at publication. Standard `title`, `description`, `enum`
  and `default` carry the rest. Rejected: a separate UI document per workflow (a second artifact to
  version and diff).
- **D1323 — No URL prefill and no browser drafts.** Values in URLs reach logs and history (ADR-0031
  keeps arguments out of query strings); drafts in browser storage would retain `pii`. Link presets are
  server-side and classified.
- **D1324 — `executeRun` is a new operation.** A `wait` parameter on `startRun` would make one
  operation answer `200` or `202` with different bodies and change what existing SDK typings promise.
  `getRunOutcome` gives the continuation after `202` and a server-side replacement for client polling.
- **D1325 — Waiters wake on hints and on one batched poll.** The existing `sagat.v1.run.finished`
  hint and feature 022's `run.result_returned` hint (coordinator decision C22) wake waiters quickly;
  every control-plane replica subscribes to both without a queue group, so the replica holding the
  waiter always receives them. One query per process every 500 ms reads the status and the committed
  early result of all waited runs (`WHERE (tenant_id, run_id) IN (…)`), so a lost hint costs at most one
  interval and the database cost does not grow with one query per waiter. Rejected: PostgreSQL
  `LISTEN/NOTIFY` (a second notification path beside NATS, connection-bound) and per-waiter polling.
  (Revised after the architecture review, finding 6, and decision C22.)
- **D1326 — Wait ceilings 30 s default, 60 s hard.** Common proxy and load-balancer idle timeouts sit
  around 60 s; a longer synchronous wait would fail in transit more often than it helps. Longer waits
  are a loop of `getRunOutcome`. These are design values to measure (NFR-026-006).
- **D1327 — Outcome disclosure reuses feature 016.** The result in `RunOutcome` is the same projection
  as `discloseRunPayload` with `source: result`; no new policy for authenticated callers.
- **D1328 — Streams are a fenced log beside the journal, with inline data** (ADR-0068). An event
  changes no run state, run version or expression input; its data is stored inline in its own row
  (never the shared `payloads` table, which is never deleted from), charged under ADR-0027 and released
  when the row is deleted. Putting events in the journal would grow every run's history with high-rate
  data, make `replay equals projection` cover non-state data and make the graph stream carry values.
  The step output remains the only partial result a workflow can depend on. (Revised after the
  architecture review, findings 2 and 3.)
- **D1329 — Streams are durable only.** A resume cursor needs stored events; a NATS-only ephemeral mode
  would lose events on reconnection. Temporal's at-most-once drop after a retry deadline is replaced by
  a bounded buffer in the SDK that reports refusal to the step code.
- **D1330 — One stream per run segment.** Continue-As-New starts the successor's log at offset 1;
  consumers follow `successorRunId`. Carrying an offset across segments would couple two runs'
  retention and transactions (G1329).
- **D1331 — Superseded attempts stay visible and live subscribers are told.** Each event carries node
  instance, attempt and generation. The feed derives control events from committed attempt state at
  read time: an `attempts` snapshot on open and resume, `superseded` when a newer attempt of a node
  instance with events starts, `settled` when its step settles. They carry no offset, so a resumed
  subscriber relies on the snapshot. No platform marker is written in the claim transaction. The
  client rule (discard events of older attempts of the same node instance) is part of the SDK
  conformance contract. (Revised after the architecture review, finding 4.)
- **D1332 — A new additive worker-protocol minor carries `AppendStreamEvents`.** Proposed as 1.7 after
  the current 1.6; drafts 019 and 025 may also need a minor, so the coordinator assigns the final
  number (G1333). Older workers keep their tasks (FR-026-050).
- **D1333 — Append lock order: step lease row (share) → run stream head (update).** An append never
  locks the run row, so it does not contend with the scheduler; completion holds the step row for
  update, so an append and a completion are ordered and the later append sees a stale lease.
- **D1334 — A public link pins one version.** Following the active version would expose fields of a
  newer schema without review; moving a link is an audited edit.
- **D1335 — Link tokens travel in the fragment; the path carries a non-secret embed id.**
  `https://<public-forms-origin>/f/<embedId>#t=<token>` keeps the token out of server and proxy access
  logs and `Referer`; the page sends it as `X-Octopus-Link-Token`; the embed id lets the server compute
  the page's `frame-ancestors` from that one link (ADR-0061 decisions 3 and 4; Blocker 1 of the security review).
- **D1336 — Public links are enabled by quota and run in their own pool.** The count quota
  `public_links` defaults to 0; no separate namespace switch. Each link has a required cap on live
  runs, and the namespace counts live public-link runs in `public_link_active_runs`, a sub-pool of
  `active_runs` counted exactly under the feature-009 lock (D42 of feature 009), so anonymous traffic
  cannot take the capacity members need (security review S7). Counts are of live runs with origin
  `PUBLIC_LINK`; nothing has to be released.
- **D1337 — On behalf of the link owner, rechecked every time, in the slots of ADR-0050 decision 7**
  (ADR-0061, decision 2). The link row is locked in slot 1 (with the read-only 029 check), the receipt
  in slot 2, the owner's membership, 029 fences and namespace row in slot 4a, the public-link pool is
  counted with slot 6's active-run admission, usage rows come last (8b). The durable rate buckets are
  debited in their own committed decision before the admission transaction, as ADR-0027 does for API
  rates, so no admission lock is held while an anonymous flood is refused. When feature 029 is enabled
  its evaluator also decides, at each submission, whether the owner may still see and start the pinned
  workflow (coordinator decision C10). (Aligned on 2026-10-27; the earlier text took the membership and
  the buckets right after the link row and was listed in 018's G1605.)
- **D1338 — No source address is stored** (ADR-0061, decision 8). The per-process guard keys on the
  connection's peer address; a forwarded header is used only when the operator lists trusted proxy
  networks, and then the right-most untrusted hop counts.
- **D1339 — Anonymous callers see only declared `public` result fields** (ADR-0061, decision 5). A
  model-written value (feature 025) has at least the highest level of everything in its transcript
  (feature 025's model-output floor, Blocker 4 of the security review), so an author annotation cannot relabel it
  `public`. The same rule serves a feature-021 synchronous route, which must declare
  `public-fields-v1` to return anything.
- **D1340 — Notifications never block an approval.** Notifications are a convenience; the approval's
  durability, deadline and decision path do not depend on them. A skipped target is audited and
  alerted.
- **D1341 — Teams uses Workflows webhooks.** Office 365 Connectors were retired in May 2026; Workflows
  webhooks accept Adaptive Cards but no interactive buttons and give no message reference to edit, so
  settlement is a follow-up card (G1324 for in-chat decisions).
- **D1342 — E-mail goes through the deployment SMTP relay; replies decide nothing.** Inbound mail is
  spoofable and has no reliable identity.
- **D1343 — Notification channels are ADR-0025 destinations with their own credentials.** `SLACK`,
  `TEAMS` and `EMAIL` join the destination discriminator; `approval.requested` and `approval.settled`
  join the outbound kinds. One delivery worker keeps rate, lease, fence, retry, `DEAD` and redrive
  identical for every destination. Messages are fixed-shape platform deliveries, so the engine sends them
  through the guarded egress client holding only that destination's credential (the placement rule of
  feature 021, ADR-0055 rule (b), coordinator decision C3); channels keep their own credentials, like
  webhook subscriptions, and do not reuse feature-021 resources (an earlier draft sentence suggesting
  that was removed).
- **D1344 — Summary values live with the approval, not in the outbox.** ADR-0027 keeps
  `webhook_deliveries.payload` a closed, protected schema of identifiers. The evaluated summary is
  stored as run content (charged) referenced by the approval request, and the delivery worker renders
  it at send time under the channel ceiling.
- **D1345 — Channel ceiling `public` by default; summary levels follow their sources; `pii` leaves
  only under the uniform egress rule.** Chat workspaces and mail lists include people outside the
  namespace. `internal` needs `event:manage` and `payload:read-internal`. `pii` follows the rule feature
  020 proposes in O1081 for every destination (coordinator decision C9): the ceiling is set by an
  administrator holding the reveal grant, the grant is rechecked at each delivery that carries a `pii`
  field, its revocation pauses the channel, each such delivery is audited, `secret` never leaves
  (O1322 asks only whether channels opt in at all). Each summary field's annotated level must be at
  least the effective level of every location its expression reads (undeclared ⇒ `pii`; model-written ⇒
  feature 025's floor), refused at publication otherwise (security review S6).
- **D1346 — Decision links are signed opaque pointers** (ADR-0061, decisions 3 and 6). Validity is
  infrastructure time (creation + min(timeout, 30 days)) and, separately, a `PENDING` approval; the
  approval deadline is workflow time and is never compared with it (feature 028's time-class catalog,
  FR-028-003, coordinator decision C13). The server derives `via` from the verified link. Across the
  OIDC redirect the page keeps the token in `sessionStorage` only and removes it on return; the link
  grants nothing, so this exposes no authority. When feature 029 is enabled its evaluator decides
  whether the caller may see the run's workflow when the link is resolved (C10). Decision-link and
  receipt keys are entries of feature 032's key inventory (C11).
- **D1347 — Recipients are channel-level.** One decision link per delivery; per-person routing waits
  for a directory (G1326).
- **D1348 — The approval form schema is a feature-016 carrier.** Its input is part of the decision
  fingerprint when present, and the fingerprint of a request without input stays byte-identical to
  today's, so a decision retried across the upgrade replays instead of conflicting (architecture review,
  finding 5; compatibility test T026-040). The input becomes `steps.<node>.output.input`; this extends
  FR-016-002 additively (task T026-067).
- **D1349 — The per-workflow OpenAPI specializes the generic operations.** `const` workflow and version
  identifiers on `startRun`/`executeRun`; no per-workflow path that would need routing, authorization
  and quotas of its own. Generated on request, cached by `ETag`, never stored.
- **D1350 — One Slack app model: a tenant-owned app bound per namespace** (ADR-0061, decision 7).
  The tenant registers its own Slack app (bot token, signing secret); the platform stores the team id
  returned by `auth.test`; an interactive channel is unique by `(team, channel)` across tenants;
  requests arrive at the binding's own URL; the tenant of a decision comes only from the decision-link
  row. Identity links use single-use 128-bit codes consumed atomically, a team bound by the code's
  tenant, per-team rate limits and confirmation in the signed-in Studio. Rejected: one
  platform-distributed app with a deployment-wide signing secret (it needs a deployment OAuth client
  and still cannot tie a team to one tenant without the same install binding). (Blocker 1 of the
  architecture review; Blocker 2 and finding 5 of the security review.)
- **D1351 — Two new permissions only.** `link:read`, `link:manage`; channels reuse `event:manage`
  (outbound subscriptions), forms and OpenAPI `definition:read`, invocation `run:start`/`run:read`,
  decisions `approval:decide`.
- **D1352 — Encoded (031) locations are never form, link or summary inputs** and are opaque in
  streams. This feature registers its positions in feature 031's encoded-field registry (forms and
  public submissions as read positions and write paths, stream channel schemas and the approval form as
  carriers, approval summaries as expression positions; coordinator decision C7) and uses only 031's
  codes: `ENCODING_READ_BY_FORM` refuses declared artefacts at publication and a public link that would
  expose or preset such a field, and is also the reason the derived start form reports at request time
  (an earlier draft code `ENCRYPTED_FIELD` was removed).
- **D1353 — The Slack message reference is recorded after acknowledgement.** `ts` is stored on the
  delivery to edit at settlement; a lost acknowledgement re-sends and produces a second message (G1322).
- **D1354 — The public page is a second minimal build entry on its own origin.** Served under
  `/f/<embedId>` on the public-forms origin with its own CSP, no shell, no sign-in code, no cookie;
  never on the Studio origin (Blocker 1 of the security review).
- **D1355 — `sagat.` channel names are reserved for platform-defined channels.** Feature 025 declares
  `stream.tokens: true` on its agent node rather than an author channel; the reserved prefix lets such a
  feature fix the schema, classification (transcript content) and budget while every append, read and
  disclosure rule of this feature applies unchanged. Reserved channels count in the reserved pool, never
  in the author pool (D1357).
- **D1356 — `superseded` is computed at read.** Whether a later attempt exists is known from the step's
  current attempt; storing a flag would require updating immutable events. With the live control events
  of D1331, feature 025's acceptance ("superseded deltas marked") is met without a write in the claim
  path.
- **D1357 — Stream budgets are per channel, per pool and per run.** Author channels declare a budget
  (default 5,000 events / 2 MiB, hard 100,000 / 32 MiB per channel) inside the author pool (default
  10,000 / 4 MiB, hard 100,000 / 64 MiB per run). Reserved `sagat.` channels take budgets fixed by their
  owning feature inside the reserved pool (default 65,536 events / 32 MiB, hard 131,072 / 64 MiB per
  run). Every run is under a hard ceiling of 262,144 events and 128 MiB over both pools. A multi-turn
  agent (feature 025, up to 4,096 events per model call) therefore streams 16 calls within the default
  reserved pool without touching author channels; feature 025 aligns its numbers to these
  (coordinator decision 4; architecture review of 025, finding 6). These are design bounds to measure.
- **D1358 — Receipt tokens are derived, not stored.** `receiptToken = base64url(HMAC-SHA-256(K[v],
  "octopus-receipt-v1\n" ‖ linkId ‖ submissionKeyHash))`, where the 128-bit-minimum submission key is
  generated by the page; the row stores the token's SHA-256 and `v`. A replay recomputes the same token,
  so the receipt survives a lost response without the server keeping a usable secret; the key set is
  the one of decision links (`SAGAT_LINK_KEYS`), separated by label, and old versions are kept while
  receipts live (security review minor item).
- **D1359 — The `/v1/public/*` routes live only on the public-forms origin.** Host-based routing in the
  control plane serves the page and its API on that origin, so the page needs no CORS and the Studio
  origin never serves anonymous routes; the control plane refuses to start when the two origins are
  equal; the host may be shared with the feature-027 apps origin. ADR-0062 (feature 027) holds the
  combined refinement of ADR-0016 for the Studio, apps and public origins and their CSP (coordinator
  decisions 2 and C15); this feature cites it.
- **D1640 — The stream cursor carries the cell epoch.** SSE ids and resume cursors are
  `<epoch>.<offset>` with feature 032's cell epoch (0 without 032). After a failover the promoted cell
  may have lost the tail of a run's log inside the RPO and will reuse offsets; a cursor of another epoch
  therefore gets `event: reset` and the reader starts again from the beginning with the `attempts`
  snapshot instead of silently mixing two histories (coordinator decision C2).
- **D1641 — Registry entries of this feature.** Feature 018's reset-rule registry: stream events are
  not copied by a reset (the new run starts an empty log); the decision input on `approval.decided` is
  copied with the event (`COPY`); approval notifications and decision links are side rows of the source
  run's request and are not copied — when a reset reopens an approval, that opening is a new opening of
  this feature (`REMATERIALIZE` through the opening path). Feature 020's content-store registry: stream
  events (`run_stream_events`, purge after run end + retention, erasure deletes at once), submission
  receipts (no values), link presets (deleted with the link). Feature 031's encoded-field registry: see
  D1352. Feature 016's carrier list: see the spec's "Amendment to feature 016" (coordinator decisions C1,
  C6, C7, C8).

## Gaps

| Id | Gap | Consequence |
|---|---|---|
| G1320 | No CAPTCHA or bot protection on public links; without stored addresses, forensics rely on counts and refusal audit. | Abuse limited by rates, caps and expiry only. |
| G1321 | Forms cannot be embedded authenticated in third-party pages (the Studio refuses framing, tokens live in memory). | Only public links are embeddable. |
| G1322 | Slack and Teams sends have no documented idempotency key. | A lost acknowledgement duplicates a message. |
| G1323 | Waiters and stream subscribers depend on server polling (extends G143). | Database cost grows with open streams; push belongs to scale work. |
| G1324 | Teams in-chat decisions need a registered bot (Bot Framework). | Teams approvers use the decision page. |
| G1325 | SPF/DKIM/DMARC and bounce handling are the deployment's. | Mail may be filtered. |
| G1326 | No directory from subject to e-mail or chat user. | No per-person routing. |
| G1327 | `EARLY_RETURN` depends on feature 022. | Reserved value until then. |
| G1328 | Synchronous custom routes depend on feature 021. | Contract hook only. |
| G1329 | No chain-level stream offset. | Consumers follow segments. |
| G1330 | Data sent to chat or e-mail leaves platform retention and erasure (LGPD). | Documented in the runbook; channel ceilings limit it. |
| G1331 | Wake-up latency, stream latency and append throughput are unmeasured. | No latency promise. |
| G1332 | Scripts stream only through feature 023's reserved `stream.append` host call; a runtime without that call cannot stream. | Feature 023 delivers it per language behind a capability gate (C17). |
| G1333 | The worker-protocol minor may collide with drafts 019/025. | Coordinator assigns the number. |
| G1334 | Feature 022's `run.result_returned` hint can be lost like any hint. | Waiters see the early result at the next batched poll (one interval). |
| G1335 | No platform-distributed Slack app: each tenant creates and registers its own app. | Setup effort per tenant; documented in the runbook. |
| G1336 | FR-016-002 lists the carriers "exactly"; stream channel schemas, approval summaries and approval forms (and 025's agent carriers) extend it. | Feature 016's text must be amended when this is implemented (T026-067). |
| G1337 | Link and receipt validity rely on the time classes of the ADR-0063 draft (feature 028). | Until 028 lands, both use the one database clock, as today. |

## Owner questions

- **O1320 — Should a possession-only decision mode ever exist** (decide by holding the link, without
  signing in, for external approvers without accounts)? *Recommended default:* no. Every decision needs
  a signed-in eligible member (ADR-0061). If wanted later, it needs its own ADR: a per-approval opt-in,
  single-use per-recipient links, a policy naming only that mode, and an audit that names the link.
- **O1321 — Should public links be enabled by default anywhere?** *Recommended default:* no; the
  `public_links` quota is 0 in every namespace, including development, and an administrator raises it
  per namespace (D1336).
- **O1322 — May channels carry `pii` summary fields at all?** *Recommended default:* yes, but only under
  the uniform `pii` egress rule that feature 020 asks the owner to accept for every destination
  (O1081): an administrator holding the namespace reveal grant sets the channel's ceiling, the grant is
  rechecked at each delivery, its revocation pauses the channel, each delivery is audited; default
  ceiling `public`; `secret` never (D1345). If the owner declines O1081, channels stay at `internal` at
  most.

### Answers of 2026-11-24 (delegate (OD-2026-11-24-4))

The owner delegated the open questions of release 0.3.0 (Owner decisions 2026-11-20-6 and 2026-11-24-4). Each answer
below weighed every option and took the one with the most architectural and business advantage, not the simplest.

- **O1320 — decided: no possession-only decision in release 0.3.0; delegate (OD-2026-11-24-4).** Options weighed:
  (a) never, every decision by a signed-in eligible member; (b) a per-approval opt-in mode now, with single-use
  per-recipient links; (c) Windmill's bearer default. (c) breaks Requirement 014-008 (eligibility at commit), the
  initiator exclusion and an audit that names a person, which are what make the platform's approvals acceptable to
  regulated buyers (the business case of surpassing Windmill is the audit, not the convenience). (b) adds a second
  decision authority, its own key handling and its own revocation story to a release that already carries 21 new
  features, for a need (external approvers without an account) that a later guest principal of federated login
  (feature 013) serves better, since that principal is a person with an identity. (a) keeps Architecture decision 0061
  decision 6 true with no exception to test. Consequence: Gap 1347 (external approvers need an account). A later mode
  still needs its own architecture decision, as the recommendation said.
- **O1321 — decided: public links are off in every namespace by default, development included; delegate
  (OD-2026-11-24-4).** Options weighed: (a) quota `public_links` 0 everywhere; (b) a non-zero default in the
  development compose profile; (c) a deployment switch plus the quota. (b) makes the anonymous surface reachable on
  any machine that runs the development profile with a published port, and a demonstration that needs it raises the
  quota in one command (the quickstart does so). (c) adds a second switch that says the same thing as the quota and
  can disagree with it. (a) is secure by default, one control, already audited through ADR-0027 administration.
- **O1322 — decided: yes, a channel may carry `pii` summary fields, only under the uniform `pii` egress rule of
  Owner question 1081 (feature 020); delegate (OD-2026-11-24-4).** Options weighed: (a) never above `internal`;
  (b) under the uniform rule; (c) under a channel-specific rule. (c) creates a second egress policy for one destination
  kind, which the coordinator decision C9 exists to prevent. (a) forces teams who approve on personal data (a refund to
  a named customer, a credit decision) to open the Studio for every approval, which is the friction feature 026 exists
  to remove, and it is the weaker product against Windmill, whose messages carry whatever the author puts in them.
  (b) keeps one rule, a named accountable administrator holding the reveal grant, a recheck at every delivery, an audit
  per delivery and `secret` never. Owner question 1081 itself is feature 020's and is not answered here; until it is
  answered, the `pii` ceiling stays unbuildable (Task 026-077 of `tasks.md` names the gate) and channels stop at
  `internal`, which is the fallback the original recommendation stated. Gap 1346.

Note (2026-11-15, from feature 041): the owner gave worker-protocol minor 1.7 to feature 041 (step logs, OD-2026-11-15-8).
Where this feature names 1.7 for its own addition, it takes the next free minor of the register in `sdks/SPEC.md` §9 when
its contract merges.

## Re-baseline against `develop` (2026-11-24)

The drafts were written on 2026-10-26 at `70f09435`. They were read again on 2026-11-24 against `develop` at
`2c94064f` (worktree `sagat-wt-windmill-parity`), and against the open branches that change what this feature builds
on: `feature/022-throughput-control`, `feature/041-step-logs-server`, `feature/007-astryx-design-system` (pull request
267) and `feature/044-workflow-as-code` (no content yet). Every path below was read at that revision. Nothing of feature
026 is built.

Identifiers. The bare decision range of this feature (D1320 to D1359, overflow D1640 to D1644) is spent but for
D1642 to D1644, which stay unused; from this re-baseline on, new decisions take the feature-scoped form
`D026-NNN`, as features 041 and 043 do. New gaps continue the bare range (G1338 upward). No new owner question is
open: every question is decided under the delegation. In prose they are written in words (Decision 026-001,
Gap 1338), as `docs/governance/identifier-scheme.md` requires.

### What changed on `develop`, and what this feature had wrong

Each stale item names the evidence and the decision that fixes it. The fixes are written into `spec.md`, `plan.md`,
`tasks.md`, `data-model.md` and the contracts in the same change.

| # | Stale item | Evidence on `develop` | Fixed by |
|---|---|---|---|
| 1 | Server validation was described as a list of JSON pointers and a reason of this feature (`FORM_INPUT_INVALID`). Every run start is now checked by the run input gate of design 043, which answers the first violation only: `422 VALIDATION_FAILED`, `details.reason` `RUN_INPUT_INVALID` or `RUN_INPUT_SCHEMA_UNUSABLE`, one `details.field` and one `details.rule`, never a value | `crates/application/src/run_input.rs` (`RunInputGate`, `Creator`, `FieldPolicy`), `specs/001-platform-foundation/contracts/openapi.yaml` (`startRun` description, `Error.details.rule` enum), Architecture decisions 0090 and 0092 | Decision 026-017 |
| 2 | `x-sagat-form` was an annotation inside `inputSchema`. The supported subset of design 043 admits one `x-` keyword only, `x-sagat-classification`; any other makes the schema unusable, so a version with a form hint would refuse every start | `specs/043-run-input-validation/contracts/input-schema-subset.md` ("the only `x-` keyword the platform reads"), the `inputSchema` description of `workflow.schema.json` | Decision 026-001 |
| 3 | The form keyword subset was this feature's own. Design 043 fixed the subset every `inputSchema` must stay inside (combinators, `pattern`, `multipleOf`, type lists, `minProperties`, `maxProperties`); a form must render a subset of it and nothing outside it | the same contract | Decision 026-002 |
| 4 | Approval input, stream events and link presets were to be validated with the `jsonschema` crate. Design 043 showed that its cost cannot be bounded beforehand and built a platform validator with a work counter | `crates/infra-schema/src/run_input/`, design 043 Technical Plan 4, Owner decision 2026-11-14-14 | Decision 026-003 |
| 5 | `runs.origin` was to gain `PUBLIC_LINK`. That column is the private-run marker of feature 017: a trigger refuses a non-null origin on a workflow that is not internal | `20261125000000_database_ad_hoc.up.sql` (`runs_private_definition_guard`, CHECK `AD_HOC_STEP`, `AD_HOC_WORKFLOW`, `CONNECTION_PROBE`) | Decision 026-004 |
| 6 | Live public-link runs were to be counted by exact queries under locks. Active runs are now read from counters the database keeps with the runs, because counting cost a start one index entry per active run | `20261017000100_active_run_counts.up.sql`, `ensure_room_for_run` in `crates/infra-postgres/src/quotas.rs` | Decision 026-004 |
| 7 | The waiter was "woken by the existing `run.finished` hint". The hint is published (outbox relay to `sagat.v1.run.finished.<tenant>`), but no process subscribes to NATS: `crates/infra-nats` is a JetStream publisher only | `crates/infra-nats/src/lib.rs`, `crates/scheduler/src/outbox.rs` (subject format) | Decision 026-005 |
| 8 | The stream append locked the step row `FOR SHARE` and the stream tables had foreign keys to `runs`. The hot path now has a statement budget tested as equalities, a heartbeat updates the step row, and feature 041 forbids its appends to take or wait for a lock on `runs`, `steps` or `run_events` and to take the tenant usage row before commit | `tests/integration/claim-transition-efficiency/work_budget.rs`, design 041 Requirement 041-008 and Problem items 3 and 4 | Decisions 026-006 and 026-007 |
| 9 | The stream purge instant was written in the run's terminal transaction, one more write on the transition design 038 made cheaper | design 038, the work budget above | Decision 026-008 |
| 10 | Five SDKs. There are six: the Rust SDK (feature 034, Architecture decision 0070) is in the conformance harness | `tests/contract/sdk-conformance/harness.rs` (`LANGUAGES`, six entries), `sdks/rust/` | spec Requirement 026-048, `contracts/sdk-conformance.md` |
| 11 | Worker-protocol minor 1.7. It is feature 041's (`AppendStepLogs`); drafts 020, 021, 025 and 026 each take the next free minor when their contract merges | `sdks/SPEC.md` register of minors | `contracts/worker-protocol.md` |
| 12 | `executeRun` took "the `StartRunRequest` body" without saying what the new fields of feature 018 do: a business identifier with `USE_EXISTING` answers `200` with the holder, `startDelayMs`, `inputBlobRef`; Signal-with-Start and Update-with-Start exist, and Update-with-Start already answers its Update result synchronously; chains have their own Query and Update paths | `startRun`, `signalWithStart`, `updateWithStart`, `queryExecutionChain`, `updateExecutionChain` in the canonical OpenAPI | Decision 026-009 |
| 13 | Synchronous custom routes were "served by the same waiter" on a run. A feature-021 route admits an event into the inbox and the engine's dispatcher creates the run later, so at the moment of the request there is no run to wait on | `specs/021-integration-resources-triggers/contracts/connector-kinds.md` (route `response` reserved, `202 EventAccepted`), `event_deliveries.run_id` (`20261012000000_events.up.sql`), design 043 Technical Plan 1 row 9 | Decision 026-010 |
| 14 | Feature 022 was cited as if present. Its throughput slice (Architecture decisions 0088 and 0089) is on `feature/022-throughput-control`, not on `develop`; early return (its User story 022-04) is not built anywhere; every run creator must call its permit port (its Task 022-064) | `specs/022-flow-control-step-policies/plan.md` "Run creators" | Decision 026-017, Gap 1338 |
| 15 | Step logs (feature 041) were not considered. They are a second per-attempt record beside the journal, with their own rpc, permission (`run:logs:read`), storage class and routes (`/runs/{runId}/logs`, `/logs/streams`, `/logs/stream`) | `docs/designs/041-step-logs.md`, Architecture decisions 0084 and 0085 | Decision 026-011 |
| 16 | Names. "Stream" now also means the log streams of feature 041, the Redis Streams destination of feature 033 (`REDIS` in the destination CHECK) and, avoided on purpose, the operator chains of design 042 | `docs/designs/042-stream-operators.md` (a word on names), `20261224000000_redis_callback_destination.up.sql` | Decision 026-012 |
| 17 | "Capability gates" were server flags. A feature that every control plane or engine must honour is switched on through the instance registry (`schedules-v1`, `run-input-validation-v1`) | `crates/application/src/instances.rs` (`Capability`), Architecture decision 0082 | Decision 026-013 |
| 18 | Migrations were named `<next>_*`. The newest on `develop` is `20270119000000_batch_purge`; from `20270118000000` every CHECK is added `NOT VALID` and validated by a later migration, and a static test enforces it | `tests/integration/platform-foundation/migrations.rs` (`a_constraint_added_not_valid_is_validated_by_a_later_migration`) | `plan.md` Migration and compatibility, `data-model.md` |
| 19 | The OpenAPI fragment said "no existing operation changes", yet `ApprovalDecisionRequest` gains `input` and `linkToken`, which changes `decideWorkflowApproval`. Two tests pin existing operations | `tests/contract/payload-classification/openapi.rs`, `tests/contract/iceberg-writes/openapi.rs` | Decision 026-014 |
| 20 | Studio paths. *Start run* now carries a business identifier and a start delay; the Astryx design system migration rewrites every feature directory; `features/integrations/` is the broker connector page that feature 021 extends | `frontend/apps/studio/src/features/runs/StartRun.tsx`, `features/integrations/README.md`, branch `feature/007-astryx-design-system` | Decision 026-015 |
| 21 | "Single region (ADR-0009)" and a declarative-only IR. The constitution amendment 5.0.0 in progress makes it one writable region per cell (feature 032) and adds a procedural mode, which enters release 0.3.0 (Owner decisions 2026-11-24-2 and 2026-11-24-3) | `docs/owner-decisions.md` 2026-11-24 | Decision 026-016 |
| 22 | `contracts/sdk-conformance.md` said `decideApproval` gains `input` and `via`; a client never sends `via` (Requirement 026-032) | this feature's own contracts | `contracts/sdk-conformance.md` |
| 23 | The reserved actor `system:public-link:<id>` was new. Feature 018 has since added `system:schedule:<id>` the same way, which is the precedent to follow; the catalog still has no `link:` permission and no principal other than `Principal` | `crates/domain/src/identity.rs` | none needed; `plan.md` cites it |
| 24 | Audit actions were listed without a mapping. Design 040 (in release 0.3.0) requires every audit action to have one OCSF mapping that a test enforces | `docs/designs/040-ocsf-event-standard.md` Requirement 040-001 | Gap 1345 |

### Decisions of 2026-11-24 (delegate (OD-2026-11-24-4))

- **D026-001 — Form hints leave the schema.** Options weighed: (a) keep `x-sagat-form` in `inputSchema` and widen the
  subset of design 043; (b) hints beside the schema, keyed by the field's JSON pointer: `inputFormHints` at the root
  of the definition and `hints` in an approval `form`; (c) no hints. (a) changes the contract of an approved and built
  design, makes a placeholder edit change the schema digest that keys the validator cache, and is dangerous in a
  rolling upgrade: a process of the previous release reads the unknown keyword as outside the subset and refuses every
  start of the version, permanently for the engine's creators (design 043 Technical Plan 1: those refusals are final).
  (c) loses parity with Windmill's title, placeholder and widget customization. (b) leaves the validated schema about
  validation only, adds one IR field behind the same fleet capability as the other IR additions (Decision 026-013),
  and suits workflows written as code (design 044), whose schemas are generated from types and whose hints are set
  beside them. Pointers are instance pointers over `properties` names only (the form of design 043's `details.field`),
  checked at publication against the schema; an unknown pointer is `FORM_HINT_INVALID`.
- **D026-002 — The form model renders a subset of design 043's subset, from 043's own walk.** The form walker reads
  the schema positions that the 043 pre-check records (`crates/infra-schema/src/run_input/vocabulary.rs`), so the form
  and the validator cannot disagree on what a schema contains. Rendered: single types; a nullable field written as a
  type list `[T, "null"]` or as `anyOf`/`oneOf` of one supported schema and `{"type": "null"}` (what Pydantic and Zod
  emit for an optional field, so code-first schemas render); `enum`, `const`, the numeric bounds including the
  exclusive ones and `multipleOf` (as the control's step), the length bounds, `minProperties`/`maxProperties` on a
  group, `pattern` (shown as "must match the author's pattern"; checked only by the server), local `$ref` and the
  `$defs`/`definitions` containers, the annotations `title`, `description`, `default` and `format` (four formats chose
  the widget; `format` is never asserted, as in 043, so the form says so). Not rendered, so the field is unsupported
  and the model `UNAVAILABLE` when the field is required: other combinators, type lists other than the nullable form,
  `additionalProperties` as a schema (extra keys are never offered), arrays of objects. A version whose schema the 043
  pre-check refuses (stored before 0.3.0, or published in `report` mode) answers `UNAVAILABLE` with the new reason
  `SCHEMA_UNUSABLE`, and the Studio offers the JSON editor. Rejected: a renderer with its own keyword list (it would
  drift from the validator, the reason Decision 1320 put the walker on the server).
- **D026-003 — One validator for every value this feature validates.** Approval form input, stream event data, link
  presets and the merged public submission are validated by the platform validator of design 043 (`InputSchemas`),
  with its work counter, depth limit and closed refusal, never by the `jsonschema` crate. Their schemas must stay
  inside the 043 subset, checked at publication by the 043 pre-check (refusal `STREAM_SCHEMA_OUTSIDE_SUBSET`,
  `FORM_SCHEMA_OUTSIDE_SUBSET`, both under `VALIDATION_FAILED`). The validations take permits from the same per-process
  pool and its fair queue (design 043 Technical Plan 3), each counted for its tenant; one stream batch takes one permit
  for all its events. Rejected: a second pool (the cache weight bound of 043 counts the matchers four permits allow,
  so a second pool needs a second bound and a second measurement); the `jsonschema` crate (unbounded cost on hostile
  schemas, the finding that produced design 043). The cost of sharing is unmeasured (Gap 1339).
- **D026-004 — A public-link run carries the link id; live counts are counters kept by the database.**
  `runs.public_link_id` (nullable uuid, set only at admission, immutable like the admission identity) replaces the
  `runs.origin` value. Two counter tables follow `run_active_counts` exactly: `public_link_active_counts` per link
  (sixteen slots, kept by the same insert, finish and delete triggers when `public_link_id IS NOT NULL`, with a `WHEN`
  clause so that every other run pays nothing) and its sum per namespace for `public_link_active_runs`. Admission reads
  them under the existing quota advisory locks of slot 6 (ADR-0050 decision 7). Rejected: `runs.origin` (refused by the
  private-run guard, and its meaning is "private"); counting runs joined through `public_link_submissions` (the cost
  grows with retained receipts, and a purged receipt would drop a live run from the count); a table of live public-link
  runs (a delete on every terminal transition).
- **D026-005 — Waiters are woken by a new NATS subscriber, and the poll remains the authority.** A port
  `RunOutcomeHints` in `crates/application` and its adapter in `crates/infra-nats`: a core NATS subscription on
  `sagat.v1.run.finished.*` (and `sagat.v1.run.result_returned.*` once feature 022 publishes it) on every
  control-plane replica, without a queue group; JetStream-published messages reach core subscribers, so nothing in the
  relay changes. The wake-up latency includes the outbox relay's own pass. NATS disabled or down: the batched poll alone
  serves waiters (500 ms default). Rejected: PostgreSQL `LISTEN/NOTIFY` (as Decision 1325), a JetStream consumer per
  replica (durable state for a hint that may be lost).
- **D026-006 — The stream append fences the lease without a lock on `steps` or `runs`.** The append transaction locks
  its own attempt row (`run_stream_attempts`, one row per node instance, attempt and generation that appended) `FOR
  UPDATE`, then reads the step's lease without a lock, on the database clock sampled after that read, and refuses with
  `STALE_ATTEMPT` when the token, generation or attempt do not match, the lease expired or the attempt row is closed.
  An attempt row is closed (`closed_at`) by the publisher's final batch (`final: true`, which the SDK sends when it
  flushes before completing), or, for an attempt whose step settled or was superseded without a final batch, by the
  first feed read or maintenance pass that sees that committed state. A feed sends `settled` or `superseded` for an
  attempt only after its row is closed. Ordering proof: an append that took the attempt row before the close commits
  before the close can take it, so its events are read before `settled`; an append that takes the row after the close
  is refused. Leasing, heartbeat and completion read and write nothing of the stream tables, and the stream tables
  carry no foreign key to `runs` or `steps`, so an insert takes no `KEY SHARE` on a row the engine locks `FOR UPDATE`.
  Rejected: `steps FOR SHARE` (blocks heartbeats and completions behind appends and creates multixacts on the hottest
  row of a streaming step); a close written by the completion (one more statement on the hot path that design 038
  budgets); a time bound after settlement (PostgreSQL 16, pinned in `infra/docker/compose.yaml`, has no
  `transaction_timeout`, so the bound could not be proven).
- **D026-007 — Stream bytes are a storage class of their own.** Bytes of `run_stream_events.data` are charged to a new
  class `workflow_stream` with the tenant quota `tenant_stream_bytes`, in the way feature 041 charges `step_log`
  (ADR-0027 Amendment of 2026-11-15), not to `tenant_content_bytes`. A batch never takes the tenant usage row before its
  commit. Business reason: a runaway token stream of an agent must not exhaust the content quota that run completions
  need, which would fail unrelated runs with `STORAGE_QUOTA_EXCEEDED`. Architecture reason: the same hot-row contention
  that feature 041 measured on `tenant_retained_usage`. If feature 041's measurement (its Owner question 041-026) gives
  the step-log class a usage row of its own, this class follows. This amends Architecture decision 0068 decision 4
  and needs an ADR-0027 amendment written with the migration (Task 026-071).
- **D026-008 — The purge time is stamped by the purge pass, not by the terminal transaction.** `run_stream_heads`
  gains `terminal_seen_at` (infrastructure time), set by the purge pass the first time it sees the run terminal; rows
  are deleted after that instant plus the effective retention. The cost is at most one pass interval of extra
  retention; the gain is no added statement on the run's terminal transition. Erasure still deletes at once.
- **D026-009 — `executeRun` takes the whole `StartRunRequest` and says what each of its fields means for a wait.**
  With a business identifier held by an open run and `businessIdConflictPolicy: USE_EXISTING`, the call waits on the
  holder and the outcome says `admission: EXISTING` (otherwise `CREATED`); `run:read` is already required, so the
  holder is disclosed exactly as `startRun` discloses it. `startDelayMs` is accepted and simply makes a `202` likely.
  `inputBlobRef` is accepted. Signal-with-Start and Update-with-Start get no waiting variant: Update-with-Start already
  answers its Update result in its own response, and the run's result is `getRunOutcome` on the run it names; a third
  start operation with a wait would duplicate the authorization and idempotency rules of feature 018 for no new
  capability. The per-workflow OpenAPI specializes `signalWithStart`, `updateWithStart` and the chain Query and Update
  operations as well (Requirement 026-040).
- **D026-010 — A synchronous custom route waits on its event, then on its run.** A route with `response.mode: wait`
  admits its event into the inbox exactly as in mode `accept` (feature 021 owns that path); the waiter is keyed by
  tenant, endpoint and event id, woken by the batched poll over `event_deliveries` (the dispatcher writes `run_id` in
  its delivery transaction) and by the run hints once the run exists. Outcomes for the route's caller, a webhook
  identity: the run's public fields under `resultDisclosure: public-fields-v1`, else `202` with no run identifier; an
  event the dispatcher refused for its input (design 043 row 9) answers `422 VALIDATION_FAILED` `RUN_INPUT_INVALID` with
  `field` `/` and `rule` `other` (the policy `Root`, since the schema's names are not the sender's to learn); a
  dispatch not done within the wait answers `202`. Rejected: admitting the run synchronously in the control plane,
  which would bypass the inbox's deduplication, rate and dead-letter rules that feature 021 builds on. Feature 021 needs
  nothing new beyond what it specifies: the reserved `response` field, refusal of `mode: wait` until 026's capability
  is active, and `event_deliveries.run_id` written by the dispatcher (it already is). Order: 021's routes (mode
  `accept`) merge first; this feature's waiter is built in its own slice; the route wait is one joint task after both
  (Task 026-065). Gap 1342.
- **D026-011 — Workflow Streams stay separate from step logs.** Options weighed: (a) one store for logs and streams;
  (b) two stores with a shared pattern. A log line is lossy by design, unclassified text, redacted by value in the
  worker and read under `run:logs:read` (Architecture decisions 0084 and 0085); a stream event is lossless, typed by
  a declared schema, classified per field and read under `run:read` through the feature-016 projection. Merging them
  would either make logs pay for schema validation and exactly-once storage, or make stream values lose their per-field
  disclosure. (b) keeps both contracts and shares what is the same: the SDK's bounded buffer and background flusher,
  the attempt identity of a batch, the storage-class accounting and the attempt fence of Decision 026-006.
- **D026-012 — Names.** "Workflow Stream" (product term, Temporal's name) and, in the IR, `streams` and `emits`; the
  REST resource `stream-events`; the SSE path `/runs/{runId}/stream-events`. Kept distinct from: the graph stream
  (`/runs/{runId}/stream`, ADR-0016), the log streams and live tail of feature 041 (`/runs/{runId}/logs/streams`,
  `/runs/{runId}/logs/stream`), the Redis Streams destination of feature 033, and the operator chains of design 042
  (which avoid the word). The glossary needs one entry per term (Gap 1344).
- **D026-013 — Each surface that every process must honour is an instance-registry capability.** New
  capabilities of `crates/application/src/instances.rs` (Architecture decision 0082): `form-hints-v1` and
  `workflow-streams-v1` (control plane and engine: the IR codec of both must read the new fields, and the engine runs
  the purge and close passes), `notification-channels-v1` (control plane and engine: the engine's delivery worker must
  know the new destinations before a channel exists), `approval-channels-v1` (control plane and engine: the opening
  fan-out runs in the engine, the decision's settlement in the control plane), `public-links-v1` (control plane) and
  `synchronous-routes-v1` (control plane; feature 021 refuses `response.mode: wait` at route creation until it is
  active, Decision 026-010). Six in all.
  Publication refuses the corresponding IR fields, and creation refuses the corresponding resources, until the
  requirement is activated. `executeRun`, `getRunOutcome`, the form model and the per-workflow OpenAPI need no
  requirement: they are reads or admissions on one replica, and an older replica answers `ROUTE_NOT_FOUND`, which the
  SDKs treat as an older server. Rejected: per-process flags (the gap Architecture decision 0081 closes), one capability
  for everything (prevents the rollout order of `plan.md`).
- **D026-014 — Changes to existing operations are marked and the pinning tests learn the mark, never a re-bless.**
  `decideWorkflowApproval` (through `ApprovalDecisionRequest.input`, `.linkToken` and `ApprovalDecisionReceipt.via`) is
  the one existing operation this feature extends; it is marked `x-extended: '026'`. The byte-stability test of feature
  016 already skips its own `x-extended` value and the new operations of later features; it gains the same exclusion
  for 026. The baseline test of feature 035 gains an exclusion of operations marked `x-extended` by a later approved
  feature, with the reviewed one-operation diff in its comment, instead of a rewrite of its baseline file. No other
  existing schema gains a field (`RunOutcome` wraps `Run`; `Run` is unchanged).
- **D026-015 — Studio work follows the Astryx migration.** The Studio tasks start after pull request 267 merges and use
  its components and tokens. New directories: `features/forms/` (the shared renderer), `features/notifications/`
  (channels and Slack app bindings; not `features/integrations/`, which is the connector page of features 004 and 021),
  `features/public-links/`, `features/approvals/`, and the second build entry `src/public/`. *Start run* keeps its
  business identifier and start delay fields beside the generated form.
- **D026-016 — Workflows written as code use the same surfaces (Owner decision 2026-11-24-3).** What this feature
  requires of design 044: a procedural version publishes `inputSchema`, `outputSchema`, its Query and Update
  declarations and its stream channel declarations as data of the version, so that forms, links, `executeRun`, the
  per-workflow OpenAPI and the stream feed treat it as any version; its activities append through
  `AppendStreamEvents` under their lease; whether workflow code itself may append is design 044's decision, and if it
  may, the append is a command of the workflow task, its publisher sequence derived from the replay position so that a
  replay deduplicates (Gap 1343); a human approval opened from code is a feature-014 approval, so `notify`, `summary`
  and `form` are arguments of that SDK call and the same opening transaction applies. Nothing of this feature waits on
  design 044; its conformance scenarios add a procedural case once 044 lands. Also from the amendment 5.0.0: a cell
  has one writable region; public links and decision pages are served by the cell's control planes, and Decision 1640
  already carries the cell epoch in stream cursors.
- **D026-017 — The new run creators, and what each caller gets on refusal.** Rows for design 043's Technical Plan 1
  (the coordinator adds them to the design, documentation first, before the code) and for feature 022's "Run
  creators" table:

  | Creator | Entry | Gate call | `Creator` label | Invalid input | Schema unusable | Gate busy |
  |---|---|---|---|---|---|---|
  | Studio generated form | `startRun` (row 1 unchanged) | as row 1 | `api` | as row 1; the form marks the one field | as row 1; the form was `UNAVAILABLE` already | `503`, the form keeps its key and retries |
  | `executeRun` | `crates/api-http/src/routes/invocation.rs` → `start_with_options` (row 1, a second entry) | as row 1, before any waiter is registered | `api` | `422 RUN_INPUT_INVALID`, `field` and `rule` (policy `Show`); no wait | `422 RUN_INPUT_SCHEMA_UNUSABLE` | `503`, retryable, key not consumed |
  | Public-link submission | `crates/api-http/src/routes/public.rs` → `crates/application/src/public_links.rs` → `start_with` | `RunInputGate::check` on the merged input of the unlocked revision, before the admission transaction; the transaction refuses a revision other than the one checked and the use case repeats once from the gate | `public_link` (new) | `422 VALIDATION_FAILED` `RUN_INPUT_INVALID`, keeping `field` only when it lies inside an exposed pointer, else `/` and `other`; `rule` withheld outside exposed fields | the uniform `404` (an unusable link), audited `public_link.refused` `SCHEMA_UNUSABLE` and raised as the alert `PUBLIC_LINK_UNUSABLE` | `503` with `Retry-After`, no detail |
  | Synchronous custom route (with feature 021) | the dispatcher, row 9 unchanged | as row 9 | `trigger` | `422` `field` `/`, `rule` `other` (Decision 026-010) | as invalid | the dispatcher retries as row 9; the caller gets `202` at the end of the wait |

  A field outside the exposed pointers is refused before the gate with `422 VALIDATION_FAILED` `FORM_INPUT_INVALID`
  and `field` `/` (the caller's own keys are never echoed). No channel of this feature starts a run: a Slack slash
  command only links an identity (Requirement 026-042), and Teams and e-mail are outbound only, so there is no
  channel-started creator; one added later is a new row. Every creator above calls feature 022's permit port when that
  slice is merged; a run queued or buffered by it is simply not terminal, so a waiter answers `202`.

### Gaps added on 2026-11-24

| Id | Gap | Consequence |
|---|---|---|
| G1338 | Feature 022 proposes `runs.origin` values `STANDALONE_STEP` and `DRAFT_TEST`; the private-run guard of feature 017 refuses a non-null origin on a workflow that is not internal, the same conflict Decision 026-004 avoids. | Feature 022 resolves it in its own slice; this feature's no-notification rule for draft test runs reads whatever 022 chooses. |
| G1339 | Stream batches and decision inputs share the four validator permits of design 043 with run starts. | Unmeasured; Task 026-061 measures start latency with streaming load, and a separate pool needs a new cache bound. |
| G1340 | An unsealed attempt (a worker that died before its final batch) gets `settled` only when a feed read or the maintenance pass closes its row. | `settled` may arrive one pass interval after the step settled; no event is lost or reordered. |
| G1341 | Arrays of objects and tuples are not rendered by forms (tuples are outside the 043 subset, Owner decision 2026-11-19-5). | Such inputs use the JSON editor. |
| G1342 | The route wait (Decision 026-010) includes the dispatcher's pass latency before the run exists. | Synchronous routes answer later than `executeRun`; measured by Task 026-061. |
| G1343 | Appends from workflow code (design 044) are undefined until design 044 decides whether workflow code may append. | Activities stream; workflow code does not, until then. |
| G1344 | `docs/glossary.md` has no entry for Workflow Stream, graph stream, log stream or operator chain. | The coordinator adds them (this feature does not edit the glossary). |
| G1345 | Design 040 requires one OCSF mapping per audit action; this feature adds about thirty actions. | Each action enters 040's mapping table in the task that adds it, once 040 is built (Task 026-054). |
| G1346 | Owner question 1081 (feature 020) is unanswered, so the `pii` channel ceiling cannot be built. | Channels stop at `internal` until it is answered (Task 026-077 gate). |
| G1347 | External approvers without an account cannot decide (Owner question 1320 answered no). | A guest principal of federated login (feature 013) is the later route. |
