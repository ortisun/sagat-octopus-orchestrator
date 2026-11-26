# Security review of design 044, round 8 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 3884f9f3db9c7ec9c7405f2970a02985c4249711
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 7's Majors 1 to 4
and Nits 5 to 12 are closed. No finding on the tenant boundary in SQL, secrets on the workflow task, export and blob
disclosure, grants, definer functions, injection, metric labels or the re-checked races.

## Major

1. Whoever runs a workflow worker writes the run's commands but is never an initiator, so a developer running the
   worker of a type can self-approve a gated payout: a workflow-type binding grants authorship; require
   `definition:publish` to set it, record the binder as an initiator, show the issuing identity; a test.
2. A workflow failure's code and message reach the run view unclassified: a closed code pattern; the message dropped or
   stored as a `pii` payload; the same for update failures; a test.
3. Each overlay cache miss reads up to 6 MiB under the run lock and the key changes at every completion, so reads grow
   with the square of a run's length against shared capacity: key the cache by run, read only new rows by command
   number, let the completion feed the cache, bound the bytes per transaction; a test.

## Nit

4. `WriteBlob` only above 256 KiB contradicts moving the largest values to blobs; no blob path for state, results, output.
5. A test name still says 4 MiB pages.
6. Replay-check and query tokens: lifetime, session binding, invalidation; ranges only in order.
7. The lease's first page is a history read and is not audited; collapse audits per session and run.
8. A test name against the run view drawn from the overlay; readers of `run_events.detail` that must strip fields.
9. A signal can spend most of a run's retained bytes: a per-signal size bound.
10. `toVersion`'s meaning, a missing target's answer, the export of a graph run.
11. Whether a `saga:override` action writes an initiator.

Counts: Blocker 0, Major 3, Nit 8
