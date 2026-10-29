# Worker protocol excerpt (draft)

Additive to `specs/001-platform-foundation/contracts/worker.proto` (ADR-0015). No new RPC. The minor that carries these
fields is "the codec minor": the next additive minor at merge time (1.7 at the base commit `70f09435`, where 1.6 is
current), coordinated with features 019 and 026, which also propose additive fields and RPCs in 1.7. Field numbers below
are the next free ones at the base commit and are re-checked at merge (T031-007); `worker.baseline.json` is regenerated
deliberately.

```proto
// Added in the codec minor. Advertised at RegisterWorker by a host whose data converter has an encoder and a decoder.
message PayloadCodecCapability {
  repeated uint32 envelope_versions = 1;   // [1]
  repeated string codecs = 2;              // subset of "zstd", "gzip", "pad", "aes-256-gcm"
}

message WorkerCapabilities {
  // ... fields 1-5 unchanged ...
  PayloadCodecCapability payload_codec = 6;  // absent: the session is never leased a task with a payload plan
}

// One encoded location, relative to the document it applies to.
message EncodedPointer {
  string pointer = 1;    // RFC 6901 into the step document or the output; "*" is every array element
  string encoding = 2;   // "encrypted" | "compressed"
  string level = 3;      // "public" | "internal" | "pii" | "secret"
  string bound = 4;      // the ctx.p the envelope at this pointer must carry (its pattern in its own carrier)
}

// Added in the codec minor. Derived from the pinned version; never from the payload.
message PayloadPlan {
  string tenant_id = 1;                      // ctx.t for encoding and decoding
  string namespace_id = 2;                   // ctx.n
  repeated EncodedPointer decode = 3;        // /input/..., /steps/<node>/output/..., /item/..., /checkpoint/... (019)
  repeated EncodedPointer output = 4;        // the node's outputSchema locations (bound = pointer)
  repeated EncodedPointer checkpoint = 5;    // the node's heartbeatSchema locations (019), when declared
  map<string, StreamPlan> streams = 6;       // channel name -> encoded locations of its schema (026), when declared
}
message StreamPlan { repeated EncodedPointer locations = 1; }

message TaskLease {
  // ... fields 1-18 unchanged ...
  PayloadPlan payload_plan = 19;  // absent when the run's version has encodingRules 0 or the node needs no codec
}
```

## Server rules

- Registration persists whether the session advertised `payload_codec` (and whether it lists `aes-256-gcm`), together
  with the negotiated minor.
- Claim (FR-031-015): a task whose node has a non-empty `decode` **or** `output` plan is eligible only for a session with
  the codec minor and a `payload_codec` capability (with `aes-256-gcm` when any of its locations is `encrypted`). The
  bounded candidate query filters before its cap and the locked-run transaction repeats the check; an ineligible task
  stays `READY` and consumes no attempt (the 1.3 blob-gating pattern, `docs/architecture/worker-runtime-protocol.md`).
  Because the run input is part of every step document, an encoded input gates every task of the run. Compensation and
  reconciliation tasks use the plan of their forward node. A worker without a codec therefore never receives an envelope.
- 019 push invocation: the push envelope carries the same `payload_plan`; a push target is eligible only when its
  registration declares codec capability, under the same rule.
- `CompleteStep` for a node with a plan: validated per spec FR-031-010 inside the completion transaction, including
  `ctx`. A refusal is acknowledged (`already_applied` semantics unchanged), the step fails with `ENCODING_REQUIRED`,
  `ENVELOPE_INVALID`, `ENVELOPE_LEVEL_MISMATCH`, `ENVELOPE_CONTEXT_MISMATCH` or `ENCODING_CARRIAGE_MISMATCH`
  (not retryable), and the refused bytes are not stored.
- Side channels (FR-031-035): 019 checkpoint details and 026 `AppendStreamEvents` of an attempt with a non-empty plan
  are refused with `ENCODING_REQUIRED` unless the node's `heartbeatSchema` / the channel's schema is a carrier; when it
  is, the same write-path checks apply to them.
- `WriteBlob` with content type `application/vnd.sagat.envelope+json`: the server parses the first 4 KiB as an envelope
  header (members before `data`, strict JSON, `ctx` equal to the run's tenant, namespace and the root pattern `""`) while
  streaming and refuses the upload (`INVALID_ARGUMENT`, `x-sagat-error: ENVELOPE_INVALID`) otherwise.

## SDK rules

- A host with a data converter offers the codec minor and `payload_codec`; a host without one does not.
- Decode only `payload_plan.decode` pointers and check each envelope's `ctx` against `tenant_id`, `namespace_id` and
  `bound`; encode exactly the `output` (and `checkpoint`, `streams`) pointers with `ctx = {tenant_id, namespace_id,
  bound}`; a planned location the handler leaves absent stays absent; a planned `null` is encoded.
- Encode the output once per attempt and resend the same bytes on every retry of the report (`done-<task_id>`).
- While its key provider reports unhealthy, a host stops claiming tasks that carry a payload plan (they stay `READY`
  for other hosts or for later) and keeps heartbeating the tasks it holds.
