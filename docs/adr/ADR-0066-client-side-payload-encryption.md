# ADR-0066: Payload encryption is client-side and declared; the engine never holds a codec key and treats encoded values as opaque

**Status:** Accepted (2026-11-24, delegate, OD-2026-11-24-4; revised the same day before acceptance)
**Decided by:** agent proposal (2026-10-26; revised 2026-10-27 after the independent architecture and security reviews
and the coordinator's decisions on their blocking findings); revised and accepted on 2026-11-24 by the agent preparing
feature 031 for build, under the owner's delegation "Quero que os agents continuem por mim. Tomem a melhor decisão"
(OD-2026-11-24-4, which widens OD-2026-11-20-6). The owner may overrule it.
**Refines / supersedes:** refines ADR-0033 (level of an encoded value, delivery of ciphertext), ADR-0023 (encoded
blobs), ADR-0032 (additive IR key) and ADR-0016 through ADR-0062's combined Studio-origin refinement (operator-approved
codec origins in `connect-src`, deployment-wide); uses ADR-0082 (the instance-capability registry) for activation,
ADR-0085 (step-log redaction by value in the worker) for decoded values, ADR-0090 (run input validation) with the
encoded view, and ADR-0099 (delivery acknowledgement) for the hand-back; supersedes none.
**Enforced by:** planned checks of feature 031 — the publication defect corpus and one fixture per row of the
encoded-field registry (`tests/contract/client-side-encryption/`), the canary and carriage fault suites
(`tests/integration/client-side-encryption/`, `tests/fault/client-side-encryption/`), the six-SDK interoperability
matrix (`tests/contract/sdk-conformance/`, scenario `payload-encryption-interop`, 36 ordered pairs), the step-log
redaction scenario `payload-encryption-step-logs`, the codec-server black-box suite (target `contract_codec_server`),
the activation tests over the registry (`tests/integration/client-side-encryption/activation.rs`), and a source rule
in `scripts/check-architecture.py` that the feature's server modules import no cipher or decompression API
(`aes_gcm`, `ring::aead`, `zstd`, `flate2`). The existing, unrelated uses — the local secret key ring in
`crates/infra-secrets`, Kafka compression in `crates/infra-connectors`, `flate2` in `crates/infra-schema` — stay
allowed, so a crate-level ban is not possible. Until those checks exist: review only.

## Context

Payloads are stored as plaintext JSON (`payloads`, content-addressed, 256 KiB) or opaque blobs (ADR-0023). Feature
016 limits who may *see* a value but not who can *read the store* (016 G279). Customers with regulated data ask for
values the platform operator cannot read, with keys they control (OpenBao Transit or a cloud KMS), as Temporal offers
through payload codecs and a customer-hosted codec server (<https://docs.temporal.io/payload-codec>,
<https://docs.temporal.io/codec-server>).

Three facts make a direct copy wrong here. First, this engine evaluates a declarative IR server-side, and the parity
drafts add search attributes, analytics projections, step caches, concurrency, rate-limit and fairness keys, forms,
agent contexts, stream operators, run input validation and cross-namespace calls over payload values; an undeclared
ciphertext would surface as a runtime failure or as a key derived from ciphertext, and a carried envelope could reach a
workflow that evaluates it. Second, stored payloads are immutable and referenced by digest: an envelope format, once
written, must stay decodable for as long as data is retained (undecided, G69) and cannot be re-encrypted in place — so
whatever v1 does not bind, it never will. Third, the control plane must sign something for codec servers to know the
caller, but anything the control plane can sign, its operator can sign too.

## Decision

1. **Custody.** Codec key material MUST exist only in customer processes (clients, workers, codec servers) and the
   customer's KMS. No platform component MAY store, resolve, receive or derive it; platform secrets (features 009/017)
   MUST NOT be a key source, and no SDK key provider MAY read `ResolveSecrets` values. No server crate MAY decrypt,
   decompress or proxy an encode/decode request, and the platform MUST NOT call a codec server.
2. **Declaration.** Encoding MUST be declared with `x-sagat-encoding: encrypted | compressed` on the carriers listed in
   feature 031's encoded-field registry (workflow input and output, `activity`/`child`/`waitEvent` outputs, and the side
   channels 019 `heartbeatSchema` and 026 stream channel schemas). A platform-executed node MUST NOT declare encoded
   output. The interpretation MUST be pinned per version (`workflow_versions.encoding_rules`); versions published before
   it stay inert.
