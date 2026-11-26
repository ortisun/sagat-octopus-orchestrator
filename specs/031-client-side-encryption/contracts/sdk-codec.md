# SDK data converter and codecs (draft, normative for the six SDKs)

Merged into `sdks/SPEC.md` as a new section by T031-033; language-neutral names, each SDK uses its idiom. Corrected
2026-11-24: six SDKs, Rust included (feature 034, Decision D031-001).

The data converter is the **only** SDK stage that transforms payload bytes, including blob envelopes (coordinator C5).
Feature 028 interceptors observe and decorate calls (headers, tracing, logging, authentication) and MUST NOT transform
payload bytes; 031 depends on 028 only for the ordering below.

## 1. Surface

```text
DataConverter(codecs: [Codec...], options?: {maxDecodedBytes, pad: true})
  Codec = Compression("zstd" | "gzip", level?) | Encryption(keyProvider, {kid, cipher: "aes-256-gcm"})
ClientOptions.dataConverter / WorkerHostOptions.dataConverter
client.payloadPlan(workflowId, versionNumber)          -> PayloadPlan with tenantId and namespaceId (cached per version id)
client.startRun(..., {versionId})                      -> encodes input per the plan of the pinned version
codec.encodeValue(value, {encoding, level, pointer})   -> envelope for a declared waitEvent location (signal or event data)
codec.decodeValue(envelope, {pointer})                 -> value (checks ctx)
KeyProvider {
  dataKey(kid)                    -> {dek: 32 bytes, wrapped: bytes, kv?: string}
  unwrap(kid, kv?, wrapped)       -> dek
  healthy()                       -> bool          // cheap probe used to pause claiming (§2)
  fingerprint?(kid, bytes)        -> bytes         // optional keyed digest for idempotency (D1669)
}
OpenBaoTransitKeyProvider({address, mount, keyName, tokenProvider, tls})   // the customer's Transit
StaticKeyProvider({keys: {kid: {kv: 32-byte KEK}}, allowInsecureTestKeys: true})  // refuses without the flag
```

- The codec chain is ordered as written; with a compression before the cipher the converter inserts `pad` unless
  `pad: false` is set explicitly (not recommended): `[Compression, Encryption]` produces
  `codecs: ["zstd", "pad", "aes-256-gcm"]`. Encryption before compression is refused when the converter is built.
- `ctx` is never chosen by the caller: the client takes `tenantId`/`namespaceId` from the payload plan, the worker host
  from the lease plan, and `p` from the plan entry; `encodeValue` accepts only a pointer of the target wait's plan.
- `dataKey` results may be reused (cache) for at most `dekMaxUses` envelopes and `dekMaxAge`; `unwrap` results are
  cached by the digest of `(kid, kv, wrapped)` for at most `unwrapTtl`, `unwrapMaxEntries`. Zero disables a cache.
  Evicted keys are overwritten where the language allows (Go, C#, Java byte arrays; Python `bytearray`; TypeScript
  `Uint8Array.fill(0)`; Rust a zeroizing buffer dropped on eviction), and are never logged or printed (string/debug
  forms show `<redacted>`).
- A key provider never reads platform secrets: the worker host does not pass `ctx.secrets()` to it, and no provider
  accepts a platform secret reference.
- JSON parsing of envelopes is strict (duplicate members refused).

## 2. Where the converter runs

| Path | Encode | Decode |
|---|---|---|
| Client `startRun`, 018 with-start, 022 standalone, 026 run-and-wait start | input, per plan of `versionId` (the SDK resolves and pins the active version when none is given) | — |
| Client `signalRun`, `signalExecutionChain` | a value produced by `encodeValue` for a declared wait location | — |
| Worker host lease | — | `payload_plan.decode` pointers, before the handler |
| Worker host report | output, per `payload_plan.output`, after the local secret scan | — |
| Worker host heartbeat (019) / stream append (026) | per `checkpoint` / `streams` plan; refused locally when the plan is non-empty and the channel has none | delivered checkpoint per `decode` |
| Blob output over 256 KiB | whole output as one root envelope, uploaded as `application/vnd.sagat.envelope+json` | read then decode |

