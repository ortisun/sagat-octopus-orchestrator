# Security review of design 042 (first draft)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 540154c80cb36cc585d18c5952f7c0abce77d430
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output was relayed by the coordinator in condensed form, grouped by subject across the three reviewers; this record lists the subjects attributed to this reviewer, in the relay's words, with the answer of the second draft. The relay attributes 2 Blocker and 8 Major subjects to this reviewer against counts of 2 and 7 (one subject may be shared or merged in the relay) and itemises 3 of the 6 Nits; the rest were not itemised. Counts are the reviewer's own.

## Blocker

1. The collection chain's ceiling bounds neither memory, bytes nor time (a denial of service of the scheduler from a published definition).
   *Answer:* one environment per chain, a blocking task with a 1 s deadline, running ceilings on elements and bytes per stage (FR-042-006, FR-042-007, section 3, ADR-0087 decisions 2-3, T042-8).
2. Held trigger values were to go to `payloads`, which is append-only (`payloads_append_only` refuses DELETE), so personal data could never be deleted.
   *Answer:* values live in a deletable, classified, quota-counted column of the chain's own table, deleted in the start or discard transaction (FR-042-024, section 4, ADR-0086 decision 2, D042-029, T042-13, T042-22). Later release.

## Major

3. Blob sources bypass the chain's bounds and break the membership checks.
   *Answer:* `materialize` bounded by the chain input ceiling, a chained manifest, output ceilings before the store (FR-042-010, D042-020, T042-45, T042-46).
4. `onError` promised at entry is not what the scheduler does.
   *Answer:* the run fails with the typed code; gap G042-008 (FR-042-008).
5. ADR and approval wording.
   *Answer:* ADR status lines, the design and the roadmap agree with OD-2026-11-13-17; D042-016.
6. Keys and `distinct` values as plain digests, shown by the API, are reversible for short keys.
   *Answer:* HMAC-SHA-256 under a per-tenant key that stays in the database; random surrogate ids in the API; key text only with the reveal right (FR-042-024, ADR-0086 decision 2, D042-030, T042-53, T042-39). Refines O042-008, put to the owner with T042-44. Later release.
7. `maxKeys` overflow as `QUOTA_EXCEEDED` waits through the dispatcher's retries and holds the event; uncollapsed audit; marks holding key slots.
   *Answer:* a terminal per (event, trigger) outcome `FAILED`/`KEY_LIMIT`, audited once per chain and minute with a count, retryable by an operator; marks do not hold a key slot (FR-042-021, ADR-0086 decision 9, D042-031, T042-18). Refines O042-009, put to the owner with T042-44. Later release.
8. No ceiling on marks and no per-tenant ceiling on held bytes.
   *Answer:* `maxMarks` per chain and a held-bytes quota (FR-042-021, T042-13, T042-16); the numbers go to the owner with T042-44. Later release.
9. Flush and retry under `event:manage` (configuration; the operator role lacks it); the starter of a flushed run not recorded.
   *Answer:* flush and retry need `run:start` and `run:operate`, discard `run:operate`; the caller is recorded as the starter (FR-042-023, ADR-0086 decision 10, D042-032, T042-34). Later release.
10. The backlog rule is unbounded and unscoped: no hard grace after `max_close_at`, the query not scoped by tenant and namespace, and claiming does not increase `attempts`.
    *Answer:* the rule counts only `attempts = 0` events of the same tenant and namespace and holds at most `maxHoldMs` after the deadline (FR-042-017, ADR-0086 decision 4, D042-033, T042-21). Later release.

## Nit

11. `distinct` canonical form.
    *Answer:* FR-042-011, D042-025.
12. (N5) `event_deliveries` rows should never be updated and are deleted with the inbox row.
    *Answer:* stated (FR-042-014, section 4).
13. (N6) The purge should lock the chain row first.
    *Answer:* stated (FR-042-022, section 6, D042-037).

Counts: Blocker 2, Major 7, Nit 6
