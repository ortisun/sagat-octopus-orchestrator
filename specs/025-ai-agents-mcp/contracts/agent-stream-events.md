# Token stream channel `sagat.agent.tokens` (draft, depends on feature 026)

Feature 026 owns Workflow Streams: the per-run stream log, `AppendStreamEvents`, delivery at
`GET /v1/runs/{runId}/stream-events`, superseded-attempt marking, per-reader classification,
retention and quotas (FR-026-013..021 in `specs/026-invocation-human-surfaces/spec.md`, draft).
Channel names starting with `sagat.` are reserved for platform channels whose schema another feature
fixes (FR-026-013). This file fixes the schema of the agent token channel and nothing else. Without
the 026 capability, `stream.tokens: true` is refused at publication and the runner appends nothing.

## Declaration

An `agent` node with `stream.tokens: true` implicitly declares and emits the channel
`sagat.agent.tokens`; authors cannot declare a channel with that name.

## Event data schema (closed carrier, classified per feature 016)

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["kind", "turn"],
  "properties": {
    "kind": {"type": "string", "enum": ["text_delta", "tool_requested", "turn_end"], "x-sagat-classification": "internal"},
    "turn": {"type": "integer", "minimum": 1, "maximum": 64, "x-sagat-classification": "internal"},
    "callIndex": {"type": "integer", "minimum": 1, "maximum": 256, "x-sagat-classification": "internal"},
    "toolName": {"type": "string", "maxLength": 64, "x-sagat-classification": "internal"},
    "textDelta": {"type": "string", "maxLength": 4096, "x-sagat-classification": "pii"}
  }
}
```

- `text_delta` carries `textDelta`; `tool_requested` carries `callIndex` and `toolName`, never
  arguments; `turn_end` carries neither and marks the end of the attempt's output.
- Thinking or other provider-opaque content is never emitted.
- With 026's per-reader projection (FR-026-018), a `run:read` holder sees no field of this channel
  (all are `internal` or `pii`), a holder of `payload:read-internal` sees kinds, turns and tool names,
  and only the feature-016 `pii` audience (reveal grant) sees text, the same audience as the
  transcript endpoint (D1290).

## Rules

- The runner appends under the model call's lease with 026's `AppendStreamEvents`, in batches of at
  most 64 events, `publisher_seq` increasing per attempt; the attempt and generation are those of the
  lease. A stale lease stores nothing.
- Deltas are not authoritative. The committed turn (journal and transcript) is. 026 marks events of a
  superseded attempt (FR-026-016); readers discard them.
- Deltas are coalesced: at most one `text_delta` event per 100 ms or per 1 KiB of text, whichever
  comes first.
- Budget (coordinator decision, review S6, aligned with 026's plan of 2026-10-27): 026 counts all
  reserved `sagat.` channels of a run in a reserved pool apart from the author pool. The agent's budget
  is the effective `maxTurns` × the per-call caps: per model call 1,024 events and 1 MiB by default
  (4,096 events and 4 MiB hard), so 16,384 events and 16 MiB for one agent at the default 16 turns.
  Publication sums this over every streaming agent node of the version and clamps the per-call caps so
  that the sum fits the reserved pool: 65,536 events / 32 MiB by default, 131,072 / 64 MiB hard, under
  026's per-run ceiling of 262,144 events / 128 MiB over both pools
  (`specs/026-invocation-human-surfaces/plan.md`, Finite ceilings). Every turn therefore keeps its
  share; a 16-turn agent streams all 16 turns, and author channels are unaffected.
- When 026 refuses an append for quota or rate (`STREAM_LIMIT`, `STREAM_RATE`), the runner stops
  streaming for that attempt and the model call continues (FR-026-019).
- A live subscriber learns that an attempt was superseded or a step settled through 026's control
  events `superseded` and `settled` (FR-026-016 in its current draft, G1295); clients discard the
  deltas of a superseded attempt and reconcile with the committed turn.
- Secret scanning of appended data is 026's (`SECRET_IN_OUTPUT`); the runner additionally never
  places credentials in deltas because it holds them only in its transport.
