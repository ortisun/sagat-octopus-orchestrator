# Research and registers — feature 035 Apache Iceberg write steps

Append-only registers (decisions `D035-NNN`, gaps `G035-NNN`, owner questions `O035-NNN`). Numbers are
feature-scoped, as in feature 033, so they cannot clash with the global ranges of other branches.
`/speckit-plan` adds the technical research below these registers; it does not renumber them.

## Owner decisions (2026-10-27)

Asked before the specification was written; recorded in `docs/owner-decisions.md`.

- **D035-001 — Write modes.** The owner chose append, overwrite and upsert (merge by key) for the
  first delivery, against the recommendation of append + overwrite with upsert later. Consequence:
  row-level deletes, key validation and concurrent-upsert isolation are in scope (FR-035-016,
  FR-035-017, US05).
- **D035-002 — Catalogs and storage.** The owner chose the broad set: REST, AWS Glue, Hive Metastore
  and JDBC catalogs; S3-compatible storage, Google Cloud Storage and ADLS Gen2 — against the
  recommendation of REST + S3-compatible first. Consequence: one fixture per type, emulator fidelity
  gaps (G035-003) and a larger network-policy surface (FR-035-004).
- **D035-003 — Destination model.** The Iceberg destination is a new connection kind of the feature-017
  catalog (recommended option). It reuses revisions, OpenBao secrets, probe, managed executor, network
  policy and ad hoc admission, and takes no dependency on the PR #53 drafts (020, 021, 024).

## Specification defaults (agent, reviewable at `/speckit-clarify`)

- **D035-004 — Tables are the customer's.** The step writes into existing tables and never changes a
  schema, partition spec, sort order, property or ref (FR-035-011, FR-035-012), mirroring 017's "no
  provisioning".
- **D035-005 — Effect key in the snapshot summary, lookup before writing.** An Iceberg commit is an
  atomic metadata swap, and the snapshot summary accepts writer properties; the stable step idempotency
  key goes there, and the executor searches the target ref's retained history for it before
  committing (FR-035-013, FR-035-018). Precedent to verify during planning: Flink's Iceberg sink
  records its last committed checkpoint id in the snapshot summary to avoid duplicate commits on
  recovery. The lookup is conclusive only within retained history; otherwise the outcome is unknown
  (FR-035-019). This is a reconciliation lookup in the sense of constitution III, not an exactly-once
  claim.
- **D035-006 — Revert is a new inverse commit, never a rollback of the table pointer.** Rolling the
  table back to an older snapshot would discard concurrent writers' commits (FR-035-021).
- **D035-007 — Destructive modes are opt-in per connection revision.** Default allowed modes are
  `append` only; overwrite and upsert need an administrator's explicit revision (FR-035-005).
- **D035-008 — Classification ceiling.** Default ceiling `internal`; `pii` only by a holder of the 016
  reveal grant; `secret` never; undeclared fields count as `pii` (016 restrictive reading, owner
  decision 2026-10-17 item 3) (FR-035-005, FR-035-025).
- **D035-009 — File locations are data.** Iceberg data file paths can embed partition values, so file
  locations never appear in outputs, logs, spans, audit or errors (FR-035-027, FR-035-030).
- **D035-010 — Parquet only in this delivery.** Tables whose default data file format is ORC or Avro
  are refused (FR-035-011, G035-002).
- **D035-011 — Catalog responses are untrusted.** Table locations, configuration overrides and vended
  endpoints returned by a catalog are confined to the connection's warehouse locations and pass the
  network policy (FR-035-004), so a hostile or misconfigured catalog cannot redirect writes.
- **D035-012 — Lookup and commit are tied by a conditional commit.** Every Iceberg catalog commits by
  compare-and-swap of the table's metadata pointer. Each commit request is conditional on the exact
  state the attempt last searched for its effect key, and a moved table means a new search before any
  retry (FR-035-018). This closes the race between a stale attempt and its successor without relying
  on lease timing. The plan must show that the client's own internal commit retries do not bypass the
  search (a client that silently re-applies an append to a newer state would reopen the race).

## Owner answers at `/speckit-clarify` (2026-10-27)

Recorded in the spec's Clarifications section and in `docs/owner-decisions.md`.

- **D035-013 — Batch size.** At most 2 GiB per step, in at most 32 record blobs of up to 64 MiB each
  (ADR-0023 blob ceiling), streamed into files (FR-035-009, FR-035-028, SC-035-009). Recommended
  option. This resolves the earlier inconsistency between a single-blob source (64 MiB) and the
  100 MiB benchmark of SC-035-009. The ceiling equals feature 024's snapshot ceiling.
- **D035-014 — Deletes in upsert.** An upsert step may declare a delete marker, a boolean record field
  that is not a table column; marked keys are removed in the same commit (FR-035-016, US05 scenario 6).
  Recommended option.
- **D035-015 — Unknown record fields.** Refused by default; a step may declare that they are ignored
  (dropped before any check, counted, never written). No schema change ever (FR-035-010,
  FR-035-027). Recommended option.
- **D035-016 — Snapshot summary identifiers.** Effect key, run id, step id, workflow name and published
  version; no values, actor or tenant identity, or credentials (FR-035-013). The owner chose the
  richer lineage option over the recommendation (key + run and step ids), accepting that workflow
  names become visible to readers of the table's metadata. Answers O035-001.
- **D035-017 — No table creation.** Confirms D035-004: a missing table fails with a typed error; the
  step never creates tables (FR-035-011). Recommended option.

## Scope extension: step-output destinations (owner, 2026-10-27)

