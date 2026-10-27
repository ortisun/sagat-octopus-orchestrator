# Feature 026 — Baseline, external semantics, decisions, gaps and owner questions

Baseline investigation: 2026-10-26, worktree `sagat-octopus-parity-specs` at `70f09435` (release
0.2.0 on `develop`). Every path below was read or listed at that revision. Nothing in this feature is
implemented. Identifiers: decisions D1320–D1359, gaps G1320–G1359, owner questions O1320–O1359
(allocated range; numbers below 1000 belong to other branches).
Revised on 2026-10-26 after the independent architecture and security reviews: decisions were
revised in place where noted, D1357–D1359 and G1334–G1337 were added. Revised again for the
coordinator decisions of 2026-10-27 (C1–C22): D1640–D1641 come from the overflow range D1640–D1644.

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
  the page's `frame-ancestors` from that one link (ADR-0061 decisions 3 and 4; security review B1).
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
  (feature 025's model-output floor, security review B4), so an author annotation cannot relabel it
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
  and still cannot tie a team to one tenant without the same install binding). (Architecture review B1,
  security review B2/S5.)
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
  never on the Studio origin (security review B1).
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
