# Second correctness re-read of design 015 (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: ad991de25901bf75bc40aa3c8a5d85d36debbe54
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 3, Nit 0

Previous re-read:
- 1 (the logical identity includes the custody): fixed. Design lines 528-533 (Requirement 015-022: the version row id and the secret version number only) and lines 1506-1509 (Task 015-10: re-seal barrier tests in both directions).
- 2 (a read racing a rewrap and then a retirement is classified `Refused`): fixed. Lines 1217-1226 and the barrier test of Task 015-19 at lines 1643-1646. The retry it adds is incomplete after a re-key (new finding 1).
- 3 (a re-key that loses its guard keeps the compromised data key): fixed. Line 1076 (retry at most 3 times, then `key_authority_count_issued_before(T)`, non-zero exit), lines 951-955 and 962-968 (the fourth entry function), and the barrier test at lines 1584-1586.
- 4 (`begin-migration`'s guard is not under the state row's lock): fixed for the key-id guard and the registry check. Lines 145-148, 152, 157-163, 223-228 and the barrier test at lines 1532-1537. The sample-open guard added in the same revision cannot run where line 145 puts it (new finding 2).
- 5 (artifacts built by a 0.2.x control plane get no signature row): fixed. Lines 991-996, Task 015-12 at lines 1541-1542, and Task 015-15 at line 1591. `crates/infra-postgres/src/scripts.rs:1396-1399` inserts with `ON CONFLICT (tenant_id, digest) DO NOTHING`, so the `AFTER INSERT` trigger fires only for a new artifact. `key_id_of` (`crates/application/src/script_signing.rs:44`) gives 16 hexadecimal characters, which matches the `local` constraint.
- 6 (a replay of a completed intent is refused once its key is gone): fixed for the way in and for a Transit retirement. Lines 438-444, 153, 155, 1082 and Task 015-16 at lines 1604-1610. A binding-version-1 replay after a full return is still answered wrongly (new finding 3).
- 7 (recording ring ids inside a re-seal's transaction): fixed. Lines 220-222 and Task 015-12 at line 1538.

1. Major — The retry after a refusal reopens with only the current wrapped key, so a re-key followed by a retirement still fails a healthy row
   Where: `docs/designs/015-opentelemetry-and-openbao.md` lines 1221-1226 (Technical Plan 8), line 1076 (`rekey` writes a new data key, nonce and ciphertext), line 1085 (the compromise runbook: rekey, then rewrap and retire), and Task 015-19 at lines 1643-1646.
   What: The reader opens once more "with the current wrapped key". A rewrap changes only the wrapped key, so that is enough for the case the test covers. A re-key changes the wrapped key, the nonce and the ciphertext; its guard at line 1076 is `wrapped_key = $old AND nonce = $old_nonce`. Scenario, following the runbook:
   - A step report reads row X at t0: a version-3 wrapped key, nonce n1, ciphertext c1.
   - `secrets rekey --issued-before T` commits X with a version-4 data key, nonce n2 and ciphertext c2.
   - `secrets retire-key-versions --below 4` sets `min_decryption_version=4`.
   - The report's `unwrap_batch` sends the version-3 key, and Transit refuses it.
   - The second read returns the version-4 wrapped key, which differs, so the reader retries. Read literally, it unwraps the version-4 key and decrypts c1 under n1. The authentication fails, the failure is classified as an integrity refusal, and the attempt fails with no retry and an alert. Requirement 015-022 says a re-key in between must not fail the reader.
   Task 015-19's barrier test runs a rewrap and then a retirement only, so it passes with this defect.
   Correction: State that the retry opens the whole current sealed value from the second read (wrapped key, nonce and ciphertext), and that the value returned is the one from that read. Add a variant of the Task 015-19 barrier test that runs `secrets rekey` and then `secrets retire-key-versions` between the first read and the Transit call, and expects both readers to succeed.

2. Major — The sample-open guard of `begin-migration` cannot run inside `key_authority_set_mode`, so as written it is checked before the lock
   Where: lines 145-148 ("Every guard of these four commands runs inside `key_authority_set_mode`, after it has taken the state row `FOR UPDATE` ... none is checked before the lock"), line 152 (one `local` value per key id "is opened with the command's key of that id and discarded"), lines 223-226, lines 962-964, and Task 015-12 at lines 1532-1537.
   What: `key_authority_set_mode` is a database function. The ring is held only by the command process, so the sample can be opened only in the command, between statements. The design gives no way to take the lock before that open, other than the function that also changes the mode. The natural implementation therefore reads `key_authority_local_key_ids()` and the samples, opens them, then calls `key_authority_set_mode`, which locks and checks only that the ids are in the ring. Scenario:
   - `begin-migration` samples and opens ids K1 and K2.
   - A second operator's `reseal-secrets` (no `--to`, today's `local` behaviour; a one-shot command that publishes no registry row) re-seals rows under the newest key of its ring, K3.
   - The first command's ring has an id K3 with other material. The id check under the lock passes, and the mode becomes `migrating`.
   - The K3 rows open with no ring the command holds, and no command leaves `migrating` without re-sealing them. This is the stranding the previous finding 4 described.
   The barrier test at lines 1536-1537 covers only the key-id guard, so it cannot catch this.
   Correction: Have the command take the state row `FOR UPDATE` as the first statement of its transaction (it runs as the schema-owning role). Then, in the same transaction, it reads the key ids and one sample per id, opens them, and calls `key_authority_set_mode`. Alternatively, pass the set of sampled key ids to `key_authority_set_mode` and refuse when the ids it reads under its lock differ. Change line 145 to match. Add a barrier test: hold `begin-migration` after its sample, commit a `local` re-seal under a new key id, release, and expect the command to be refused.

