# Second security re-read of design 015 (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: 9050efb36c86ac5de2177147fe765a0b446e4fbb
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 1, Nit 2

Previous re-read:
- 1 (the output scan is narrowed to versions current since the lease began): fixed. In docs/designs/015-opentelemetry-and-openbao.md, lines 1097-1107 open every retained secret version under `transit`, using one `unwrap_batch` per report. During an outage, a report with uncached older versions is refused as retryable. Task 015-30 (lines 1547-1552) inverts the test, and ADR-0152 decision 6 (lines 61-62) matches.
- 2 (feature fragments' allow-lists certify themselves): partly fixed. Lines 418-422 and 1050-1069 add declared custody prefixes, an overlap check between owners that includes the platform, and the negative fixtures. ADR-0152 decision 5 (lines 57-59) matches. Containment is not specified segment by segment, which leaves a bypass (new finding 1).
- 3 (the sealing version is not enforced as a ceiling): fixed. Lines 857-860 make the `secret_versions` trigger refuse a `wrapped_key_version` above `seal_version`. Line 883 applies the same check to every inventory entry, line 826 caps `key_authority_raise_floor`, and line 1417 adds the barrier test at `seal_version + 1`.
- 4 (`SECURITY DEFINER` trigger functions have no pinned search path): fixed for the functions this delivery creates. Lines 845-854 pin `search_path = pg_catalog, pg_temp`, qualify objects with `public.`, and add a privilege-audit row on `proconfig`. Line 1404 does the same in the task. The audit row's scope is new finding 2.
- 5 (inbound webhook verification tells the sender about the key service): fixed. Lines 428-430 and 1207-1211 answer `Refused` and `Denied` with today's plain `401`, with no code and no body. `Unavailable` stays `503`.
- 6 (nothing disposes of the ring after step 8): fixed. Lines 180-187 add step 9: delete the cluster Secrets, keep the ring offline only while pre-migration backups are retained, then destroy it, and rotate exposed values at their source. The custody wording is new finding 3.
- 7 (rewrap has no cursor): fixed. Line 991 adds the `id > $after` cursor within a run and counts each failure once per run.
- 8 (the environment-variable header goes to a non-loopback `http` Collector in clear): fixed. Lines 268-274 add the `otel_header_plaintext` gauge and the `OtelHeaderPlaintext` alert after 1 hour. The case becomes an error in release 0.4.0 with `SAGAT_OTEL_ALLOW_PLAINTEXT_HEADERS=true` as the opt-out. Lines 1236 and 1343-1347 carry it into the risk table and the task.

1. Major — The custody containment check compares text while the overlap check compares path segments, so a fragment can still grant `sagat/credentials/`
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 1060-1064 (Technical Plan 7), and the requirement at lines 419-422.
   What: Line 1060 says only that every grant's path "must start with" one of the policy's declared prefixes. Line 1062 compares path segments, but only for the overlap check. A checker built from this text will test containment with a plain string prefix. Failing scenario:
   - A feature's allow-list declares the custody prefix `{kv_mount}/data/sagat/cred`.
   - Compared by segments, `cred` is not a prefix of `credentials`, so this does not overlap the platform's `{kv_mount}/data/sagat/credentials/`.
   - The fragment grants `read` on `{kv_mount}/data/sagat/credentials/*`, or on the glob `{kv_mount}/data/sagat/cred*`, and lists the grant in its own allow-list.
   - As text, the grant starts with the declared prefix, so the checker passes it.
   - That feature's token can then read the platform's KV credential custody. This is the attack of the earlier Major finding 2, through a different door.
   Correction:
   - Every declared prefix must end with `/`.
   - A grant is contained only when the prefix's segments are a leading run of the grant's segments.
   - Also refuse any grant whose path, read as an OpenBao glob (`*` and `+`), could match a path under another owner's prefix.
   - Add a negative fixture for this case: prefix `{kv_mount}/data/sagat/cred` with a grant on `{kv_mount}/data/sagat/credentials/*`.

2. Nit — The search-path audit row covers only "every function of this design", but later features register `SECURITY DEFINER` count functions
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 845-854, and the re-seal inventory at lines 875-881.
   What: Line 846 counts "every count function an entry registers" among the functions that must pin their search path. Those entries are added by other features in their own deliveries (line 875). The audit row at lines 853-854 lists only the functions of this design. A later entry's `<entry>_sealed_count` or `<entry>_count_below` can therefore be `SECURITY DEFINER` without a pinned search path and still pass the audit. Inside `sagat_app`'s session, an unqualified relation in its body could then be shadowed by a temporary table that `sagat_app` creates. That could make a count return zero, and a zero count lets a passage finish or a key version be retired.
   Correction: Make the audit row generic. It should fail for any function in the platform's schemas where `prosecdef` is true and `proconfig` lacks `search_path=pg_catalog, pg_temp`. It should also check every `regprocedure` listed in `key_authority_reseal_inventory`.

3. Nit — Step 9 stores the ring "under the same custody as those backups", which recreates the exposure the step exists to close, and its reason for keeping the ring is wrong
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 183-185, and the way back at lines 191-198.
   What: Step 9 explains that the ring together with a pre-migration backup reads current secrets. It then tells the operator to keep the ring under the same custody as those backups, so whoever can reach the backup store gets both halves. The step also says "the way back needs it too". That is wrong: after `finish-migration` there are zero `local` rows (line 871), so a return re-seals under any ring. The phrase invites the operator to put the archived ring back into the cluster on the way back. Current values would then be sealed again under a ring that also opens the old backups and may already have been exposed.
   Correction:
   - Hold the archived ring under a custody separate from the backups (different access and location), with the same retention.
   - Drop "the way back needs it too".
   - In step 2 of the way back, say that a return uses a freshly generated ring key id, never the archived ring.
