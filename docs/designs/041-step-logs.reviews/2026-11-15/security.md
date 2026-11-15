# Security review of design 041 (second round)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 879c550b1726870fa56392b1cff20272f114a5ed
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a fresh `security-reviewer` agent was given the design path, ADR-0084, ADR-0085, research.md and its own report of 2026-11-13, and asked whether each first-round finding is answered and what new defects the revision has; it was not given the coordinator's conclusions or the EGM stage reports. Its report is recorded here, condensed; repository paths are relative to the root. First-round findings still open keep their ids with the prefix `r1-`; new findings are numbered fresh. The reviewer marks new M1-M5, N1-N7 and N9 as checked against the code, and M6 and N8 as suspicions that follow from the text. It read, beyond the design: `crates/application/src/worker.rs`, `crates/infra-postgres/src/{execution,reveal_grants,content_usage}.rs`, the migrations `20261024000000_durable_quotas`, `20261008000200_runtime_role_grants`, `20261126000000_authority_read_locks`, `crates/api-grpc/src/limits.rs`, `crates/api-grpc/build.rs`, `crates/api-http/src/{dto_scripts,streams}.rs`, `crates/api-http/src/routes/{scripts,execution}.rs`, `crates/application/src/api_admission.rs`, `crates/domain/src/{identity,scripts}.rs`, `crates/telemetry/src/redact.rs` and the Python, Go, TypeScript and Java SDK hosts.

## First-round findings

| Id | Status | Where answered, or what is missing |
| --- | --- | --- |
| B1 | Answered | FR-041-001 (88-92), FR-041-007 (122-128), section 6 (631, 638-640), SC-041-004 (228-231), scenario 2 (662-664), ADR-0085 decisions 1-2; `_TaskLog` checked (`host.py:172-178`, built at `:712`). New problems beside it: M1, N1. |
| M1 | Partly answered | The checks are listed (section 2 steps 2-3, 473-483; ADR-0084 decision 5; D041-015); order and session check incomplete: r1-M1. |
| M2 | Answered | FR-041-008, step 7, D041-013, ADR-0084 decision 3, SC-041-001. |
| M3 | Answered | FR-041-007 (128-133), section 2 (463-464), D041-016, ADR-0085 decision 3; how to build it is N2. |
| M4 | Partly answered | FR-041-007 (123-128) covers keys, numbers, exceptions and context; number redaction still leaks: r1-M4. |
| M5, M6 | Cannot be assessed | Text lost in the first-round relay; defects in nearby areas are reported as new. |
| M7 | Answered | FR-041-004 (107-109), step 6 (488-491), wire (414-418), D041-018, ADR-0084 decision 7; a related cost problem is M2. |
| M8 | Answered | FR-041-023 (209-212), section 4 (564-575), T041-30, ADR-0085 decision 9, D041-025; leftovers N4, N5. |
| N1, N2, N6, N7, N8, N9 | Answered | Session token (411-412, 473-474; its check is r1-M1), FR-041-003 (102-103), section 8 (691-695), section 2 (459-466), section 5 (579-585) and section 10, ADR-0085 status line. |
| N3 | Answered as asked | FR-041-010 (150-152), section 5 (587-591); what "sealed" means is M6. |
| N4 | Partly answered (Nit) | FR-041-012 (163) and section 5 (611) re-authorize every poll, but the reused `follow` loop refreshes the principal every `recheck` = 30 s (`streams.rs:50`, `execution.rs:1381-1395`); the grant check re-reads the role (`reveal_grants.rs:47`) only in classified namespaces, so in an ordinary namespace a demoted or removed developer keeps tailing for up to 30 s. Re-read the locked membership role in each poll, or state the window. |
| N5 | Partly answered (Nit) | Lines 426-428 skip the generated `Debug` only for `AppendStepLogsRequest` and `StepLogRecord`; `StepLogField` and its oneof enum still print key and value; add both to `skip_debug` (`crates/api-grpc/build.rs:17`). |

## Blocker

None.

## Major