3. Major — After a full return, a replay of a binding-version-1 intent from before the migration is answered `IdempotencyKeyReused` instead of as an expired request id
   Where: lines 438-444 (Requirement 015-014: "the file key for binding version 1"; an expired answer only when "its binding key is no longer available"), way-back step 2 at lines 209-210 (a fresh intent key file), line 128 (in `returning` new intents bind at binding version 1), and Task 015-16 at lines 1604-1610.
   What: A binding-version-1 row records no identity of the file key that made it. Today the replay recomputes the binding and compares the bytes before it checks expiry (`crates/infra-postgres/src/managed_secrets.rs:127-133`). Scenario:
   - Intent A is activated in `local` under file key F1.
   - Way in: step 7 removes F1.
   - Way back: step 2 installs a fresh key F2, and `finish-return` brings the deployment to `local`.
   - The client retries A with the same request id and the same payload. A file key is now available, so the rule "binding key no longer available" does not apply. The binding recomputed under F2 differs, and the answer is `IdempotencyKeyReused`, a conflict that tells a correct client its key was reused with another payload.
   Nothing can tell an F1 row from an F2 row. Task 015-16 tests replays only after the retirement and after step 7, never after a return.
   Correction: Record the time of the latest `finish-migration` in `key_authority_state` through `key_authority_set_mode`. Treat a binding-version-1 intent created before that time as bound under a retired file key. `finish-migration`'s guard already makes every such intent older than 24 hours at that point, so it is answered as an expired request id. That check must run before the binding comparison. Add a replay of a pre-migration intent after `finish-return` to Task 015-16.

I could not run anything. Confidence:
- Finding 1: plausible. It follows from the wording of lines 1223-1224 together with the re-key's guard at line 1076.
- Finding 2: confirmed in the design text. The guard needs the command's key material, which a database function does not have.
- Finding 3: confirmed in the code. `crates/infra-postgres/src/managed_secrets.rs:127` compares the bindings before the expiry check at line 131, and binding version 1 carries no key identity.
