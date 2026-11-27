# Security re-read of design 015 (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: efec195a137e9eb638d5a3ed2de70184ab93541f
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 2, Nit 6

Previous report:
- 1 (rewrap changes `key_id`, which the associated data binds): fixed. In `docs/designs/015-opentelemetry-and-openbao.md`, lines 306-307 put the literal `transit` in the associated data and never the key version. Lines 865-875 give a `transit` row no `key_id` and reserve the id `transit`.
- 2 (`sign_script(digest)` drops the tenant and language): fixed. Lines 324-326 sign `script_signing::message(tenant, digest, language)`, and lines 749-762 define `ScriptSigner::sign(&ScriptMessage)`.
- 3 (the output scan skips Transit rows during an outage): fixed. Line 770 adds `StoredValue::Transit` and lines 772-773 make it `Unavailable`, never skipped. Lines 418-422 refuse the report as retryable, and lines 483-485 add the scan to Success criterion 015-005.
- 4 (rewrap loops forever and its port takes one tenant): partly fixed. Line 744 makes the port take `&[(TenantId, WrappedKey)]` and return a result per item. Lines 474 and 967 make a failure block the retirement. Rewrap still has no stated cursor (line 965, see new finding 7).
- 5 (`min_decryption_version` is check-then-act; backups): fixed. Lines 827-835 add a sealing floor that a trigger enforces under the state row's lock, and line 967 raises it before counting. Lines 442-445 and 968 cover the restore and the trim limits.
- 6 (losing OpenBao loses every secret): fixed. Lines 438-446 require snapshots, key custody and a paired restore. Lines 310-314 gate rotation on a snapshot, and lines 1029-1039 add the snapshot job.
- 7 (the existing KV custody is ignored): fixed. Lines 69-72 define the three custodies, the table at lines 290-297 routes each write, lines 865-875 give the representation constraint, and lines 1457-1458 derive `SecretView.provider` from the custody.
- 8 (moving the intent HMAC to Transit is underspecified): fixed. Lines 373-383 add binding version 2 over the metadata and the value's SHA-256. Lines 896-900 add the columns and the trigger, and line 1144 changes the chart.
- 9 (the policy test is too strict and too weak): fixed for the platform's own policies. Lines 390-402 and 1020-1027 make it an exact allow-list that includes the token self-paths, line 403 validates the mount names, and line 993 adds secret cleanup. A new gap in the fragments is new finding 2.
- 10 ("its own token per custody" cannot be built): fixed. Lines 999-1004 run one Agent per custody, and so does Task 015-23 (lines 1448-1450). What remains is the accepted Gap 015-005.
- 11 (the sandbox runner gets an OpenBao credential): fixed. Lines 326-331 serve the keys from the control plane, with a refresh and eviction, and line 994 gives the runner no token.
- 12 (a cache that expires from insertion): fixed. Lines 552 and 1046-1053 add the per-tenant bound, and lines 1070-1077 add the background refresh.
- 13 (rotation plus rewrap keeps the data keys): fixed. Lines 315-317 and 966 add rekey, and lines 974-976 add the runbook procedure after a token is stolen.
- 14 (the Collector bearer token can be sent in clear): partly fixed. The file-based header is refused over plain `http` (lines 257-260, 648). The environment-variable header is still sent to a non-loopback `http` endpoint with only a warning (lines 260-262, see new finding 8).
- 15 (redaction does not cover the new material): fixed. Lines 267-271 and 653-656 add the names and shapes, lines 245-246 say the log body is protected only by the in-process scrub, and lines 1435-1436 extend the canary.
- 16 (feature 031's keys in the platform's OpenBao): fixed. Lines 447-452 keep them outside it, and line 1146 adds the NetworkPolicy check.
- 17 (the OpenBao client is specific to KV): fixed. Lines 424-433 add a timeout per attempt within a total budget and the lookup cache, and lines 766-768 add one client per mount.
- 18 (core dumps and wiping): fixed. Lines 1098-1100 use `rustix` for `RLIMIT_CORE` and `PR_SET_DUMPABLE`, and line 768 parses into a type that wipes on drop.
- 19 (the derivation context encoding is not fixed): fixed. Lines 84-85 fix it, and line 1342 adds a golden test.
- 20 (the new commands and metrics clash with existing ones): fixed. Line 956 extends `reseal-secrets`, and lines 425-428 and 1082 keep one metric family.
- 21 (versions compared as text): fixed. Lines 865-869 add the integer `wrapped_key_version`.
- 22 (the rollout gate is underspecified): fixed. Lines 113-115 name the process kinds, and lines 1104-1108 make an older binary refuse a `transit` row. I checked this against the code: `crates/infra-postgres/src/secrets.rs:57` treats a row with no key id and no provider reference as corrupt.

