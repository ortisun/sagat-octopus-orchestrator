# ADR-0016 — The Studio is a same-origin app over the REST API; the server validates and compares, and live views follow the journal by sequence

Status: **Accepted** | Date: 2026-10-10 | Feature: 007 | Builds on ADR-0008 (the runtime graph is a projection of the journal) and ADR-0003 (the declarative IR); changes neither

## Context

Feature 007 asks for a browser application where an author builds a workflow without programming, edits the
code of a node, and an operator watches a run and acts on a failure. The platform already offered drafts with
compare-and-swap, publish-time validation, a runtime graph, a journal without payloads and operator decisions,
but four things a browser application needs were missing, and each has a tempting answer that would have put
platform semantics in the wrong place:

1. **Validation while the author works.** Publish validates, with one message. The tempting answer is a
   validator in TypeScript, quick and wrong the moment the Rust rules change.
2. **What changes between a draft and a version, and what breaks.** The tempting answer is a client-side diff
   of two JSON documents, with whatever the client thinks "breaking" means.
3. **A live graph.** The tempting answers are to reload the whole graph on a timer (a graph of 10 000 nodes,
   every second, for every open tab) or to push a snapshot per event (a client that misses one is wrong
   forever).
4. **Where it lives and how it signs in.** The tempting answers are CORS for any origin, a token in
   `localStorage`, and a CDN for the editor.

## Decision

**1. The server validates.** `POST /workflows/{id}/validate` runs the same codec as publish on the document in
the body (or the stored draft) and answers 200 with every problem found (`code`, `nodeId`, JSON pointer `path`,
safe `message`) and the warnings; nothing is stored, nothing is audited. A draft is still saved when it is
invalid, because the author is mid-edit: **publishing is what is gated**, in the server (`422`) and in the
page. Advice the platform does not give (no timeout, nothing undoes a step of a saga) is computed in the
browser, shown apart, and never blocks.

**2. The server compares.** `GET /workflows/{id}/diff?from=&to=` (a version number or `draft`) compares two
documents as written, in `sagat-infra-ir`, and names what **breaks the contract** with callers and emitters
(a trigger removed or changed, an input property removed, retyped or newly required, an output property
removed or retyped, an awaited event type that disappears) and what **somebody must prepare** (a new activity,
a new secret, a different script artifact). Runs already started are pinned to their version and unaffected.

**3. A live view follows the journal by sequence.** Every transition of a run is a journal event with a
sequence, and a node changes only through events that name it or activate it. `GET /runs/{id}/graph/changes?since=N` returns
the events after `N` and the **full objects of the nodes they name or create**, the new sequence, and `reset: true` with
the whole graph when the client is ahead or more than 500 events behind. `GET /runs/{id}/stream` is the same
payload as `text/event-stream`, resumable with `Last-Event-ID`; polling the changes endpoint is the fallback,
so a client that moves between the two, or misses a connection, loses nothing. That applying every delta in
order gives the whole graph is a test of the platform, not a hope. A stream is bounded per subject and per
tenant, polled by the server at half a second (two when quiet), ends when the run does, at the token's expiry
and after 15 minutes at the latest.

**4. Same origin, memory only, local code.** The control plane serves the build under `/studio/` from
`SAGAT_STUDIO_DIR`; the page and the API share an origin, so there is no CORS. Every answer carries a content
security policy that allows scripts, connections and workers from that origin only (and the issuer's, when a
sign-in is configured). The editor is bundled, not fetched from a CDN. The bearer token is held in the page's
memory, never in storage, a cookie or a URL, and is gone when the page is reloaded; the stream is read with
`fetch` because `EventSource` cannot send the `Authorization` header.

## Consequences

- One definition of "valid", "different" and "changed" for the Studio, the API and, later, the SDKs and the
  GitOps of feature 011.
- A new endpoint set to keep compatible: `validate`, `diff`, `graph/changes`, `stream`, in the OpenAPI contract
  with conformance tests; the shape of `RuntimeNode`/`RuntimeEdge` became named schemas shared by the graph and its
  changes.
- The server polls the database for every open stream (gap G143); a push from the engine belongs to the scale
  feature. Streams are a bounded resource per tenant.
- A Studio on another origin is an ingress decision (a reverse proxy that serves both), not an option of the API.
- Browsers show no payload: the journal has none, by design (G144).
- The Studio is a test target of its own: unit and component tests, and journeys in a real browser against a
  real platform, run by the same harness style as the SDK conformance suite.
