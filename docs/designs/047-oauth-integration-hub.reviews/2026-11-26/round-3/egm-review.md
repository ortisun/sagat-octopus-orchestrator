# egm-review for the approval of design 047 (2026-11-26)

Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: 6a57e37db990c842ad1066f8bba1cf62f3e33d28
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary. Round 1 (2026-11-24, at `f6fef1d6`) ran all five stages of `egm-review`; round 2 (at
`e1d438b0`) ran the zoom-out, comprehension and critic stages. On the approval rule of Owner decision 2026-11-25-2 the four
stages that did not run in round 2 ran on 2026-11-26 at `4bc5767b`: structure, writing, implementation readiness and
codebase details (`stage-structural.md`, `stage-style.md`, `stage-readiness.md`, `stage-codebase.md` beside this file).
Their Blockers and most of their Majors were fixed in the text before the reviewed commit (Decisions 047-065 to 047-067),
and a codebase-details re-read at the reviewed commit checked the fixes. What stays open at the reviewed commit is listed
below and counted; each is settled in writing by a delegate row and, where it needs text, a follow-up task done before
the task it names.

## Fixed in the text before the reviewed commit

Codebase details: both Blockers (the redirect URI from a new setting `SAGAT_STUDIO_ORIGIN`; the `resource` host call in all
five sandbox languages with the shims where they are) and Majors 3 to 7 (migration timestamps, feature 017's OpenBao
files, feature 028's surfaces extended, feature 021's checks reused, `workflow.schema.json` and a fragment contract), and
the Nits. Structure: Major 2 (the delivery tasks carry `Owner-gate: OD-2026-11-25-3`; the condition stated where approval
is), Major 5 (the decision range and the probe's meaning), Nits 6, 9. Writing: Majors 1, 2, 4, 5 and 6 (the Principle V
row, both client errors, a sliding window, the withdrawn-offer test placed, Terms for token revisions and broker passes).
Readiness: Majors 1, 2, 5, 6 and 7 (feature 021 and 028 tasks first, the operator surface refused until Architecture
decision 0074 in `rest-api.md` too, the data model's columns, window tables and reserved words, tests placed in their
phases, feature 028's tables moved to Task 047-13), Nits 9 and 10.

## The codebase-details re-read at the reviewed commit

Every earlier finding answered, except: the capability enum of `crates/application/src/instances.rs` not named (a Nit);
`OpenBao::retire_inner` (`openbao.rs:297`), which already writes the `cas=0` tombstone, not cited (a Nit). New: two Majors
(findings 7 and 8 below) and three Nits (the `apps/dev-idp` routes are not an RFC 6749 token endpoint; feature 028's task
range ends at 028-066 in two places and 028-067 in two; the bootstrap scripts need editing and Helm already has
`controlPlane.studio`, so the value should be `controlPlane.studioOrigin`).

## Open at the reviewed commit

1. Major (structure 1). Requirement identifiers are prose ("Requirement 047-001") where the design template asks for
   `FR-047-001`, and approved design 043 uses that form.
2. Major (structure 3). Owner decisions 2026-11-25-2, 2026-11-25-3 and 2026-11-24-320 were not in
   `docs/owner-decisions.md` when the stage ran.
3. Major (structure 4). No task refs Success criterion 047-002, and Success criterion 047-001 only by the benchmark.
4. Major (writing 3). Requirement 047-013 carries several rules in one paragraph.
5. Major (readiness 3). Files without one owner: Task 047-14 is builder B's but edits builder A's crates
   (`publishing.rs`, `runs/`); Requirement 047-023's block check at admission and delivery has no task.
6. Major (readiness 4). `rest-api.md` gives no request or response schemas beyond authorizations, callback and import,
   no tenant route listing offers and no offer identifier.
7. Major (codebase re-read 1). Task 047-2 puts "the five new permissions" in `Permission::ALL`, but there are six, and
   two are operator-only (`oauth:offer:manage`, `hub:version:block`); `ALL` grants the admin role
   (`crates/domain/src/identity.rs:400`), so adding them would give operator powers to Admin.
8. Major (codebase re-read 2). The protocol-minor gating has a second store (`crates/infra-postgres/src/execution/path.rs`),
   a second gate (`crates/application/src/worker/scoped_claims.rs`) and the trait (`crates/application/src/execution.rs`),
   and it is run-wide today, not per task.

Counts: Blocker 0, Major 8, Nit 3

## The author's answers

1. Rejected, Owner decision 2026-11-24-321 (delegate): constitution 4.7.0 (Owner decisions 2026-11-20-8 to 2026-11-20-10)
   requires identifiers in words in prose, and `scripts/check-plain-words.py` enforces it; the template's form predates it.
   Design 046, approved under the same rules, uses the same form.
2. Settled, Owner decision 2026-11-24-322: Owner decisions 2026-11-25-2 and 2026-11-25-3 are now in
   `docs/owner-decisions.md` (merged from `develop`), and 2026-11-24-320 is registered with the approval.
3. to 6. Settled, Owner decisions 2026-11-24-323 to 2026-11-24-326, by follow-up Task 047-28 before Task 047-1 (the
   success-criterion references and an end-to-end task for Success criterion 047-001; Requirement 047-013 split into
   numbered rules; each file given one task and one builder, with a task for Requirement 047-023's block check; the
   request and response schemas, the offers listing route and the offer identifier written into `rest-api.md`).
7. Settled, Owner decision 2026-11-24-327, by follow-up Task 047-27 before Task 047-2: only the four tenant permissions
   go in `Permission::ALL`; the two operator permissions stay out of it, held by the platform operator only, as
   `rest-api.md` already says.
8. Settled, Owner decision 2026-11-24-328, by follow-up Task 047-27 before Task 047-15: every gating file listed and a
   per-task protocol requirement added beside the run-wide one. The three Nits go into the same task.
