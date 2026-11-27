# Correctness re-read of design 015 (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: a7cd7ac1d3cff9e207cd53336914ae8ddac5329b
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 6, Nit 1

Previous report:
- 1 (older engine skips the scan on a `transit` row): fixed. Design lines 324-325, 933-937 and 1210-1213. At `crates/infra-postgres/src/secrets.rs:57`, `stored_from` returns `corrupt("local secret key")` for a row with no key id and no provider reference, so an old reader fails closed.
- 2 (rotation outruns the snapshot): fixed. Lines 347-351, 893-898, 1029-1030, 489-493 and 535-538. Standby regions are deferred to Task 015-32 under Owner decision 2026-11-27-150.
- 3 (retirement relies on a sleep): fixed. Lines 861-876 and 1033, the barrier tests of Task 015-12, and Success criterion 015-002.
- 4 (allow-list and port gaps): fixed. Lines 427-438, 787-809, 992-994 and 1055.
- 5 (re-key selector and token revocation): fixed. Lines 1032 and 1040-1042. The test of a row rewrapped before the re-key is not listed, but the selector on `data_key_issued_at` makes it structural. A concurrent rewrap still defeats it (new finding 3).
- 6 (deliveries fail on rewrap and re-seal): partly fixed. Lines 500-502 and 843-844. Custody is part of the compared identity, so a re-seal still fails deliveries (new finding 1).
- 7 (intent replay across a rotation): partly fixed. Lines 410-420 and 964-965. A replay of a completed intent after its key leaves is still refused (new finding 6).
- 8 (script-signing migration and re-sign): fixed. Lines 362-390 and 945-960. An old control plane writing artifacts during the upgrade is a new gap (new finding 5).
- 9 (runner artifact cache): fixed. Lines 367-368, Task 015-15, and Success criterion 015-007 at lines 537-538.
- 10 (finishing the migration is check-then-act): fixed for the finish commands. Lines 103-113, 145-150 and 893-898. The begin command's guard is new finding 4.
- 11 (custody per secret versus per version row): fixed. Lines 904-910, 933-937 and 980-983.
- 12 (one error kind only): fixed. Lines 454-469 and 1259-1281.
- 13 (cache sizing and outage claims): fixed. Lines 1152-1183.
- 14 (no reader configured skips the scan): fixed. Lines 113, 468-469 and 822-823.
- 15 (rewrap audit per run): fixed. Lines 792, 924 and 1031.
- 16 (readers not listed or tested): fixed. Lines 457-461 and 827-833. Success criterion 015-005 lists outbound webhook signing and the broker and connector credentials, but Task 015-19's outage tests do not name them; Task 015-10 tests each site's mapping.
- 17 (stale registers and ADR text): fixed. `specs/015-opentelemetry-and-openbao/research.md` lines 28-29, and ADR-0152 no longer contains the contradicting phrases.
- 18 (redaction shapes): fixed. Lines 276-279 and 703-706.
- 19 (telemetry configuration against the code): fixed. Lines 698-706 and Task 015-3.
- 20 (two label sets): fixed. Lines 472-478.
- 21 (token cache keyed by time): fixed. Lines 479-480.
- 22 (version column drift, rewrap guard, `transit` key id): fixed. Lines 936-937, 941-942 and 1031.
- 23 (deployment details): fixed. Lines 299-303, 1059-1064, 1145-1148 and Task 015-7.
- 24 (small inaccuracies): fixed. Lines 244, 362, 1181-1183 and 1597.

1. Major — The logical identity includes the custody, so a re-seal still fails the deliveries that Success criterion 015-004 says never fail
   Where: `docs/designs/015-opentelemetry-and-openbao.md` lines 500-502 (Requirement 015-022), 843-844, 1022, Success criterion 015-004 (lines 526-527), Task 015-10 (line 1452) and Task 015-18.
   What: The re-seal updates the version row in place. Today's re-seal is `UPDATE secret_versions SET key_id = $3, nonce = $4, ciphertext = $5 WHERE id = $1 AND key_id = $2` (`crates/infra-postgres/src/secrets.rs:724`), and the design keeps that, guarding per value by the old sealed bytes. The row id and the version number stay the same, but the custody changes from `local` to `transit`, or back on the way back. Requirement 015-022 compares "the version row's id, its version number and its custody". Scenario: a delivery reads version row R as `local`; `reseal-secrets --to transit` commits R as `transit`; the delivery's second read sees a different custody and returns `Unavailable` (`crates/application/src/worker/secrets.rs:200`). The report path at line 291 behaves the same way. With a secret resolved every 10 milliseconds during the re-seal, Task 015-18 and Success criterion 015-004 ("zero failed resolutions") fail. Task 015-10's test "holds a delivery between its reads while a re-seal commits" cannot pass as the requirement is written. This was my earlier correction's wording ("and the custody, if needed"), and it is wrong.
   Correction: Define the identity as the version row's id and its version number only. A move to KV custody through feature 017's `migrate` writes a new version row, so it still changes the identity. Keep the re-seal barrier test of Task 015-10 and run it in both directions.

