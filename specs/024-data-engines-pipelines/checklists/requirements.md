# Specification Quality Checklist: Data engines and pipelines

**Purpose**: Validate specification completeness and quality before planning and implementation
**Created**: 2026-10-26; revised 2026-11-26 against `develop` at `e5bda041`
**Feature**: [spec.md](../spec.md)

**Marker semantics**: `[x]` means the author checked the criterion against the text on the date of
the last revision (2026-11-26); it is not a reviewer's approval and says nothing about
implementation. On 2026-11-26 every item that was unticked was disposed under the owner's delegation
(Owner decision 2026-11-26-165): ticked where the corrected set meets it, or left unticked with
"Accepted as it stands, delegate (Owner decision 2026-11-26-165): <reason>" or "Rejected and fixed:
<what>". No security item is accepted this way; a security finding is fixed.

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs) — *not met by design*: following the
  house style of feature 017, the spec names the executors (`apps/database-runner`, the data runner,
  the data mode of `apps/sandbox-runner`), the blob purpose, protocol identities and engine
  mechanisms (read-only transactions, job IDs) because they carry the security and durability
  guarantees. Crate-level placement is kept in the plan. Accepted as it stands, delegate (Owner
  decision 2026-11-26-165): the guarantees of Principles I, III and V are stated in terms of these
  mechanisms; removing them would leave requirements no test can check.
- [x] Focused on user value and business needs (eight user stories with failure scenarios).
- [ ] Written for non-technical stakeholders — *partly*: user stories and success criteria are
  readable by a product owner; the functional requirements target engineers and reviewers, as in
  every feature of this repository. Accepted as it stands, delegate (Owner decision 2026-11-26-165):
  the audience of the requirements is the builders and the reviewers; the stories carry the product
  view.
- [x] All mandatory sections completed (scenarios, requirements, success criteria, assumptions).

## Requirement Completeness

- [x] No `[NEEDS CLARIFICATION]` markers remain; the four product choices (Owner questions 1240 to
  1243) are decided (Owner decisions 2026-11-26-160 to 2026-11-26-163).
- [x] Requirements are testable and unambiguous (each FR is traced to an SC and a task).
- [x] Success criteria are measurable (pass/fail proofs, 100% rounds, closed label sets, recorded
  statement budgets).
- [ ] Success criteria are technology-agnostic — *partly*: SC-024-001 names the engine versions
  under test, SC-024-004 an architecture check and SC-024-017 a budget target; each is what the
  criterion measures. Accepted as it stands, delegate (Owner decision 2026-11-26-165): Principle VIII
  asks for evidence on named engines and checks, and a criterion without them could not fail.
- [x] All acceptance scenarios are defined (Given/When/Then plus a failure scenario per story).
- [x] Edge cases are identified (pins vs pointer moves, overlapping producers, quota during upload,
  tombstone in flight, encrypted parameters, cancellation of uploaded parts).
- [x] Scope is clearly bounded (out-of-scope list; gaps G1240–G1256 and G024-001–G024-005).
- [x] Dependencies and assumptions identified (merged features 001–018, 035, 038, 041 and 043, design
  046, parallel features 018–032, the refinements of ADR-0013, ADR-0023, ADR-0027, ADR-0035 and
  Architecture decisions 0071 and 0079).

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria (SC table maps FR-024-001..042).
- [x] User scenarios cover primary flows (operational engines, warehouses, produce/audit/publish,
  query, lineage/freshness, manual publish/rollback, Polars, external assets).
- [ ] Feature meets measurable outcomes defined in Success Criteria — *not applicable yet*: nothing is
  implemented. Accepted as it stands, delegate (Owner decision 2026-11-26-165): every success
  criterion is future evidence that its task closes; this item is met only by the build.
- [x] Implementation detail that leaks into the spec is limited to what the guarantees depend on
  (see the first item).

## Constitution and security (added 2026-11-26)

- [x] Principle V: untrusted DuckDB and Polars code runs only in per-attempt containers that a
  data-mode sandbox runner starts at the data runner's request (Architecture decisions 0058 and
  0079); the data runner holds no container engine; neither it nor the containers receive a secret.
- [x] Principle V's two declared sets: the five remote drivers use only the second (the managed
  database capability to the lease-owning `database-runner` with `database:execute`), with the
  `database` and `warehouse` profiles kept apart by binding; no amendment is needed.
- [x] The `pii`-egress rule of Owner question 1081 applies to every new-driver connection (ceiling,
  reveal grant, recheck, pause, audit), and the publication event carries no column value.
- [x] Tenant boundary: foreign resources are opaque `NOT_FOUND`; the tenant battery covers every new
  operation and RPC (SC-024-009, Task 024-062).
