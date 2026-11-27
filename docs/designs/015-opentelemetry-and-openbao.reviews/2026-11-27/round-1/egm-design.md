# EGM stages 5 to 8 (zoom-out, comprehension, critique) of design 015 (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: 6c2d7b06f5038520a1b36df31cbe0e79689c1ee3
Reviewer: egm-design
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 1, Major 9, Nit 6

1. Blocker — Retiring and trimming a key version checks only `secret_versions` and ignores the other tables of the re-seal inventory
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 205-224 (rewrap, rekey, retire, trim), 729-739 (the re-seal inventory), 830-833 (command table); docs/designs/040-ocsf-event-standard.md lines 555-562 and Task 040-22 (line 868).
   What: The design creates the re-seal inventory for "every table that holds values sealed by the ring or under a Transit-wrapped data key" (line 729). Design 040 plans to move the sealed columns of `audit_sinks` to the `transit` authority (Task 040-22). But an inventory entry has only `<entry>_sealed_count(custody)` and `ResealTarget::reseal_batch(to, limit)`. Neither one counts by Transit key version, and neither one rewraps. `secrets rewrap`, `rekey`, `retire-key-versions` and `trim-key-versions` count and rewrap only "rows", which in the text means `secret_versions`.
   Why: Once a second entry holds Transit-wrapped data keys, `retire-key-versions --below N` raises `min_decryption_version` while that entry's older wrapped keys still exist, and those values stop opening. `trim-key-versions` then deletes the key versions for good. The design built the inventory precisely so that a feature cannot be forgotten, yet the irreversible command still forgets it. That is permanent loss of secret material.
   Correction: Give each inventory entry a count by wrapped-key version (`<entry>_count_below(version)`) and a rewrap and rekey batch in `ResealTarget`. Make `rewrap`, `rekey`, `retire-key-versions` and `trim-key-versions` iterate every registered entry, with the sum checked under the state row's lock. Extend Task 015-34's test so that a second entry holding one row below `N` refuses both retirement and trimming.

2. Major — The data-key cache is keyed by the wrapped key, so every rewrap empties it
   Where: lines 908-911 (cache keyed by "(tenant, wrapped key digest)"), 928-931 (background refresh), 830 (rewrap), 467-476 (CUJ 2), 943-946 (alert `KeyAuthorityRefused`).
   What: A rewrap changes `wrapped_key` and leaves the data key unchanged. Every delivery after a rewrap therefore misses the cache and makes a cold Transit call. The refresh task meanwhile keeps unwrapping the old wrapped keys of entries "used in the last 300 seconds". If retirement follows the rewrap, as CUJ 2 does, those refreshes fall below `min_decryption_version` and come back as `refused`.
   Why: For about 300 seconds after each rewrap the platform has lost the outage cover that CUJ 3 and Owner decision 2026-11-26-116 promise. A hot tenant is throttled at 8 calls in flight (line 926), which puts "no delivery failed" in CUJ 2 at risk. And a routine retirement can fire the "at once" integrity alert and write `key_authority.refused` audit records for rows that are healthy.
   Correction: Key the cache by something a rewrap does not change and a rekey does change, for example (tenant, version row id, `data_key_issued_at`). Make the refresh unwrap the row's current wrapped key, not the cached one. Add a test: after rewrap, retire with a warm cache, and see no cache miss storm and no `refused` outcome.

