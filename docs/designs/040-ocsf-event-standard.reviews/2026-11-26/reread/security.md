# Security re-read of design 040 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: d6d00600598c4d2a528625ec8d577e647fb09d67
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 1, Nit 4

Round 1 security: 1 fixed, 2 fixed, 3 fixed, 4 fixed, 5 fixed, 6 fixed (the data model; its rolling-deploy half comes back as finding 1), 7 fixed, 8 fixed, 9 fixed, 10 fixed, 11 (the second item numbered "10", the restore) fixed, 12 fixed, 13 fixed, 14 fixed, 15 fixed, 16 fixed, 17 fixed.

Checked in code, not only in the text:
- Item 3: `step_log_usage_if_revealed` checks the role and every namespace's grant in one snapshot (`crates/infra-postgres/src/reveal_grants.rs:84-115`). Reusing it with "no fence" keeps the window to one batch.
- Item 10: the `payload_reveal_grant` resource id is `<namespace>/<subject>` (`crates/application/src/reveal_grants.rs:296`).
- `SecretCipher::seal` and `open` take associated data (`crates/domain/src/secrets.rs:169,175`).
- D040-008 in `research.md` is amended, so the "verifies the other" claim is gone.

1. Major — The operator stream can skip records: its start and new-tenant rules are undefined
   - Where: `docs/designs/040-ocsf-event-standard.md:496-502`, `:424-426`, `:383`, Task 040-24 (`:731`).
   - What: a configured `start: now` "applies when the configured destination changes", but no destination column is persisted. The only thing to compare against is each replica's memory. So every restart, and every switch between old and new configuration during a rolling deploy, looks like a change, and the backlog is dropped. For example: the collector is down for an hour, the engine restarts, and that hour's cross-tenant probes never reach the operator.
   - Also: each tenant's row is "created when a tenant's first record is written", which either adds a statement to the writer (breaking Non-functional requirement 040-001) or leaves the row's start cursor undefined. Started at `xmin`, the first records of a newly provisioned attacker tenant, its probes included, are lost. Section 4 (one global query) and section 5 (a cursor per tenant) also disagree.
   - Correction: persist a fingerprint (a hash, not the URL) and a configuration revision in a one-row table, and apply `start` once per change of revision. Discover tenants from a global discovery cursor and set each new row just below the first record found. Make section 4 match section 5.

2. Nit — `pg_read_all_stats` granted to `sagat_app`
   - Where: `:583-585`, `:593`.
   - What: this lets the runtime role read the query text and activity of every session on the cluster, other databases and operator sessions included, just to compute one age.
   - Correction: use a `SECURITY DEFINER` function owned by a monitoring role, with `search_path` pinned, returning only the maximum age, and grant `EXECUTE` on it to `sagat_app`. Precedent: Gap 041-016.

3. Nit — `/rotate-signing-key` is outside Requirement 040-013
   - Where: `:146-152` and `:558`.
   - What: an admin who holds `audit:sink:manage` but no reveal grant can rotate the key of someone else's `pii` sink. They get a key the tenant's SIEM will accept, and the rotation neither checks their authority nor makes them the authorizing subject.
   - Correction: add rotation to Requirement 040-013's list of acts that re-establish the subject and are refused with `PII_CEILING_NOT_PERMITTED`.

4. Nit — `/test` outcomes can reveal internal DNS
   - Where: `:559-561`.
   - What: the closed set has no transport or resolution class. If a resolver refusal of a private address answers `denied` while NXDOMAIN or a connection error answers differently, an admin who can set egress policy can map internal hostnames.
   - Correction: state that every egress refusal, resolution failure and transport error returns the same outcome. Same class as G035-092.

5. Nit — Runtime role and `pg_control_system()` (suspicion, not verified)
   - Where: `:403`.
   - What: if `sagat_app` cannot run this function, every pass fails. That fails closed, so it is availability, not disclosure.
   - Correction: run `SAGAT_TEST_AS_RUNTIME_ROLE=1` against a statement such as `SELECT system_identifier FROM pg_control_system()` in Task 040-15. If it is refused, take the identifier at migration time into a constant row.

Files read: `docs/designs/040-ocsf-event-standard.md`, `docs/designs/040-ocsf-event-standard.reviews/2026-11-26/round-1/security.md`, `docs/adr/ADR-0150-security-and-audit-events-leave-as-ocsf-projections-of-the-native-record.md`, `crates/infra-postgres/src/reveal_grants.rs`, `crates/application/src/reveal_grants.rs`, `crates/domain/src/secrets.rs`.