3. **Opacity across every draft.** Publication MUST refuse every registered expression position that reads an encoded
   location (non-carriage), and every consumer feature MUST call the same check and use the registry's codes; a position
   not registered is refused for documents with encoded locations. The engine MUST NOT read an envelope's `data`. Run
   input validation (ADR-0090) validates the encoded view, in which each encoded location is checked only as an
   envelope, after the envelope checks.
4. **Envelopes exist exactly where declared, bound to their place.** Every write into a carrier of a rules-1 version
   MUST refuse plaintext at a declared location, an envelope at an undeclared location, an envelope of the wrong level,
   and an envelope whose context (`ctx`: tenant id, namespace id, pointer pattern) is not the run's tenant and
   namespace and the location's pattern. Cross-document admissions (child, failure-handler child, pins from other runs,
   reset or rerun into another version) MUST also map the source's pinned plan through the carriage path and refuse
   unless the target's pinned plan declares every mapped location — whatever the target's rules. Encoded values MUST NOT
   cross namespaces in v1. Refused values MUST NOT be persisted in payloads, the journal, audit, dead letters, logs or
   errors (two stated exceptions: an inbox event's data stored at admission, an uploaded blob awaiting the ADR-0023
   sweep).
5. **Envelope v1 is permanent and bound.** `{"$encoded": {v: 1, codecs, cty, lvl, ctx, key, iv, data}}` with
   AES-256-GCM over a data key wrapped by a customer key, optional zstd/gzip then padding to size buckets before
   encryption, and the canonical (RFC 8785) header — `ctx` included — as associated data. Parsing is strict (duplicate
   members refused); the header is at most 3,072 bytes. Every future SDK release MUST decode v1; a change of format is a
   new `v`, added beside v1.
6. **Rotation is forward-only.** Retained envelopes MUST NOT be rewritten; rotation applies to new writes; operators
   MUST keep key versions decrypt-capable while data under them may be retained. Destroying a key is the customer's
   crypto-shredding lever, not a compliance claim of the platform.
7. **Classification gates ciphertext.** An encoded location's effective level MUST be the strictest level in its
   subtree. Envelopes are disclosed only to the 016 audience of that level, on explicit request; `secret` envelopes are
   disclosed to nobody; a codec endpoint declares a level ceiling, `pii` only when set by an administrator holding the
   reveal grant, rechecked at every `pii` assertion.
8. **The customer authenticates the person; the platform only narrows.** The Studio and the CLI MUST call the customer
   codec server directly. The codec server MUST authenticate every person with the customer's own identity provider (a
   session cookie in the browser, a customer-issued bearer token for the CLI). The platform adds a signed, audience-bound,
   single-use assertion (ES256, at most 300 s) naming tenant id, namespace id, purpose and the highest level the caller
   may receive; a codec server MUST intersect its own policy with it and MUST NOT decode on the assertion alone. The
   platform access token MUST NOT be sent to a codec server. Codec origins are approved by the operator (CSP
   `connect-src`, deployment-wide) and selected per namespace by an administrator.
9. **Enablement is fleet-wide, through the registry.** Publication with encoding MUST be refused until the requirement
   `payload-encoding-v1` is recorded for the control plane and the execution engine in feature 018's
   instance-capability registry (ADR-0050 decision 9, ADR-0082), which succeeds only when every live instance of both
   kinds lists it; the recorded requirement is the activation and the registry's history is its evidence. The publish
   transaction MUST fence on the requirement epoch, and the requirement MUST NOT be removed while any version with
   `encoding_rules = 1` exists. Validation of already published rules-1 versions never depends on it.
10. **Plaintext stays in the customer's process, logs included.** The SDK worker host MUST redact decoded values by
    value from step-log text and failure messages before they leave the process (ADR-0085 rule 1 extended to decoded
    values), and MUST decode before it asks for secrets, so that a key-provider failure before the handler starts can
    hand the lease back without spending an attempt (ADR-0099 decision 5). Six official SDKs implement the converter
    under one conformance suite.

## Considered options

1. **Client-side, declared, opaque engine, bound envelopes, customer-authenticated browser decoding (chosen).** The
   platform enforces what it can without a key; the trust boundary is visible in the IR and pinned.