- [x] Six-SDK conformance (TypeScript, Go, Python, C#, Java, Rust) for the new nodes and the
  `assets` property (SC-024-012).
- [x] No exactly-once claim; every driver names its recovery rule; ambiguous writes wait for a person.
- [x] Finite defaults and hard ceilings for every new resource (plan); zero is not unlimited.
- [x] Failure tests planned: crash at each commit point, duplicates, lost broker hint, races with
  barriers, failed compensation, engine, data runner and data-mode restarts.
- [x] Unproven items marked pending: Architecture decision 0079's acceptance and Kubernetes half
  (Gap 024-001), the layer path for Polars (Gap 024-002), the real-service suite (Task 024-027).

## Gates before implementation (reviewer-owned)

- [x] Owner review of the specification set (T024-001): approved for implementation by the owner
  (Owner decision 2026-11-11-1) and in release 0.3.0 (Owner decision 2026-11-24-1); re-read against
  `develop` on 2026-11-26 under the delegation of Owner decision 2026-11-24-4.
- [x] ADR-0058 accepted; Owner questions 1240 to 1243 answered and recorded (T024-002): Owner
  decisions 2026-11-26-160 to 2026-11-26-164.
- [ ] Dependencies acknowledged by 017 (driver boundary), 005 (runtime profile), 021 (catalog
  generalization) and the ADR-0023 owner (retention of one purpose). Rejected and fixed: an
  acknowledgement by another feature's author is not a gate this set can tick. Features 017, 005 and
  035 are merged, so the dependencies are now stated against their code (research evidence rows of
  2026-11-26); the supersession of ADR-0023 in part is decided (Owner decision 2026-11-26-163); and
  Task 024-002 updates each dependent through its owner and records the status of the drafts (018,
  020, 021, 023, 031) at build start.
- [ ] Worker protocol minor and migration versions allocated by the coordinator. Rejected and fixed:
  no allocation is needed. The register of `sdks/SPEC.md` section 9 gives this feature the next free
  minor when its contract merges (Owner decision 2026-11-15-8; Decision 024-014), and the foundation
  slice takes the migration versions after the latest of `develop` when it writes them (Task
  024-010); field numbers are rechecked against the canonical file at merge.

## Notes

- The ingestion path (step outputs into transforms, D1274) was added during self-review after the
  first draft lacked one; spec, data model, IR and runtime contracts were updated together.
- Facts about BigQuery and Snowflake that the consulted pages do not confirm are G1241, not
  assumptions.
- An independent read-only architecture review (2026-10-26) reported 2 blocking and 9 should-fix
  findings: publication event identity, lock order and key-share upgrade, double data charging,
  `pii` disclosure through the result view, quarantine laundering through `from`, audit/byte
  binding, unenforced read-only claims for Snowflake/BigQuery, metadata pruning versus foreign
  keys, spoofable external observations, gate ordering, and unacknowledged ADR refinements. Each
  was fixed in the spec set (D1265, D1267, D1273, D1275–D1278, G1254, ADR-0058 decisions 4, 5, 9,
  10); the reviewer did not re-review the fixes.
- Round 2 (2026-10-27): the coordinator's architecture review (Blockers 1 to 3, the should-fix
  findings 1 to 7 and the minors) and coordinator decisions C1, C4, C6, C7, C8, C18 and C21 were
  applied: DuckDB became the `dataQuery` data node (D1279), the data container's three mounts are
  declared and tested, ADR-0058 then superseded ADR-0013, ADR-0023 and ADR-0027 in part (O1243 for
  destructive deletion), warehouses run in their own runner deployment, the publication event is a
  protected-system record, Polars followed 023's `python-data` (FR-023-027), lock order cites
  ADR-0050 slot 4, journal events declare 018 reset rules, data-node parameters use 031's codes. Not
  re-reviewed after these edits.
- Round 3 (2026-11-26): an independent audit against `develop` at `e5bda041` (after features 017,
  018, 035, 038, 041 and 043 merged and design 046 was approved). Every finding was fixed in the set
  and in ADR-0058: the data containers are requested through Architecture decision 0079's attempt API
  by a new data runner and role; feature 035's managed kinds, binding profiles, egress ceiling,
  recovery-rule vocabulary, staging, relay and blob allow-list are reused; `ResolvedDatabase` field
  numbers no longer collide with feature 035's; the protocol minor is the next free one; six SDKs;
  instance-registry capabilities; design 043's gate, design 038's budget and design 041's step-log
  rule are applied; Polars uses design 046's layer; Owner questions 1240 to 1243 are decided and
  ADR-0058 is accepted (Owner decisions 2026-11-26-160 to 2026-11-26-164); the checklist items above
  are disposed (Owner decision 2026-11-26-165). The research register records the new entries as
  Decisions 024-001 to 024-015 and Gaps 024-001 to 024-005.
