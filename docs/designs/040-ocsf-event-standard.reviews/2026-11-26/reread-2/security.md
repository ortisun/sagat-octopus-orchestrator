# Second security re-read of design 040 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: d70ba587a14f0eed3d2517afc37285a23d1f80aa
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 1, Nit 2

Previous re-read: 1 fixed, 2 fixed (one leftover line in the runbook list, see Nit 3 below), 3 fixed, 4 fixed, 5 fixed.
- 1: the stored fingerprint is an HMAC (lines 524-526). The configuration revision only rises, `start` applies once, and each operator-stream write compares the revision (533-539). A global discovery cursor starts each new tenant just below its first record (530-533). Section 4 now matches section 5 (397-398).
- 2: the age now comes from a `SECURITY DEFINER` function owned by a monitoring role (623-626).
- 3: rotation is now in Requirement 040-013, refused with `PII_CEILING_NOT_PERMITTED` (150-151).
- 4: `/test` now answers `unreachable` for every egress refusal, resolution failure, transport error and timeout alike (599-600).
- 5: `system_identifier` is read through a narrow `SECURITY DEFINER` function (418-419).

1. **Major: Task 040-26's re-seal can silently undo a change of destination** (`docs/designs/040-ocsf-event-standard.md:812-815`)
   - `replace_sealed` compares only `key_id` (`crates/infra-postgres/src/secrets.rs:724-725`). That is safe for `secret_versions` because a version row never changes, but a sink's URL, credential and signing-key columns are changed in place by `PATCH` and by rotation.
   - Failing case: the re-seal reads `url_sealed` under K1. While replicas are still being rolled to the new key ring, an administrator `PATCH`es the URL away from a compromised collector on a replica whose active key is still K1. The re-seal's `WHERE url_key_id = K1` still matches, so it writes the old URL back. The re-seal does not raise the revision, so the pass sends to the removed destination.
   - Fix: also compare the old ciphertext (`AND url_sealed = $old`), or the sink's revision, and delete the line's claim that the update "cannot overwrite a concurrent change" until it does.

2. **Nit: what the operator stream does with a refused record is ambiguous** (`:527-528`, `:144-145`, `:512-518`, `:574-576`)
   - The operator stream inherits "the same ... halving and refusal". But step 6 writes `audit.sink_record_refused` into the tenant's trail, and `audit_operator_export` has no `refused_window_start` or `refused_in_window` columns.
   - Read literally, a builder could write the refusal into the tenant's trail. That would tell the tenant an operator sink exists and give it the id of a withheld `access.denied.cross_tenant` record.
   - Fix: say that an operator-stream refusal goes only to logs and metrics, and add the two window columns to that table.

3. **Nit: the system-identifier function's `search_path` is not pinned, and the runbook still mentions `pg_read_all_stats`** (`:418-419`, `:634`)
   - Unlike the monitoring function, the system-identifier `SECURITY DEFINER` function does not say its `search_path` is pinned. State `SET search_path = pg_catalog`.
   - The runbook list still says "the grant of `pg_read_all_stats`". Say that this role goes to the monitoring role, never to `sagat_app`.

Checked, no new Blocker or Major:
- **`startFrom`:** it reads only the tenant's own index window, and only below `xmin`. It refuses under `rebase`'s guard, the cursor is never shown, and records already sent are rechecked against the grant.
- **`rebase` and Gaps 040-007 and 040-008:** these are stated limits on what reaches the stream. The records stay in the audit table. No tenant can choose when they happen, because the pools bound statements and tenants get no transaction control.
- **Step 6's refusal rule:** a record passes as refused only beside an accepted sibling, with a cap of 10 per hour. A sink's own refusals stay out of its own scope.
- **Requirements 040-009, 040-011 and 040-013:** every change of URL, credential, home namespace or ceiling, every key rotation and every resume makes that person the authorizing subject. On a `pii` sink it needs the grant or a move to `internal`.
- **The storage block:** the sealed columns have their own key ids, the associated data includes the column name, and the table has `tenant_id NOT NULL` and a foreign key to the namespace.

Files read:
- docs/designs/040-ocsf-event-standard.md
- docs/designs/040-ocsf-event-standard.reviews/2026-11-26/reread/security.md
- specs/040-ocsf-event-standard/research.md (Decisions 040-016 to 040-018)
- crates/infra-postgres/src/secrets.rs

From the ADR, `docs/adr/ADR-0150-security-and-audit-events-leave-as-ocsf-projections-of-the-native-record.md`, I only searched for the changed terms; I did not read it in full.