1. Major — The output scan under `transit` is narrowed to the versions current since the lease began, so a value from an earlier attempt reaches the journal
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 1055-1065 (Technical Plan 8) and Task 015-30 (lines 1485-1490). This contradicts ADR-0152 decision 6 and the guarantee in the comment at crates/application/src/worker/secrets.rs lines 227-229 ("for every version of the secret that is still stored").
   What: Today the scan opens every retained version. The design now opens only "the secret version current when the attempt's lease began and every secret version created since", and keeps the full scan for `local` rows only. Under `transit` every row is `transit`, so after the migration a secret gets a weaker scan than before. Failing scenario:
   - A long-lived SDK worker builds a database client from `db_password` version 1 during attempt 1 of run A.
   - The tenant writes version 2. During the rotation overlap, version 1 still works at the target database.
   - Attempt 1 of run B is leased after version 2 exists. The same worker process fails and puts its connection string, with the version 1 password, into the step output.
   - The scan opens only version 2, accepts the report, and the old password is stored in the journal. Anyone with run read access can then see it, without `secret:read`.
   The narrowing saves no round trip, since Transit's `batch_input` unwraps every retained version in the same single call.
   Correction: Under `transit`, scan every retained version through the one `unwrap_batch` call, as for `local` rows. Drop `secret_versions_since`, and invert the Task 015-30 test "a secret version older than the lease start is not opened" into "is opened". During an outage, a report with uncached older versions is then refused as retryable, which fails closed.

2. Major — The feature fragments' allow-lists certify themselves, so a fragment can grant another custody's paths, including Transit's
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 403-406 (Requirement 015-016) and lines 1020-1027 (Technical Plan 7). Also docs/adr/ADR-0152-openbao-transit-holds-the-platform-keys-and-each-custody-has-its-own-mount-and-token.md decision 5.
   What: The checker compares each `.hcl` with the `.allow.json` beside it, and both are written by the same feature in the same delivery. The fixed refusals (`sys/`, other `auth/` paths, wildcards in a mount segment, `keys/*/config|rotate|trim`, `export/`, `backup/`, `restore/`) do not stop a fragment granting `update` on `sagat-transit/decrypt/sagat-secrets` or `read` on `secret/data/sagat/credentials/*`, if its allow-list lists the grant. For example, a design 047 broker fragment carrying that grant passes the test. The broker's role could then unwrap the data keys of every tenant. "Each custody MUST have its own mount or path prefix" is therefore not enforced by the test that is meant to enforce it.
   Correction: Each allow-list declares its custody prefix or prefixes. The checker then:
   - refuses any grant outside the fragment's declared prefix;
   - refuses prefixes that overlap between fragments or with the platform's own (the Transit mount and `sagat/credentials/`);
   - has a negative fixture for a fragment that grants `sagat-transit/decrypt/sagat-secrets`.

3. Nit — The database enforces the sealing floor but not the sealing version as a ceiling
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 827-830 (the `secret_versions` trigger) and line 965 (rewrap "the target is at most `seal_version`", a check in the command only).
   What: Requirement 015-012 (lines 310-312) says a new key version "MUST seal nothing until the operator attests" a snapshot. The trigger refuses a `wrapped_key_version` below `min_seal_version` but accepts one above `seal_version`. A seal or rewrap at the latest key version is then committed, and a loss of OpenBao before the next snapshot loses it. This can come from a code defect, a `datakey` or `rewrap` call without `key_version`, or a compromised control plane, whose token permits both. The trigger already reads `seal_version` under the same lock.
   Correction: Also refuse `wrapped_key_version > seal_version` in the `secret_versions` trigger and in every inventory entry's trigger. Add a barrier test with a seal at `seal_version + 1`.