2. Major — A read that races a rewrap followed by a retirement is classified `Refused` and fails a healthy step for good
   Where: lines 1031-1033 (rewrap, retire), 465-468 (a scan that meets `Refused` fails the attempt with no retry), 1264 (`below_minimum`), 1161-1170 (the scan opens every retained version as `ScanOnly`, so it always calls Transit), and Success criterion 015-002 (line 523).
   What: Delivery and the report path read the stored rows, commit, then open them through Transit, then read again (`crates/application/src/worker/secrets.rs:282-291`). Scenario:
   - A report reads row X with a version-3 wrapped key at t0.
   - The last rewrap batch moves X to version 4 at t0 plus 10 milliseconds.
   - `retire-key-versions --below 4` raises the floor, counts zero and sets `min_decryption_version=4` (with no standbys this takes well under a second).
   - The report's `unwrap_batch`, which can wait up to 5 seconds behind the in-flight bounds of Technical Plan 8, then sends the stale version-3 wrapped key.
   - Transit refuses it, and the design maps that refusal to `Refused`.
   The step fails without its retry policy, and an integrity alert fires for a row that is healthy. A cold-cache delivery and a webhook signing fail the same way. The existing test holds a delivery across a rewrap only, never across a rewrap followed by a retirement.
   Correction: Before classifying a Transit refusal as `Refused`, compare the wrapped key sent with the row's current one (the second read already exists). If they differ, open again once with the current wrapped key, and answer `Refused` only when the current stored value fails. Add a barrier test: hold a report and a cold-cache delivery after their first read, run rewrap and retire, release them, and expect both to succeed.

