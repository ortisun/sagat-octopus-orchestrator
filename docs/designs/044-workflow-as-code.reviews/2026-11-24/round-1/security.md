# Security review of design 044, round 1 (2026-11-24)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 44133eb732bb25f93904c0d144a7b5af1fa26d9f
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
only (no shell) and checked several claims in code.

## Blocker

1. Envelope secrets are not tied to activities: code can attach any envelope secret to any envelope activity, and a
   `c-<n>` step has no immutable node from which `ResolveSecrets` (`declared_secrets_of`, `crates/application/src/worker.rs:2665`)
   reads the declared set; make the envelope a per-activity map, refuse a command whose secrets are not its activity's,
   store the parsed fragment immutably and read secrets only from it.
2. History export "payloads as stored" returns classified fields to `run:read` plus `history:export` without a reveal
   grant, which ADR-0031 rules out; export redacted by default (feature 016), raw only with `payload:read-internal` and
   a reveal grant, `secret` never; an ADR amending ADR-0028 and ADR-0031; state the roles.
3. Fragments carry publisher-only fields: activity `outputSchema` (classification chosen at run time), `destinations`
   (egress to any connection), approval `policy` (four-eyes defeated); refuse them in fragments and take them from the
   published version (per-activity output schema, named approval policies).

## Major

4. Expression fields in fragments (`idempotencyTemplate`, `correlationExpr`, child `input`) let the worker inject
   expressions the server evaluates, contradicting "the server evaluates no expression on a code run's payloads"; make
   them literals and refuse every expression field.
5. `script.*` or `script.sha256-...` activity names bypass `envelope.scripts` and the publication-time script check
   (`crates/domain/src/scripts.rs:599`, `crates/application/src/definitions.rs:37-58`); `sagat.database.*` bypasses the
   refused kinds; refuse the `script.` and `sagat.` prefixes and check script digests per command.
6. A `*` binding (`crates/domain/src/binding.rs:165`) would cover `workflow:<type>`; nothing reserves `sagat.workflow`
   on the activity path; exact match only, reserve the name, exclude `~workflow` from the activity claim and the
   activity RPCs.
7. Permissions and role grants undecided for `history:export` (not in `Permission::ALL`), worker queries, updates, the
   move job and the promotion check; add a permission-by-role table.
8. Moves and resets need only `run:operate`: add `version:activate`, `run:batch` for many runs, a published and
   approved target; tests.
9. Worker queries and code-update results bypass ADR-0031 (disclosure declaration, closed result schema, server
   validation); a receipt read by key is an oracle; require the declaration, validate, `no-store`, bind reads.
10. Failure and nondeterminism details are worker free text; take the expected command from the server's record, check
    the actual target's grammar, closed causes, a bounded class pattern, drop the rest.
11. Bindings re-checked only on completion; idempotent replays would return eager lease tokens; re-check on every call;
    never store raw tokens; a replay returns no eager leases.
12. How replay-check and query tasks read history (up to about 200 MiB) and which blobs a workflow lease may read are
    undefined; per-run read capability, `ReadBlob` limited to that run's history, commands reference only blobs written
    under the same lease.
13. External callers can force a `HISTORY_LIMIT` hold with declared signals or rejected updates, and a stream of signals
    makes every terminal command `UNHANDLED_EVENTS` forever; caps on undelivered signals and unaccepted updates,
    headroom for the run's own events, stop admitting after repeated `UNHANDLED_EVENTS`.
14. No bounds on long waits (`waitMs`, worker queries) or on outstanding steps per run; quotas with typed `429`.
15. A trigger on `runs.workflow_version_id` is not a privilege boundary (`sagat_app` can set the session setting the
    repository uses elsewhere); revoke the column update, a `SECURITY DEFINER` move function with a fixed
    `search_path` that checks tenant, namespace, workflow and code kind; grants for the new tables; a test.
16. Publication approval (feature 011) reviews only the envelope, and the digest is what the worker claims; say so;
    optional signed build provenance; register the gap.
17. With `PYTHONHASHSEED=0` an outside sender can craft colliding keys that make every replay exceed the deadline and
    hold the run permanently; decode payloads into a seed-independent structure or cap keys per object.

## Nit

18. Fragment parsing under the run lock; parse before locking.
19. Unsalted input digests confirm guesses; never expose them, or use a per-tenant keyed hash.
20. The seed and generated UUIDs are predictable; keep the seed out of reads, warn in the SDKs.
21. The TypeScript bridge "single message function" is itself a host function; create it inside the context or drop the
    claim.
22. A worker can compute the chain from the history without running code, so the chain does not limit a forged verdict.
23. `ReleaseSticky` scope; denied and cross-tenant attempts on the new routes are not in the audit list.

Counts: Blocker 3, Major 14, Nit 6