4. Nit — The `SECURITY DEFINER` trigger functions have no pinned search path
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 810-814 and Task 015-12 (lines 1349-1350).
   What: `key_authority_share()` gets `SET search_path = pg_catalog, pg_temp`. The trigger functions are `SECURITY DEFINER` too and run as the schema owner inside `sagat_app`'s session, but no search path is stated for them. Two consequences follow:
   - With the session's search path, an unqualified relation in their bodies can be shadowed by a temporary table that `sagat_app` creates.
   - With the pinned path copied over, the unqualified call `key_authority_share()` no longer resolves.
   Correction: State that every `SECURITY DEFINER` function, the trigger functions included, sets `search_path = pg_catalog, pg_temp` and qualifies every object with its schema. Add a privilege-audit row that lists `proconfig` for each.

5. Nit — Inbound webhook verification would tell an unauthenticated sender about the platform's key service
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 413-414 and 1165-1166 ("`500` on the other two"), with the error codes and remediation text of lines 1170-1171.
   What: Today an unknown endpoint, a bad signature and a missing secret all answer one plain `401`. The comment at crates/application/src/events.rs line 873 says of the missing secret: "Not the sender's fault and not theirs to learn". The design answers `Refused` and `Denied` with `500`, and the generic mapping (lines 1157-1159) carries `KEY_AUTHORITY_REFUSED` or `KEY_AUTHORITY_DENIED` with remediation text. Any internet caller holding an endpoint URL then learns that the platform's OpenBao policy refuses it, or that a stored secret is corrupt.
   Correction: For inbound webhooks, answer `Refused` and `Denied` with the same plain `401` as `Refusal::NoSecret` today, or with a bare `500` without a code or body. Keep the audit record and the alert on the operator's side. `Unavailable` stays a `503` with `Retry-After`.

6. Nit — Nothing says what happens to the ring after step 8, though pre-migration backups stay readable with it
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 176-179 (way in, steps 7 and 8), line 1210 (Constitution Check V) and Task 015-22 (lines 1440-1447).
   What: Step 8 removes the ring from the processes. The Kubernetes Secret `secrets.existingSecret` and any copy the operator holds still exist, and every PostgreSQL backup taken before `finish-migration` holds `local` rows sealed under that ring. Values not changed since the migration have the same plaintext today. The ring plus an old backup therefore reads current secrets, outside OpenBao. That contradicts "no platform key outside OpenBao" in the Constitution Check.
   Correction: Give the key-authority runbook a ring-disposal step after step 8. The ring is held offline under the custody of the backups for as long as a backup older than `finish-migration` is retained, then destroyed. The runbook should also say that migrating does not protect values against a ring that was already exposed: those values must be rotated at their source.

7. Nit — Rewrap still has no stated cursor, so rows that cannot be rewrapped can stall it (earlier finding 4, residual)
   Where: docs/designs/015-opentelemetry-and-openbao.md line 965.
   What: `secrets rewrap` selects "values whose wrapped key version is below the target, batches of 250 ordered by id". A row that fails per item keeps its old version and is selected again. With 250 failing rows at the lowest ids, every batch is the same 250 and the pass never finishes. `rekey` (line 966) says "with an id cursor"; `rewrap` does not.
   Correction: Add "with an id cursor (`id > $after`) within a run, counting failures per item" to the rewrap row. Add a test with 250 rows that fail at the lowest ids and a rewrappable row after them.

8. Nit — The header from `OTEL_EXPORTER_OTLP_HEADERS` still goes to a non-loopback `http` Collector in clear (earlier finding 14, residual)
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 260-262.
   What: Only the header read from the file is refused over plain `http`. The environment-variable header keeps today's behaviour with a single warning at start, and no release is named in which it becomes an error. A bearer token set this way crosses the network unencrypted for as long as the configuration stays. The chart path is safe: line 1147 requires `https`, and line 1150 delivers the header by file.
   Correction: Count the case (`otel_header_plaintext_total` or a gauge) and alert on it. Name in the release notes the release in which it becomes the same configuration error as the file case, with `SAGAT_OTEL_ALLOW_PLAINTEXT_HEADERS=true` as the explicit opt-out.