3. Major — The gate table checks only "the ring", but each passage needs specific local keys
   Where: lines 152-157 (configuration per mode), 279-281 (version-1 intents in `migrating`, version-1 binding in `returning`), 256-259 (only the local key signs in `returning`), 967-969 (capabilities), 993 and 997 (way back, steps 2 and 5); apps/control-plane/src/cli.rs lines 445-446 (without `SAGAT_SCRIPT_SIGNING_KEY` "the script routes do not exist").
   What: In `returning`, new signatures must be local (the trigger refuses a `transit` row) and new intents must be bound at version 1 with the intent key file. In `migrating`, pending version-1 intents can only be verified with the file key. Yet the gate table closes a process only when it "lacks the ring or its Transit configuration". `key-authority-return-v1` is listed "with both the ring and a Transit configuration". Way-back step 2 adds "the ring, and the intent key file" and never the script-signing key, while step 5 says `scripts resign` needs it.
   Why: In `returning`, a control plane that has the ring but no script-signing key stays open and fails every script build, and `finish-return` can never reach zero artifacts without a local signature. A process without the intent key file fails feature 017's writes in `returning` and its replays in `migrating`. The operator following Technical Plan 10 hits exactly this case.
   Correction: State, per mode, which of the three local keys each process kind needs: `migrating` needs the ring and the intent key file; `returning` needs all three on the control plane. Make `key-authority-return-v1` require all of them. Add the script-signing key to way-back step 2, and add the missing cells to Task 015-18's table tests.

