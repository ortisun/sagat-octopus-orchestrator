# History replay — what 028 needs from feature 020's export, and what it reports

Feature 020 owns the exported run-history document: `sagat-run-history-v1`, returned by
`POST /runs/{runId}/history-exports` (FR-020-036) in `STRUCTURE` or `DISCLOSED` mode
(`specs/020-visibility-analytics-archival/contracts/run-history.schema.json`). Feature 028 consumes it in three places:
the test server's replay check, `sagat history answers` and `sagat history compare`. This page lists the fields 028
requires; 020's draft states that its format carries them. US03 stays blocked until that format is accepted (G1404).

- The replay check needs no value: a `STRUCTURE` export is enough, and `complete` is not required.
- `history answers` needs values: it reads a `DISCLOSED` export and uses only the values present; a `withheld` or
  `external` marker is treated as undisclosed. `complete: true` means every answer can be derived, not that the
  others are refused.
The public journal (`listRunJournalEvents`, `JournalEvent`) is not sufficient: it carries no codec-level event detail.

## Required fields

| Field | Why |
|---|---|
| `format` and `formatVersion` of the document | refuse an unknown format (`HISTORY_UNSUPPORTED_SCHEMA`) |
| producer build version and **event-schema version** | report and refuse what this build cannot decode |
| tenant-free run identity: run id, workflow name, version number, version digest | correlate; the tenant and namespace are not needed and should not be required |
| the pinned workflow version **document** (canonical JSON) or its digest plus the document in a sibling entry | version compatibility check (FR-028-015) |
| every journal event in sequence: `seq` (1..n, no gap), event type, node, instance path, and the codec-level `detail` the server's event codec decodes | decoding and folding (`event_codec::decode`, `RunState::replay_addressed`) |
| the recorded projection at export time: run status, stop reason/code, per step instance status, attempt count, chosen route, chain segment and links | the comparison |
| per step attempt output digest (SHA-256) — per attempt, not only per step (coordinator decision C22) | `OUTPUT_DIGEST` divergence and `history answers` sequencing |
| optional disclosed values: step outputs and failure codes **only as the export's disclosure policy released them**, each with its effective classification | `history answers` only |

The document must contain no secret value (ADR-0033 `secret` is never exported) and no credential. A value whose
effective level is `pii` appears only if the export revealed it under the reveal grant of feature 016.

## Replay check (test server)

1. Validate size (default 8 MiB, ceiling 32 MiB) and events (default 10,000, ceiling 100,000).
2. Decode the pinned version document with the IR codec; list capabilities this build does not enable
   (`unsupportedCapabilities`); continue so that event-level problems are also reported.
3. Decode each event in order; the first undecodable event is `EVENT_REJECTED` at its `seq` and stops the fold.
4. Fold with the domain reducer; compare with the recorded projection by instance path: `RUN_STATUS`, `RUN_STOP`,
   `STEP_STATUS`, `STEP_ATTEMPTS`, `ROUTE`, `CHAIN_LINK`, `OUTPUT_DIGEST`.
5. `compatible` is true when there is no divergence and no unsupported capability. Report bounded to 1,000 divergences.
6. Write `testing.history.replayed` (actor, document digest, verdict, counts) before answering.

## `sagat history answers <history> --out <answers.yaml>`

Writes a `worker dev` answer file (feature 012 D208): for each activity name, a `sequence` with one entry per recorded
attempt in order — `output: <disclosed value>` for a success, `fail: {code, retryable, message: "replayed"}` for a
failure, `lose: true` for an attempt that ended by lease expiry. A success whose value was not disclosed is omitted for
that activity (the activity then answers `NO_MOCK` at the first call that needs it) and listed in the summary. A
reconciliation is answered with its recorded verdict. Nothing is written for a value with effective level `secret`; a
`pii` value only if present in the document. The file starts with a comment naming the history digest.

## `sagat history compare <history> --run <runId>`

Reads the run's journal and graph through REST and compares, by instance path: the sequence of steps activated, chosen
routes, attempt outcomes (code and retryability) and terminal status. It reports differences; it does not claim that
a different workflow version is compatible with the history's (versions never migrate: constitution IV).