1. r1-M1 (still open): the check order and the session check are incomplete. Section 2 (473-477), ADR-0084 decision 5. Step 2 runs before step 3 but depends on a namespace: `current_worker_authorized` takes the namespace secrets lock (`execution.rs:1017-1027`) and `bound_patterns` reads bindings per namespace (`execution.rs:4756-4775`); before step 3 only the session's namespace is known. The session check omits the `namespace_id` match and `expires_at > clock_timestamp()` that `worker_session_room` requires (`execution.rs:679-701`). Scenario: a subject bound in A and B sends A's session token with a lease it holds in B; the removal of its binding in B is not seen until the lease expires; an expired session is accepted. *Fix:* read the lease row first (namespace, `activity`), match the session on (tenant, that namespace, id, token hash, subject, unexpired), then run the binding and membership checks there; decide whether a session expiring mid-attempt ends capture.
2. r1-M4 (still open): redacting numbers on each language's own decimal text leaks numeric secrets. FR-041-007 (124-125). Java's `Double.toString(12345678.0)` is `1.2345678E7`, which does not contain `12345678`; the server stores canonical JSON from Rust, where the digits appear, and scrubs only messages, keys and string values (463). *Fix:* `sdks/SPEC.md` defines one canonical decimal form (the stored one) and the SDK redacts on it; conformance cases with 12345678 and 1e15.
3. M1: the process-wide secret set drops a value when its attempt ends. Section 6 (638-640); ADR-0085 "Good" (100). An attempt resolves an API key, builds a cached client with it and ends; a later attempt logs an error containing the key, which is stored and served: the shared-state case ADR-0085 option 4 was rejected for, absent from rule 12 and G041-001. *Fix:* keep values for the life of the process (bounded), or add the case to rule 12 and G041-001 for the owner; a `step-logs-redaction` case where A resolves and ends, then B logs.
4. M2: rows that carry no log bytes are not limited by the tenant quota. Section 3 (546-548), section 2 (502-503), NFR-041-COST (824). `sagat_quota_measure` charges only the column octets (`durable_quotas.up.sql:152-174`); an empty line with null fields and every marker cost 0, so each run can hold 100,000 free rows and `tenant_log_bytes` never moves. *Fix:* a fixed per-row charge to `log` (a fixed-size measurement kind in the ADR-0027 manifest) or a per-tenant row ceiling from counters; a test that floods empty messages.
5. M3: `UPDATE (has_classified_schemas)` on `namespaces` gives the runtime role new privileges and new lock contention. Section 3 (538, 553-555), FR-041-021. `sagat_app` holds `SELECT, INSERT` on `namespaces` (`20261008000200_runtime_role_grants.up.sql:20`); namespace rows are locked `FOR SHARE` by `sagat_lock_namespace` (`20261126000000_authority_read_locks.up.sql:11-16`) for payload disclosure, event payloads, database admission and reveal grants; an `UPDATE` in the publication transaction takes `FOR NO KEY UPDATE`, which conflicts with them; the grant also lets the role lock any namespace row and clear the flag. *Fix:* an insert-only table (tenant, namespace) with `SELECT, INSERT`, `ON CONFLICT DO NOTHING`; readers test for a row.
6. M4: the withheld script log still reveals its size class. FR-041-022 (205-208), ADR-0085 rule 6 (59-62), CUJ 5 (366). `ExecutionBody` (`dto_scripts.rs:597`) and `ExecutionSummaryBody` (`dto_scripts.rs:653`) keep `log_truncated`, so a viewer learns that a script log reached 1 MiB. *Fix:* omit `logTruncated` whenever the log is withheld on the get and list routes; set `logWithheld` regardless of existence; both in T041-29's viewer test.
7. M5: `Class::Logs` shares the one admission counter. Section 1 (444-445), section 2 (455-457). `Shared.in_flight` counts every class and discovery is shed at three quarters of it (`limits.rs:520-563, 595`); a Logs call holds its ticket while it waits up to 50 ms for a per-tenant permit, so one tenant flooding `AppendStepLogs` lowers `PollTasks` capacity for all tenants on the replica. *Fix:* a Logs counter outside `in_flight`; the per-tenant sub-cap as a try-acquire with no wait; extend the class test.
8. M6 (suspicion): the cursor and the stream id may expose database-wide counters. Section 5 (587-591), section 3 (526, 533). The cursor holds `stored_xid` and the global identity `id`; the stream's public id is an identity bigint; "sealed with a server key" names no key and does not say encryption or integrity only; existing cursors are plain base64 (`routes/primitives.rs:193-209`). If the seal only signs, a tenant reading consecutive cursors and stream ids learns how fast all tenants write. *Fix:* an AEAD-encrypted cursor with a named key (source, sharing, rotation); a UUID public stream id.

## Nit

9. r1-N4 (partly answered): see the table; up to 30 s of tail after demotion in an ordinary namespace.
10. r1-N5 (partly answered): `StepLogField` and its oneof enum are not in `skip_debug`.
11. N1: FR-041-007 (127-128) captures "Rust `tracing` fields", which needs a layer on the process subscriber every attempt shares, against FR-041-001 (91-92); capture only the fields passed to `ActivityContext::log()`.
12. N2: the scrubbers cannot be called as described: `scrub_shapes` (`telemetry/src/redact.rs:66`) and `redact_credentials` (`domain/src/scripts.rs:779`) are private, the placeholder is `[redacted]` (`redact.rs:13`), `scrub` always cuts at 2,048 characters (`redact.rs:46-51`), and numbers and booleans are never hidden (`redact.rs:36-37`); T041-6 should name the entry points, the placeholder parameter and the rule for a number under a sensitive key.
13. N3: redacting field keys can produce two `[REDACTED]` keys; define the merge rule so the stored canonical JSON has no duplicate keys.
14. N4: the delete route is not classified in `ApiOperation` (`api_admission.rs:32-151`); add `RunLogsDelete`, and answer 404 to a `stream` of another run rather than auditing "0 streams".
15. N5: retention does not reach attempts that keep their lease (section 4, 570 skips leased streams), so FR-041-020 (200) is false for them; purge by `stored_at` inside leased streams.
16. N6: the lock order omits the binding advisory lock (`hold_worker_binding`, `execution.rs:4759`), which reports take first: binding, secrets lock, membership (`worker.rs:2729-2733`).
17. N7: section 1 (433-435) says ordinary roles do not hold `database:execute`, but admin does (`identity.rs:585`); for an admin token the refusal rests on executor ownership and live capability (`worker/database.rs:63`); correct the text and the `step-logs-negotiation` assertion.
18. N8 (suspicion): a filtered tail whose position moves only on delivery rescans up to 16 MiB on every reconnect, across up to 200 tails per tenant; keep the scanned position and send it as the `id` of `: ping` events.
19. N9: `current_worker_authorized` refuses only `DatabaseRunner` (`execution.rs:1040-1043`) and `runner` holds `worker:execute`, so refusing runner rests on the role read at authentication; refuse runner in the locked read too.

Counts: Blocker 0, Major 8, Nit 11