Owner's words: "Quero que o output dos steps também possam ser salvas no iceberg, postgres, etc...
Exemplo. Acabei finalizar um pagamento, esse pagamento poderia descer para o lake". Asked as four
questions after the clarification session; recorded in `docs/owner-decisions.md`.

- **D035-018 — Both modes, chosen per destination.** A step's output may go to synchronous
  destinations (inside the run; dependents wait) and asynchronous ones (batched export after commit).
  The recommendation was asynchronous only, for the payment-to-lake example.
- **D035-019 — Five destination kinds, all in both modes.** Iceberg, customer PostgreSQL, object
  storage files, ClickHouse and broker streams. The recommendation for the synchronous mode was
  Iceberg and PostgreSQL only; the owner chose all five (FR-035-038..042, US08).
- **D035-020 — Asynchronous export is 020's.** Draft 020 already specifies declared records
  (FR-020-019) exported by sinks (`OBJECT_STORAGE`, `CLICKHOUSE`, `POSTGRESQL`, `STREAM`, `PULL`) from
  the journal with per-sink cursors; its US04 is the payment example. 020 gains an `ICEBERG` sink that
  reuses this feature's connection kind and lookup (recommended option). 020's pending constitution
  amendment concerns retention, not the export.
- **D035-021 — Synchronous destinations do not depend on 020.** They reuse this feature's record
  sources and mapping (FR-035-009, FR-035-010) over the step's output, not 020's declared-record
  contract, so they can be built before 020; the Studio shows both modes together (FR-035-044).
  Agent decision; reviewable.
- **D035-022 — Destination writes are derived steps scheduled by the accepted completion.** The
  completion transaction schedules them (constitution I); dependents wait for them (FR-035-035).
  Precedent: derived compensation steps (ADR-0010).
- **D035-023 — Default failure policy `hold`.** A destination outage must not compensate a source
  step's committed effect (a charged payment); `fail` and `continue` are explicit choices
  (FR-035-037). Agent decision; owner question O035-004.
- **D035-024 — Recovery per kind** (FR-035-039..042): PostgreSQL by a constraint-backed key in one
  transaction; object storage by an effect-key object name, create-only or checked, compared by
  checksum; ClickHouse by the effect key as the insert deduplication token, bounded by the server's
  guarantee; broker streams by delivery rows committed with the completion (ADR-0025) and a stable
  message id, at-least-once.
- **D035-025 — Two new 017 connection kinds**, `object_storage` and `clickhouse`; broker streams reuse
  ADR-0025/033 outbound destinations; PostgreSQL reuses the 017 kind.

## Gaps

- **G035-001** — Kerberos-secured Hive Metastores are not supported in this delivery.
- **G035-002** — ORC and Avro data files are not written; such tables are refused.
- **G035-003** — Emulator fidelity: Glue, GCS and ADLS Gen2 (and possibly the REST catalog flavours
  of cloud vendors) will be tested against local emulators or reference implementations; real-service
  verification needs cloud accounts the repository does not hold. Each such type is labelled
  "verified against <emulator>" until real-service evidence exists (FR-035-032, SC-035-010).
- **G035-004** — No atomic commit across several tables.
- **G035-005** — A revert cannot proceed after the table owner compacts, rewrites or expires the files
  involved; it waits for a person (FR-035-021). Expiry also shortens the lookup window (FR-035-018).
- **G035-006** — Table maintenance stays with the customer; frequent small commits produce small
  files, documented in the runbook but not mitigated by the platform.
- **G035-007** — The table's sort order is not enforced on write.
- **G035-008** — ClickHouse deduplication holds only for table engines that deduplicate inserts and
  within the server's deduplication window; outside it an uncertain insert goes to review
  (FR-035-041).
- **G035-009** — Broker streams are at-least-once: a consumer may see a message twice and must
  deduplicate by message id (FR-035-042, ADR-0025).
- **G035-010** — No atomicity across destinations: one destination can hold the data while another
  failed (edge cases); only Iceberg destinations can be reverted.
- **G035-011** — With synchronous destinations every run makes its own commit or insert; at high
  volume Iceberg gets many small commits and files. The asynchronous 020 sink batches; the runbook
  says which mode suits which volume.

## Owner questions

- **O035-001** — Should the snapshot summary carry the run and step identifiers beside the effect key
  (useful lineage, but platform identifiers become visible to everyone who can read the customer's
  table metadata) or the effect key only? Recommendation: effect key plus run and step ids, no actor
  or tenant identity (as written in FR-035-013); confirm at `/speckit-clarify`. **Answered
  2026-10-27 (D035-016):** the owner added the workflow name and version.
- **O035-002** — Priority of ORC/Avro (G035-002) and Kerberos Hive (G035-001) for a later delivery.
- **O035-003** — Default and maximum limits per step (records, bytes, files) and the reference
  environment of SC-035-009; the plan proposes values, the owner approves them.
- **O035-004** — Confirm `hold` as the default failure policy of synchronous destinations (D035-023).
- **O035-005** — Should 020's `OBJECT_STORAGE` and `CLICKHOUSE` sinks reference the new 017 connection
  kinds (`object_storage`, `clickhouse`) instead of carrying their own settings and secret, so a
  destination is configured once for both modes? Recommendation: yes (020's `POSTGRESQL` sink already
  uses a 017 connection, and its `ICEBERG` sink will).

## Sources to consult during planning

Not yet consulted for this specification; the plan records what was read and when: the Apache
Iceberg table specification (format versions, snapshots, summary, refs, row-level deletes, partition
transforms), the REST catalog OpenAPI (configuration, credential vending, commit requirements), the
AWS Glue, Hive Metastore and JDBC catalog commit protocols, and the Flink Iceberg sink's
duplicate-commit protection.
