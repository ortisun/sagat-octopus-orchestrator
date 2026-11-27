# Seventh security re-read of design 015 (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: ad991de25901bf75bc40aa3c8a5d85d36debbe54
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 1, Nit 3

Previous re-read:
- 1 (the inventory could not report the key ids of its `local` values, and the check did not run under the state row's lock): fixed in the text.
  - docs/designs/015-opentelemetry-and-openbao.md lines 145-148 put every guard of the four mode commands inside `key_authority_set_mode`, after the state row is taken `FOR UPDATE`.
  - Line 152 checks the key ids from `key_authority_local_key_ids()`.
  - Lines 948-955 add `<entry>_local_key_ids() RETURNS SETOF text` as a fourth owner-role `SECURITY DEFINER` function with the pinned path. It is a `regprocedure` column of `key_authority_reseal_inventory`, so the generic audit row at lines 917-921 reaches it.
  - Lines 908-910 add it to the pinned-path list. Lines 962-964 combine it under the `FOR UPDATE`.
  - Task 015-12 (lines 1532-1537) has the barrier test. Task 015-34 (lines 1723-1726) builds it.
  - Findings 2 and 3 below are new gaps in this fix.
- 2 (a ring key with the right id but other material passed the check): fixed in the text, at line 152, lines 224-227, and Task 015-12 lines 1534-1535. Each key id's sample `local` value is opened with the command's key and the command is refused when it does not open. The live-process guard at lines 157-163 adds a second barrier. Finding 3 below is about how the opening is done and which entries it reaches.

1. Major — `scripts resign` signs any artifact that lacks the target signer's row without checking the signature it already has, so a holder of `sagat_app` gets their own code signed for any tenant at each passage
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 390-396 and 991-996 (changed text: the first signature row is copied by the database from the artifact's own `signature` and `key_id`, which the writer supplies). Lines 405-408 and line 1067 (`scripts resign`, unchanged text). The grant `GRANT SELECT, INSERT ON script_artifacts TO sagat_app` is at crates/infra-postgres/migrations/20261014000000_scripts.up.sql line 303. I raise this outside the D015-022 changes because the new trigger is now the only way a first row is written, and the damage is unauthorized script execution, which docs/architecture/security-and-sandbox.md line 44 rates critical.
   What: Failing scenario:
     - The attacker holds the `sagat_app` credential but no signing key. In `local`, they insert into `script_artifacts` an artifact for tenant B, with bytes of their choice, a matching digest, any 16-hexadecimal `key_id` and 64 random bytes as the signature.
     - `script_artifacts_first_signature` copies these as a `local` row, which `local` mode accepts. Runners refuse the artifact, because the signature does not verify.
     - The operator then runs `begin-migration` and `scripts resign`. Resign sees no `transit` row and has Transit sign `script_signing::message(B, digest, language)`. Every runner now trusts the attacker's code for tenant B.
     - The same happens in the other direction. An artifact inserted in `transit` with `key_id = 'transit-v<N>'` and random bytes is signed with the local key by `scripts resign` in `returning`.
     - Resign skips every check the normal signing path makes (security-and-sandbox.md line 100: the bundle must match its version, under the key handed out at the claim). It also ignores the tenant restriction of Gap 118.
   Correction:
     - Before it signs an artifact, `scripts resign` verifies one of the artifact's existing signature rows over `script_signing::message(tenant, digest, language)`, with the key a runner would trust: the legacy local public key by key id, or Transit `verify` at that key version.
     - An artifact with no row that verifies is never signed. It is counted, audited, and reported with a non-zero exit, so the passage cannot finish until an operator removes it or rebuilds it.
     - Add to Task 015-15 a test for each passage in which an artifact carries random signature bytes, and resign refuses it.

2. Nit — The registry's new `ring_check_values` column and `instance_register` argument have none of the bounds, grants or classification the rest of the registry has
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 157-160 and Task 015-12 lines 1548-1549. Lines 1013-1023 (storage classification) do not list it. In crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql, every column is bounded (lines 38-44), and the existing signature is revoked from `PUBLIC` and granted only to `sagat_app` (lines 387 and 405). No migration sets `ALTER DEFAULT PRIVILEGES`.
   What:
     - Adding an argument creates a new `SECURITY DEFINER` overload. PostgreSQL grants `EXECUTE` on it to `PUBLIC` by default, so any role in the database could register instance rows unless the migration revokes it. The design does not say whether it does, or what happens to the six-argument signature.
     - `bytea[]` has no limit. A holder of `sagat_app` can fill up to 1,000 rows with values of any size, which slows every scan of the registry and the read `begin-migration` makes under its lock.
     - The design does not say that `sagat_app` gets no `SELECT` on the new column. The current grant is column-level, which helps.
     - Forged rows only make `begin-migration` refuse (it fails closed), but that refusal is a denial of service anyone with `EXECUTE` can cause.
   Correction:
     - State the constraint `cardinality(ring_check_values) <= 64` and that every element has `octet_length = 16`, checked in the function and in a CHECK.
     - State `REVOKE ALL ... FROM PUBLIC` and `GRANT EXECUTE ... TO sagat_app` on the new signature, and drop or keep the old one according to ADR-0131's expand and contract rule.
     - State that no `SELECT` is granted on the column to `sagat_app`.
     - Add the column to storage classification as `protected_system` with its CHECK as evidence.
     - Add privilege audit rows for these.

3. Nit — The material check is said to run "inside `key_authority_set_mode`", which would mean sending the ring key to PostgreSQL, and no inventory part returns a sample value for any entry other than `secret_versions`
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 145-146 and line 152, lines 224-225, lines 948-960 (the four functions and `ResealTarget`, none of which returns or opens a sample), and Task 015-34 lines 1723-1726.
   What:
     - Opening a `local` value needs the AES key. Doing it inside a SQL function would put key bytes in statement parameters, where `log_statement`, error logs and `auto_explain` can record them.
     - Read literally, the text invites this. Otherwise the opening happens in the command, where the design does not place it under the lock.
     - The open also needs the row and its associated data for each entry. Neither the registered functions nor `ResealTarget` provide them. An implementer will therefore sample only `secret_versions`.
     - Failing scenario: a key id seals only design 040's `audit_sinks` values. The command's ring holds that id with other material, and no live process holds the real key, so the registry guard does not catch it either. The wrong check value is recorded, `reseal-secrets --to transit` refuses the real key, and the installation is stuck in `migrating`. This is the same gap previous finding 1 closed for key ids, now for key material.
   Correction:
     - Say that the command, running as the schema-owning role, takes the state row `FOR UPDATE` first in its own transaction. Still in that transaction, it reads one sample per key id and opens it in its own process, then calls `key_authority_set_mode` with only key ids and check values. The ring key never reaches the database.
     - The sample's plaintext is wiped and never logged, audited or printed. A refusal names the key id only, never the tenant, secret or row.
     - Add `open_sample(key_id)` to `ResealTarget`, or a fifth registered function `<entry>_local_sample(key_id)` that returns the sealed row, so every entry is sampled. Add both to Task 015-34.
     - Add a test with a second entry whose only key has the right id and other material, and see the command refused.