2. **Undeclared codec on every payload (Temporal style).** Every server-side expression, key or form over an encoded
   value fails at run time or derives garbage; the engine could not refuse plaintext. Rejected for IR documents. A
   code-first workflow (design 044), whose payloads the server never evaluates, may encode whole payloads through the
   same converter under rows it registers.
3. **Platform-side encryption at rest with a customer key (BYOK) that the engine unwraps.** Transparent to expressions,
   but the platform holds usable keys. Rejected for this need; storage-level encryption remains a deployment concern.
4. **Server-side decode proxy for the Studio.** Plaintext and the decode capability pass through the platform. Rejected.
5. **Platform assertion sufficient on its own (the first draft of this ADR).** The control plane holds the signing key,
   so an operator or a compromised control plane could mint `pii` assertions and decrypt envelopes copied from the
   database. Rejected by the security review (its first blocking finding).
6. **Forward the user's platform access token to the codec server.** A credential valid at the platform API reaches a
   third-party origin. Rejected.
7. **Header-only AAD in v1, context binding in a later v2.** v1 is permanent, so v1 envelopes would remain writable and
   transplantable forever. Rejected (the security review's first serious finding); the cost is that carriage keeps bytes
   only within one namespace and pointer.
8. **Deterministic (SIV) encryption for natural idempotency and deduplication.** Leaks equality of values. Rejected;
   idempotent replay uses a header fingerprint plus an optional keyed content fingerprint (feature 031 D1535, D1669).
9. **A separate activation table and tenant audit actions for activation (the 2026-10-27 text).** A second store of
   the same fact as the registry could disagree with it, and an audit record needs a tenant that a deployment-wide act
   does not have. Rejected on 2026-11-24 for the registry's requirement row, its history and a publication fence.

## Consequences

- Authors must declare encoded fields and accept that the engine cannot branch, correlate, cache, index, key, rate-limit,
  form, operate on or prompt over them, nor validate their plaintext constraints; moving an encoded value to another
  pointer needs a customer worker step that re-encodes it; encoded values cannot be passed to another namespace (030)
  in v1.
- Payloads grow (base64, header, padding) and lose content-address deduplication; quotas count the encoded bytes.
- Workers need the codec protocol minor (the next free one when the contract merges, 1.10 or later), which also means
  every lower minor; any task whose step document holds envelopes is gated to codec-capable sessions, so a mixed fleet
  can leave such tasks `READY` until a capable worker arrives.
- Customers must run an identity provider in front of their codec server; browsers that block third-party cookies may
  require hosting the codec server on the Studio's site (G1539). The Studio's CSP becomes deployment-configurable.
- Metadata (structure, lengths, level, tenant, namespace, location, key reference) stays visible; replay within one
  namespace and location is not detected (G1520, G1521). A binary older than 018's registry, started after activation,
  is neither refused nor seen by the registry (G1537). Step-log redaction misses short and transformed values
  (G031-002).
- Rollback: once a rules-1 version exists, the requirement stays recorded and a binary without the feature cannot serve;
  envelopes remain readable by customer SDKs; nothing is migrated.

## Evidence

None yet; this is accepted design, not a test result. Specification, research and planned tests:
[spec](../../specs/031-client-side-encryption/spec.md),
[research](../../specs/031-client-side-encryption/research.md) (D1520–D1559, D1665–D1669, D031-001 to D031-014,
G1520–G1541, G1665, G031-001 to G031-003),
[plan](../../specs/031-client-side-encryption/plan.md),
[envelope contract](../../specs/031-client-side-encryption/contracts/envelope.md),
[encoded-field registry](../../specs/031-client-side-encryption/contracts/encoded-field-registry.md).

## Acceptance

- **2026-11-24, delegate (OD-2026-11-24-4).** Revised, then accepted. What the revision changed and why: Rust is the
  sixth official SDK (decision 10, Enforced by); activation uses the registry that feature 018 built instead of an
  activation table of its own (decision 9, option 9); run input validation of design 043 checks the encoded view
  (decision 3); decoded values are redacted from step logs in the worker and a key-provider failure before the handler
  is handed back at protocol 1.8 (decision 10); the envelope header bound is 3,072 bytes because the largest legal
  header is 2,317 (decision 5). Accepted because every alternative either gives the operator a decrypt path (options 3
  to 6), leaves v1 transplantable (option 7), leaks equality (option 8), or lets the engine meet ciphertext it would
  evaluate (option 2); the chosen design is the only one that keeps the operator out of the plaintext while the server
  still enforces what it can without a key.