4. Major — The database does not record a minimum version for `sagat-scripts` or `sagat-intent`, so two guards cannot work as written
   Where: lines 692-694 (state row columns), 759-761 ("a `transit` row at or above the minimum version"), 288-289 and 824 (`finish-migration` counts under the state row's lock), 834-835 (retire for scripts and intents), 226-229.
   What: The `finish-migration` count of artifacts "without a trusted Transit signature" runs inside `key_authority_set_mode`, a SQL function under a row lock. "Trusted" depends on the script key's minimum version, which lives only in Transit (`key_info`), and a database function cannot call Transit. Separately, retirement of the two keys has no database fence like `min_seal_version`. A signature or a binding that read version `N-1` before the count and commits after it lands below `N`.
   Why: A builder cannot implement the `finish-migration` guard as specified. The retirement race the design closes for secrets (Success criterion 015-002) stays open for scripts and intents, leaving an artifact that cannot run, or a pending intent whose replay is refused, after a retirement that reported zero.
   Correction: Add `scripts_min_version` and `intent_min_version` to `key_authority_state`, raised through an owner-role function before Transit's minimum is set. Have the signature and binding triggers refuse a row below them, as the `secret_versions` trigger does. Have the counts read them. Add barrier tests that mirror Success criterion 015-002.

5. Major — The runner gate imposes an unstated 24-hour wait, and the rollout order leaves it out
   Where: lines 266-273, 764-766 (sightings keyed by (subject, form), deleted after 24 hours), 821 (`begin-migration` guard), 974-980 (way in, steps 1 to 4).
   What: A runner upgraded in step 1 leaves its `absent` sighting in place for up to 24 hours. Step 4, `begin-migration`, is refused until that row ages out. No command clears a sighting once the runner is known to be upgraded, and the rollout order does not mention the wait.
   Why: An operator who follows Technical Plan 10 is refused at step 4 with a list of subjects that are already upgraded, and has no documented remedy other than waiting a day.
   Correction: Either write the wait into the rollout order (step 1, then 24 hours with no `absent` or `configured-v1` sighting), or refuse only on sightings newer than the last `key-list-v1` sighting of the same subject. Say which in Requirement 015-013 and Task 015-15.

6. Major — The Collector credential gets its own KV custody and an Agent sidecar per pod, which is over-built for release 0.3.0
   Where: lines 122-124, 854 (custody table, row "Collector credential"), 863-866 (one Agent per custody), 1158-1160 (Task 015-7 must merge after Tasks 015-17 and 015-23), 1314-1316.
   What: The control plane and the engine get a third OpenBao Agent sidecar, a policy and a KV prefix only to template one telemetry ingest header. The design already has a simpler path for the same file, a Kubernetes Secret mounted by the chart (`telemetry.headersSecret`), used for the database runner.
   Why: Gap 015-005 accepts that a compromised process holds every token it uses, so a separate custody for a low-value ingest token adds no guarantee. It costs a sidecar per pod, a policy fragment and its allow-list, and a dependency between the two lanes that otherwise touch different crates.
   Correction: Use `telemetry.headersSecret` for every process. Drop the `sagat/telemetry/` custody, its policy and its Agent, and remove the order point that ties Task 015-7 to Tasks 015-17 and 015-23. Keep the overlap rotation of line 902-904, which works the same way with a Secret.

7. Major — The strongest simpler alternative is not weighed
   Where: lines 1125-1135 (Alternatives Considered), 1112 (Complexity Tracking, "Only `local`: no rotation by command, keys in the environment"); docs/adr/ADR-0152-openbao-transit-holds-the-platform-keys-and-each-custody-has-its-own-mount-and-token.md line 70.
   What: The obvious middle option is never compared: keep the ring and the two other keys, deliver them from OpenBao KV through the Agent into files the process reloads, and derive per-tenant keys in process with HKDF. It answers both objections the doc raises against `local` (no environment variables, rotation without a redeploy), and it removes the cache, the dependency of each cold open on OpenBao, and most of the outage design. It gives up "the key never leaves OpenBao" and Transit's per-use audit. "Transit `encrypt` of each value" appears only in ADR-0152, not in the design.
   Why: The question "is this the simplest design that meets the requirements" cannot be answered while the main competitor is missing. The Complexity Tracking row is weighed against a weaker alternative than the real one.
   Correction: Add the KV-delivered ring as an alternative and a Complexity Tracking row, naming the guarantee it loses (key material in process memory, no per-use audit) and why that loss matters for regulated work. Copy the `encrypt` option from the ADR into the design.

8. Major — Key words are overloaded or used before they are defined
   Where: Terms, lines 38-57; lines 47, 159-166, 821 ("the requirement `key-authority-ring-absent-v1`"); lines 50 and 722 (`secrets.authority` holds a custody); lines 168, 453 and 910 ("version").
   What: "Requirement" means both a design requirement (Requirement 015-010) and an instance-registry requirement the operator records. "Authority" means both the mode (`key_authority_state.mode`) and a custody (`secrets.authority`). "Version" covers secret versions, Transit key versions, `seal_version`, binding versions and `wrapped_key_version`, and "keys of current versions" (line 910) can be read either way. Gate, capability, heartbeat, passage, sealing floor and schema-owning role are used from line 43 on and never defined.
   Why: A reader new to the project cannot tell whether "the requirement is recorded" refers to this doc or to the registry. A builder of the cache cannot tell which version "current" means.
   Correction: Add terms for registry requirement and capability (citing ADR-0082), gate, heartbeat, passage, sealing floor and schema-owning role. Write "registry requirement" every time. Explain that `secrets.authority` holds a custody, or rename the term. Qualify every "version" (secret version, key version).

9. Major — The modes and passages are described in four places, mechanism first and order last
   Where: Requirement 015-010 (lines 133-166), Requirement 015-015 (lines 283-296), Technical Plan 6 command table (lines 819-835), Technical Plan 10 (lines 954-1005).
   What: The reader meets the four-mode gate table, with heartbeats, triggers and capabilities, five requirements before learning what each passage does, and about 800 lines before the ordered steps. The requirements carry the implementation: Requirement 015-012 alone spans 35 lines of trigger, retry and standby detail. The four descriptions already differ in small ways, for example "a Transit signature" (line 289) versus "a trusted Transit signature" (line 824).
   Why: Restating the same mechanism in four places invites contradictions (findings 3 and 4 hide in exactly these seams), and a new reader cannot follow the passages in one pass.
   Correction: Add one section, "Key authority modes and passages", right after Terms. It should hold a state diagram (`local` to `migrating` to `transit` to `returning` to `local`) and, per transition, the command, its guard, the registry requirement and the configuration each process needs. Requirements then state outcomes and cite that section, and Technical Plan 6 and 10 keep only what is not already there.

10. Major — The Overview, Goals, Risks and Constitution Check claim more than the design delivers
   Where: lines 18 and 22, 413, 454, 806, 1077, 1100, 1102, 1106.
   What: "OpenBao becomes the authority for every key the platform holds" (line 18) contradicts O015-008 (design 042's per-tenant HMAC key stays in the database). "No platform key in an environment variable or a file under `transit`" (line 413) holds only after the operator records `key-authority-ring-absent-v1`, which nothing forces, and none of the four alert rules watches `key_authority_local_key_configured`. The Principle V row, "Keys never leave OpenBao", ignores that plaintext data keys leave OpenBao on every unwrap and are cached for 300 seconds (Gap 015-003). The Principle III row is "n/a" because Transit calls are "idempotent per row", but line 806 says a rotation is not idempotent, and the started/completed reconciliation is exactly Principle III's consultation pattern. The risk row "A rotation outruns the backups" says a snapshot holds the version, when the platform only records the operator's attestation (line 200). The Delivery Constraints row does not mention that its Regions clause is met only once Task 015-32 lands.
   Why: The Check and the Risks table are where a reviewer and an operator learn the residual risks. Stating them as absolutes hides an accepted security gap, an unenforced end state and a trust-the-operator gate.
   Correction: Scope the Overview to the three keys and name what stays outside. Make the Goal conditional on step 8 and add an alert on the gauge. Rewrite Principle V to cite Gap 015-003. Mark Principle III Pass, with the intent and outcome records as evidence. Mark Delivery Constraints "Pass with conditions", citing Task 015-32 and its owner gate. Restate the backup risk row as an attestation, with the monthly restore drill as its check.

11. Nit — The refusal of ring key id `transit` is not explained
   Where: lines 190 and 746-747; crates/domain/src/secrets.rs lines 192-211 (the associated data binds the key id).
   What: The doc says a `transit` row has no key id, then refuses a ring key named `transit` without saying why. The reason is that the literal `transit` fills the key-id slot of the associated data, so a ring key with that id would blur the two custodies. A builder may delete the check as a leftover of an earlier revision.
   Correction: Add one sentence giving this reason.

12. Nit — Two sections disagree on whether the chart configures the sandbox runner
   Where: lines 123-124 versus 860-862.
   What: Requirement 015-006 says the header file is "a Kubernetes Secret mounted by the chart for the database runner and the sandbox runner". Technical Plan 7 says the chart has no sandbox-runner template and the operator configures it.
   Correction: Say in Requirement 015-006 that the sandbox runner's header file is the operator's to provide.

13. Nit — "The two phases of ADR-0131" are neither explained nor in that record
   Where: lines 970 and 992.
   What: The two-phase removal of a registry requirement is set out in ADR-0082's amendment of 2026-11-26 (made for ADR-0131), not in ADR-0131's own decisions. The doc does not say what the phases are or how long the operator waits between them.
   Correction: Cite the ADR-0082 amendment and give the phases in one line: draining, then removal once every live process has applied the drain.

14. Nit — The production Collector's pod discovery needs Kubernetes permissions that the doc does not mention
   Where: lines 568-572, 418, 592.
   What: `kubernetes_sd_configs` with role `pod` needs a service account allowed to list and watch pods in `SAGAT_COLLECTOR_NAMESPACE`. The chart does not deploy the Collector, and the runbook items name none of this.
   Correction: List the role and role binding the operator's Collector needs in Technical Plan 3 and in the observability runbook of Task 015-22.

15. Nit — Review history sits in the Overview
   Where: lines 32-36.
   What: A new reader meets review rounds and a decision number before learning what the design does.
   Correction: Move the revision history to a short note at the end of the Overview, or leave it to the research register.

16. Nit — The default `SAGAT_ENV=prod` labels development telemetry as production
   Where: lines 536-537 and 1024.
   What: The compose stack sets nothing, so its spans and logs carry `deployment.environment.name=prod`.
   Correction: Have the compose files set `SAGAT_ENV=dev`, or say why `prod` is the safer default.
