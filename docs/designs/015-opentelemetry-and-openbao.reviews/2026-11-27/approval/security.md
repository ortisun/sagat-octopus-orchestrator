# Approval read of design 015: security (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: a6002f91db92d8df89bb5487d6e9e6cbaa20ef05
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 0, Nit 2

Previous re-read:
- 1 (Major, `scripts resign` signed an artifact without checking the signature it already had): fixed in the text, but finding 1 below is a remaining gap.
  - docs/designs/015-opentelemetry-and-openbao.md lines 436-451 make `scripts resign` check an existing signature row before it signs. The check runs in the command's process, over `script_signing::message(tenant, digest, language)`, with a key a runner trusts.
  - An artifact with no row that verifies is never signed. It is counted, audited as `key_authority.scripts_resign.unverified` and reported, and the command exits non-zero. The operator then rebuilds it or quarantines it.
  - Line 1130 repeats the rule in the commands table, and line 1131 adds `scripts quarantine-artifact`.
  - Lines 1060-1066 add the table `script_artifact_quarantine`. It is insert-only, only the schema-owning role writes it, and `sagat_app` has `SELECT` only.
  - Task 015-15 (lines 1673-1678) has the test for each passage with random signature bytes.
- 2 (Nit, the new registry column and argument had no bounds, grants or classification): fixed in the text, at docs/designs/015-opentelemetry-and-openbao.md lines 176-191.
  - The CHECK allows at most 64 values of exactly 16 bytes, and `instance_register` applies the same bounds.
  - The seven-argument signature is `SECURITY DEFINER` with the pinned path, revoked from `PUBLIC` and granted to `sagat_app`. The six-argument signature is kept under ADR-0131 and stores no values.
  - `sagat_app` gets no `SELECT` on the column. The column is classified `protected_system`, and the privilege audit gains rows for all of this.
  - Line 963 adds the seven-argument function to the pinned-path list.
- 3 (Nit, the material check would have sent the ring key to PostgreSQL, and only `secret_versions` was sampled): fixed in the text.
  - Lines 145-157: the lock is the first statement of the command's transaction. The sample is opened in the command's process, the plaintext is wiped and never logged, and `key_authority_set_mode` receives key ids and check values only. A refusal names the key id only.
  - Lines 1005-1011 add `<entry>_local_sample(key_id)` as a fifth registered function and `ResealTarget::open_sample`.
  - Line 963 adds `<entry>_local_sample` to the pinned-path list.
  - Tests: Task 015-12 lines 1600-1607 has the barrier test and the test that the ring key never appears in the database logs. Task 015-34 lines 1805-1813 adds the second-entry test.
- 4 (the remaining Nit of the previous re-read): reread-7/security.md has only three findings, and all three are resolved above. There is no fourth finding to check.

1. Nit — A `sagat_app` holder can take the target signer's slot with a forged signature row, which avoids the new check and the counts it relies on
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 433-436 and 443-446 (changed text). The text it relies on: lines 1040-1047 (`signed_at ... DEFAULT clock_timestamp()`, primary key `(tenant_id, digest, key_id)`, `sagat_app` gets `SELECT, INSERT`), lines 427-428 (the endpoint serves the newest trusted signature by `signed_at`) and lines 1053-1056 (the counts are made in SQL).
   What:
     - The fix says an artifact that fails the check "has no signature by the target signer", so `finish-migration` and `finish-return` stay refused. A holder of `sagat_app` can make that untrue.
     - In `migrating`, they insert into `script_artifact_signatures` a row with key id `transit-v<N>` (or a future `transit-v<N+k>`) and 64 random bytes. They can do this for their own planted artifact or for any tenant's genuine one.
     - In `returning`, they do the same with the current local key id.
     - `scripts resign` then skips the artifact without checking it, because it already has a target-signer row at that key id, and the primary key makes the platform's insert a no-op.
     - The SQL count treats the forged row as a signature, so the passage finishes.
     - The endpoint serves the forged row first, because the writer can set `signed_at` in the future. Every runner refuses it, so tenant B's script stops running after the passage. The forged row is permanent, since the table is insert-only.
     - Nothing gets to run that should not, because the runner still verifies. What the attacker gets is denial of service for chosen tenants, and the problem is not reported.
   Correction:
     - When the artifact already has a row for the target signer and key id, `scripts resign` checks that row too. A target row that does not verify is counted, audited and reported as unverified, and the command exits non-zero, so the operator quarantines or rebuilds the artifact.
     - The `BEFORE INSERT` signer trigger sets `signed_at` to `clock_timestamp()` whatever the writer sends.
     - The same trigger refuses a `transit` row above the newest `sagat-scripts` version recorded by `key_authority_record_rotation`.
     - Add to Task 015-15 a test for each passage with a forged row at the target key id on a genuine artifact.

2. Nit — The rebuild path compares only the bytes with the stored artifact, but the signature also covers the language
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 446-448 (changed text) and Task 015-15 line 1677. Context: crates/application/src/script_signing.rs lines 10-11 and 33-40 (the digest covers the bytes only, and the message binds tenant, digest and language so that "the same bytes cannot be run as another language"). crates/infra-postgres/src/scripts.rs lines 1393-1399 (an existing artifact row with the same digest is kept as it is, `ON CONFLICT (tenant_id, digest) DO NOTHING`).
   What:
     - A holder of `sagat_app` can read tenant B's sources and predict the bytes of a deterministic build.
     - They plant an artifact row for B with those exact bytes, the matching digest, another `language` and a random signature.
     - When B's build finishes, the "digest already stored" path compares only the bytes, which match, and signs.
     - If it signs over the stored row's language, B's code is signed to run as another language, which is the invariant the message was designed to protect. If it signs over its own language, the runner refuses the artifact, and B's script never runs.
   Correction:
     - The rebuild signs only when the stored row's language (and every other field the runner uses to run the artifact) equals what it has just built, and it signs `message(tenant, digest, built language)`.
     - When a field differs, it signs nothing and reports the artifact as unverified, for the operator to quarantine.
     - Add to Task 015-15 a test where the planted row has the same bytes but another language.