A workflow function of design 044 (workflow as code) uses this same converter for the payloads it reads and writes;
how its payloads are planned is 044's to register (D031-012).

The converter is the stage nearest the transport. With 028 interceptors, outbound interceptors run before encoding and
inbound interceptors after decoding; no interceptor runs between the converter and the transport. Every keyed write
encodes once per logical request and reuses the encoded body for every transport retry of that request; a report
encodes once per attempt.

While `keyProvider.healthy()` is false, the worker host stops claiming tasks with a payload plan (they stay `READY`, no
attempt is spent) and probes again with backoff; a long KMS outage therefore does not burn retry budgets of tasks not yet
claimed (D1665).

A host decodes a lease's planned locations before it calls `ResolveSecrets` and before the handler starts. At protocol
1.8 or later, a decode that fails because the key provider is unavailable hands the lease back (`FailStep` with
`TASK_NOT_STARTED`, ADR-0099 decision 5): the attempt is not spent, once per attempt; a second such failure of the same
attempt is reported as `PAYLOAD_CODEC_UNAVAILABLE` (D031-004).

**Redaction of decoded values (FR-031-038, D031-007).** For every attempt in flight in the process, the host adds the
decoded string leaves of each planned location and the canonical JSON text of each decoded value to the by-value
redaction set of the per-attempt logger of design 041 (ADR-0085 rule 1: values of at least 6 bytes, longest first,
replaced by `[REDACTED]`), and applies the same set to `safe_message` before a `FailStep` is sent. The set lives only
while its attempt runs and is never logged.

**What the converter never touches.** `error_code`, `safe_message` (beyond the redaction above), design 045's
`next_retry_delay_ms`, the lease and acknowledgement fields, identifiers and idempotency keys (D031-003).

## 3. Failure codes

| Code | Retryable | When |
|---|---|---|
| `PAYLOAD_CODEC_UNAVAILABLE` | yes | key provider timeout, connection refused, 5xx, sealed (at 1.8 or later, a first decode failure before the handler is a hand-back instead) |
| `PAYLOAD_KEY_UNAVAILABLE` | no | unknown `kid`, version below the provider's minimum, permission denied |
| `PAYLOAD_DECODE_FAILED` | no | malformed or duplicate-member envelope, `ctx` mismatch, tag mismatch, bad padding, decompression error, invalid JSON |
| `PAYLOAD_DECODED_TOO_LARGE` | no | decoded bytes over `maxDecodedBytes` |
| `PAYLOAD_ENCODE_FAILED` | no | value not serializable (§8 of `sdks/SPEC.md`), envelope over the inline or blob limit |
| `SECRET_IN_OUTPUT` | no | plaintext output, checkpoint or stream event contains a value resolved by `ResolveSecrets` in this attempt |

The worker host reports them with `FailStep`; the client raises them as `SagatError` before any request is sent.
Messages carry the code and the pointer pattern, never a value, ciphertext, key or provider response.

## 4. Conformance (tests/contract/sdk-conformance)

| Scenario | Checks |
|---|---|
| `payload-encryption-builders` (build only) | byte-identical documents with `x-sagat-encoding` |
| `payload-encryption-vectors` | each SDK decodes the reviewed vectors and fails the negative ones (including `ctx` and duplicate-member cases) with the recorded codes |
| `payload-encryption-interop` | each SDK encodes a fixed set; the harness has every SDK decode it (6 × 6 = 36 ordered pairs, 30 across languages) |
| `payload-encryption` | runtime: encoded start, decode in the worker, encoded output, downstream decode; wire capture at the transport hook shows envelopes only; plaintext start refused |
| `payload-encryption-faults` | Transit stopped (claiming paused; a claimed lease handed back with `TASK_NOT_STARTED` and its attempt unchanged; a second failure of the same attempt retryable), unknown `kid`, disabled version, cache expiry, secret in output, lost report retried with identical bytes, re-encoded start retry after restart |
| `payload-encryption-gating` | a worker at the previous minor and one without a codec are never leased a task with a decode or output plan |
| `payload-encryption-step-logs` | two concurrent attempts; a canary decoded in each handler and written to a step-log line, a field and an exception message is in no stored step-log row, failure or span of either attempt (added 2026-11-24, FR-031-038) |
