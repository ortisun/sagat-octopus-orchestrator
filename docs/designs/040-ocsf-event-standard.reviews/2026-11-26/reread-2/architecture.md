# Second architecture re-read of design 040 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: 3f1aa6284446f2fe21864eebfc18d2de4dd2e0d7
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 2, Nit 3

Previous re-read: 1 fixed, 2 partly fixed (the prose is fixed but Success criterion 040-007 is not), 3 fixed, 4 fixed, 5 partly fixed (the prose is fixed but the storage and the test are not), 6 fixed, 7 fixed

1. Major: Majors 2 and 5 of the previous re-read are fixed in section 4's prose, but not in the checks that enforce them.
   - Where: Success criterion 040-007 (lines 197-198), the storage comment (line 563), Task 040-9 (lines 741-746) and Task 040-13 (lines 768-771).
   - What: the criterion still says "with the epoch step delivers; without the step, every sink reports `CURSOR_INVALID`". Both cases are `CURSOR_INVALID`, so a test written to it passes whether or not `rebase` refuses. Nothing checks the discovery cursor either. The storage comment for `audit_operator_config` has no `discovery_cursor_cluster` and no stored `start`, and Task 040-9 does not create that table.
   - Correction: rewrite the criterion to match lines 423-426: `CURSOR_INVALID` until `rebase`, `rebase` refuses without the epoch step, and a tenant created after the restore is discovered. Add the two columns to line 563 and the table to Task 040-9.

2. Major: the fix leaves a sink's own `audit.sink_*` records out of that sink's scope, but the requirement and the query do not, and it hides tampering.
   - Where: section 5 step 6 (lines 512-514), against Requirement 040-009 (lines 124-126), the section 4 query (line 388) and Task 040-11 (line 754).
   - What: the requirement and the query exclude only the withheld actions and `audit.sink_delivered`, so a builder who follows them does not apply the exclusion. One who follows step 6 hides all of the sink's own records from it: a change of ceiling to `pii`, a URL change, and an administrator's pause and resume. A tenant with one sink never sees these in its SIEM. Only `audit.sink_record_refused` can feed back on itself.
   - Correction: exclude only the sink's own `audit.sink_record_refused`. Write that predicate into Requirement 040-009 and the query, and add a test.

3. Nit: Task 040-26 (`reseal-secrets` for `audit_sinks`, lines 802-804) is not in any slice. Slice 2's order (line 706) leaves it out, so sinks could ship before the re-seal that keeps them working after a key rotation. Add it to Slice 2.

4. Nit: the operator fingerprint is "SHA-256 of the URL and of the credential" (line 519), stored in a row the runtime role can read. A plain hash with no key or salt lets anyone who can read that row guess a weak credential offline, which goes against "never … a credential" in Requirement 040-011. Use an HMAC keyed by something held only in the engine's configuration, for example the signing key.

5. Nit: the `rebase` guard `max(tx_id) >= xmax` (lines 418-419) only catches a missing epoch step until the new cluster's counter passes the restored maximum. After that, `rebase` is allowed, and records written in between sort below the rebased cursor and are skipped. Either record this as a residual risk in Gap 040-007, or make the guard check that the epoch was actually advanced.

Files:
- docs/designs/040-ocsf-event-standard.md
- docs/adr/ADR-0150-security-and-audit-events-leave-as-ocsf-projections-of-the-native-record.md
- specs/040-ocsf-event-standard/research.md
- docs/designs/015-opentelemetry-and-openbao.md (line 433 now lists the `audit_sinks` columns)