3. Major — A re-key that loses its guard to a concurrent rewrap silently keeps the compromised data key, and nothing recounts
   Where: line 1032 (`rekey`, guarded by `wrapped_key = $old AND nonce = $old_nonce`, with an id cursor), line 1031 (rewrap), lines 1040-1042 (the compromise procedure), and Requirement 015-012.
   What: The re-key reads row X, unwraps its data key, gets a fresh data key, and updates X only `WHERE wrapped_key = $old`. A rewrap of X committed in between (the operator's routine rewrap, or a second terminal) changes `wrapped_key` but keeps the same compromised data key and the same `data_key_issued_at`. The re-key's update then matches zero rows, the cursor moves past X, and the command ends. Retirement counts by key version, not by issue time, so it passes. The stolen data key still opens X. The design does not say what a guard miss does, and no command counts values with `data_key_issued_at < T` after the re-key. The passages are safe only because their finish commands recount.
   Correction:
   - On a guard miss, re-read the value and retry it while it is still selected (`data_key_issued_at < T`).
   - End `rekey` with a count over every inventory entry (add `<entry>_count_issued_before(T)` to the re-seal inventory) and exit non-zero unless it is zero. Alternatively, make rewrap and re-key exclude one another with a transaction-level advisory lock taken by both commands.
   - Test it with a barrier: hold the re-key between its read and its update, commit a rewrap of the row, release, and expect the row to be re-keyed or the command to fail.

4. Major — `begin-migration`'s key-id guard is not stated under the state row's lock, and a miss strands the deployment in `migrating`
   Where: line 147 (the guard, "checked in the command's transaction"; the finish commands at lines 148 and 150 say "under the state row's lock", this one does not), lines 205-217 (the refusal that "no installation can be left in `migrating`" relies on), line 101 (there is no transition from `migrating` back to `local`), and Requirement 015-010.
   What: Scenario: a serving process's ring seals under key K2, and the command's ring does not hold K2 (for example during a ring rotation, or with a Job built from an older Secret). `begin-migration` runs its query for `local` values whose key id it does not hold, finds none, and only then takes the state row `FOR UPDATE` in `key_authority_set_mode`. A `local` seal under K2 commits in between; its trigger took `FOR SHARE` and released it at commit. The mode becomes `migrating` with a K2 row whose check value was never recorded. Every process holding K2 keeps its gate closed (lines 213-215). `reseal-secrets --to transit` refuses any ring holding K2. Without K2 the row cannot be opened. No command leaves `migrating` except through `finish-migration`, which needs that row gone, so the deployment is stuck. Even with no such row committed, every serving process whose ring holds a key that sealed nothing yet is missing from the command's ring closes its gate at the change of mode, which the design says no passage step may do.
   Correction:
   - State that every guard of the four mode commands runs after the state row is taken `FOR UPDATE`. Local seals take `FOR SHARE` through their trigger, so they then serialise with the guard.
   - Have each control plane and engine publish its ring's check values in its registry row, and refuse `begin-migration` while a live process holds a key whose check value the command's ring does not have.
   - Add a test that commits a `local` seal under an unknown key between the guard and the change of mode, and expects the command to be refused.

5. Major — Artifacts built by a 0.2.x control plane after the migration ran get no signature row and can never run in `local` mode
   Where: lines 370-380 (the trust decision reads only `script_artifact_signatures`; "the migration copies each existing artifact's own signature"), lines 953-954, and line 1023 (`scripts resign` is refused in `local`).
   What: During the rolling upgrade of way-in step 1, the first new control plane runs the migration, and the old replicas keep serving. An old replica finishing a build inserts into `script_artifacts` with its `signature` and `key_id` (`crates/infra-postgres/src/scripts.rs:1395-1412`) and writes no row of the new table. New control planes serve from `/script-runner/artifacts/{digest}` only the newest row of `script_artifact_signatures`, so that artifact has no signature and every step that uses it fails. In `local`, `scripts resign` is refused, so it stays broken until a migration begins. The same thing happens after a rollback of one replica in any mode. In that case it fails closed, but `finish-migration`'s count was already taken.
   Correction: Write the first signature row from an `AFTER INSERT` trigger `ENABLE ALWAYS` on `script_artifacts`. The trigger copies the artifact's own `signature` and `key_id`, with the signer `transit` when the key id matches `^transit-v[0-9]+$` and `local` otherwise. The signer trigger then refuses an old writer's local artifact in `migrating` and `transit`, so it fails closed at build time. New code no longer inserts the first row itself. Add a test that inserts an artifact the way 0.2.x does after the migration, in `local` and in `migrating`.

6. Major — A replay of a completed intent is refused once its binding key is gone or retired
   Where: Requirement 015-014 (lines 410-420), line 148 (`finish-migration`: zero *pending* binding-version-1 intents), line 1038 (`intents retire-key-versions`: counts *pending* intents below `N`), way-in step 7 (the intent key file is removed), and Task 015-16.
   What: Today a replay of an activated intent returns its receipt at any age after the binding check (`crates/infra-postgres/src/managed_secrets.rs:122-136`: the 24-hour expiry applies only to intents that were not activated). The design now verifies a replay against the stored binding version and key version. Two scenarios:
   - After way-in step 7, a client that retries a provisioning request it completed in `local` has no file key to verify against.
   - After `intents rotate-key` and `intents retire-key-versions --below N`, a retry of an intent completed an hour earlier at key version `N-1` meets Transit's `min_decryption_version`, which the design maps to `Refused`.
   Either way a correct retry with the same request id is answered with a 500 `KEY_AUTHORITY_REFUSED`, or with `IdempotencyKeyReused`, instead of its receipt. The design does not state what the answer is.
   Correction: Make the guards of `finish-migration`, `finish-return` and `intents retire-key-versions` count intents of the old binding version or key version that are pending *or created within the last 24 hours*. Define a replay of an activated intent older than 24 hours whose binding key is no longer available as an expired request id (the answer an unactivated expired intent gets today), never as a key-service refusal. Add a test that replays a completed intent after a retirement and after way-in step 7.

7. Nit — Recording retired ring ids inside a re-seal's transaction upgrades a share lock to an update lock
   Where: lines 205 and 856 (`reseal-secrets --to transit` "adds every key id it re-seals away from" through `key_authority_record_retired_ring`, which takes the state row `FOR UPDATE`), and lines 866-867 (the re-seal's trigger has already taken it `FOR SHARE`).
   What: The design does not say in which transaction the id is recorded. If it is recorded in a re-seal's transaction after its update, that transaction holds `FOR SHARE` and then asks for `FOR UPDATE`. Two concurrent re-seal runs deadlock, and PostgreSQL aborts one of them. Any seal that holds `FOR SHARE` makes the re-seal wait for it.
   Correction: Record all the ring's ids once, in the command's first transaction, before any re-seal. This is safe because the record only grows.

I could not run anything. The defects in the design text are confirmed by reading it. Confidence for the others:
- Findings 1 and 5: confirmed in the code at the lines cited.
- Findings 2, 3 and 6: plausible. Each needs the stated interleaving, which nothing in the design prevents.
- Finding 4: plausible. It depends on the order of statements inside the command's transaction, which the design leaves open.
