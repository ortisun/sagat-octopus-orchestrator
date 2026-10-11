# Research & Decisions: GitOps, versioning and definition promotion

## Main decision

Digest-immutable registry; active-version alias/pointer as transactional metadata; exportable JSON/YAML manifests.

## Rationale

Maintain Git pipelines, auditing, immutable versions and safe deployment of workflows/scripts across environments. The decision respects the recovery, auditability, security and versioning properties defined in the Constitution.

## Alternatives considered

1. **Generic ad hoc queue-based solution:** rejected as the sole durability mechanism, since message delivery is not equivalent to a transaction over workflow state.
2. **Independent services per feature from day 1:** postponed to avoid operational and consistency complexity without a benchmark.
3. **SDK-specific logic:** rejected for core invariants, to avoid behavior differences between languages.

## Internal references

- `docs/adr/` (architectural decisions)
- `docs/architecture/overview.md`
- `docs/test-strategy.md`
- `specs/001-platform-foundation/contracts/`

## Risks and mitigation

- Complexity/scope risk: deliver P1 user stories separately, running contract tests.
- Semantic ambiguity risk: encode reducers and golden fixtures, do not decide via UI alone.
- Crash/redelivery risk: test harness with real failures at checkpoints.
- Security risk: specific pentest and tenant isolation from the first milestone.

## Questions for spike validation

- Latency and overhead benchmarks on the critical path defined for this feature.
- Detail compliance and retention requirements according to the deployment environment.

## What was decided while building it (D183–D198) and what is left (G193–G208)

Numbering continues after feature 008 (D160–D182, G169–G192). The decision is
[ADR-0018](../../docs/adr/ADR-0018-publishing-is-one-transaction-the-pointer-has-a-history-and-an-approval-is-of-a-document.md);
how to operate it is `docs/runbooks/gitops.md`; the data is in `data-model.md`; the evidence and the reviews are in
`quickstart.md`.

### Decisions

- **D183 — A namespace has an approval policy: how many people other than the author must approve a publication.**
  Production asks for one by default and every other environment for none; an administrator
  (`publication:manage`) sets 0 to 3 with a reason, audited. No row means "the default of the environment", so a
  namespace created later is covered. Tested: `integration approvals.rs` (the default, the set, who may set it).
- **D184 — The rules of a request are in the domain, and the database holds the ones it can.** The author never
  decides; a person decides once (primary key `(request, approver)`); one rejection ends it; only the author
  withdraws; there is one open request per workflow (a partial unique index over `PENDING`/`APPROVED`); editing the
  draft supersedes it. Tested: `crates/domain/tests/versioning.rs` (11), `integration approvals.rs`, `fault
  requests.rs` (a person approving from eight tabs at once approves once; an edit racing an approval never lets the
  request publish).
- **D185 — An approval is of a document, and is counted again when it is used.** A request names the draft revision
  and the digest of the document. Publishing a request publishes the draft at that revision; the approvals are
  counted against the policy **in force at that moment**, in the publishing transaction, so a policy raised after
  the approvals asks for the extra ones (`NOT_ENOUGH_APPROVALS`) and a request published twice makes one version.
  Tested: `approvals.rs::a_policy_raised_after_the_approvals_asks_for_one_more_before_it_publishes`, `fault
  requests.rs::an_approved_request_published_at_once_makes_one_version`.
- **D186 — A direct publish into a namespace that asks is refused by the server** (`409 APPROVAL_REQUIRED`), not by
  the clients. There is no flag to skip it. The same call of a namespace that does not ask works as it always did.
- **D187 — The history of the pointer is a table that is only added to** (`workflow_activations`), written in the
  transaction that moves the pointer, protected by a trigger that refuses update, delete and truncate, and by a
  runtime role that can only select and insert. The last row of a workflow is its pointer. Tested: `integration
  isolation.rs::the_history_of_the_pointer_cannot_be_rewritten`, `integration activation.rs`.
- **D188 — Activation is a compare-and-swap with a gate.** `PUT /workflows/{id}/active-version` with
  `expectedVersion`, a reason, and `acceptBreaking` (default false). When `expectedVersion` is absent the service
  pins the move to the version it compared against, so a publication in between is `ACTIVE_VERSION_CHANGED` and
  not a comparison against a stale version. The breaking-change findings are the ones the Studio diff shows
  (`crates/infra-ir/src/diff.rs`): one definition of "breaks callers" for the editor, the API and the tool.
- **D189 — The triggers follow the pointer and keep their ids.** On every move the triggers of the version are
  registered again with ids derived from the workflow and the trigger, so a trigger is the same row pointed at
  another version, and an event delivered twice across a promotion or a rollback starts one run (the edge case
  "duplicate trigger after promotion"). Tested: `integration activation.rs::the_triggers_in_force…`, `fault
  events.rs` (a duplicate around a rollback; events racing the pointer start one run each).
- **D190 — A promotion publishes the same document into another namespace.** From the canonical bytes of the
  version, to the same environment or a later one, as the workflow of the same name; it never overwrites a draft
  with unpublished changes; the target's policy decides between publishing and a request bound to that document;
  an identical active document is `ALREADY_ACTIVE`; the history of the target says where it came from. Tested:
  `integration promotion.rs` (8), `e2e us03`.
- **D191 — Dependency digests have a foreign key.** `workflow_version_scripts` ties each version to the artifacts
  it pins with `ON DELETE RESTRICT` (SQLSTATE `23001`). FR-011-006 asks to block the removal of artifacts used by
  *active runs*; a rollback can make any published version active again, so the guard is the stricter "pinned by
  any version". A retention policy that lets artifacts go is not built (G193). Tested: `integration artifacts.rs` (3).
- **D192 — The migration rebuilds the history from what existed.** Every publication until now moved the pointer, so
  the history is the list of versions in order; a pointer that was moved by publishing an identical document again
  is closed with one `ACTIVATE` row by `system:migration`. The artifacts pinned by existing versions are filled in
  from their documents. Tested: `integration migration.rs` (back and forth with data in the tables).
- **D193 — Three permissions.** `publication:approve` (developers, operators, administrators), `publication:manage`
  (administrators), `version:activate` (operators, administrators). `definition:publish` is still what opens,
  withdraws and publishes a request. The matrix test in `crates/domain/tests/identity.rs` holds all 35.
- **D194 — `sagat` is a library and a thin executable that speaks only the public REST API.** It validates and
  compares with the server's own codec (`sagat-infra-ir`, the same crate), so a document the tool accepts is a
  document the server parses the same way and the tool has no semantics of its own to drift. Authority stays on the
  server: the tool never decides what an approval or a rollback allows.
- **D195 — The tool is convergent.** Creation and publication carry an idempotency key derived from the workflow, the
  draft revision and the digest, so running `publish` again after a lost answer or a crash is the same operation;
  a draft that already equals the manifest is not rewritten; an approved request is published by running `publish`
  (or, for a promotion, `promote`) again; a namespace with workflows the manifest does not list is left alone.
  Tested: `fault cli.rs` (interrupted half way; two pipelines at once; the same publication with its key twice).
- **D196 — The manifest is closed and has a published schema.** `apiVersion`, `kind`, `namespace`, and workflows with
  `name`, `file` (relative, below the manifest, links followed and checked), optional `releaseNotes`; unknown fields
  are refused. `specs/011-gitops-versioning/contracts/manifest.schema.json` states what a manifest says on its own;
  the contract test holds the schema and the reader to the same corpus (a mutation of the schema fails it).
- **D197 — What the tool does with the machine it runs on.** The token comes from a file or the environment and is
  never an argument (arguments are visible to every user of the machine) and never printed; plain `http` is
  accepted only for loopback; redirects are not followed, so a token goes where it was pointed and nowhere else; a
  document or manifest over the size the platform takes is refused before it is read; every output line that
  comes from the server is its own words (code, reason, remediation, trace id), never the request.
- **D198 — A change of the pointer is stored strictly after the one before it.** The first version stamped each
  change with a time taken before the transaction waited for the workflow row lock, so two operators racing, or a
  replica whose clock was behind, could leave the newest row of the history not being the version the pointer
  held. Found by the fault suite (`fault history.rs` and `fault storm.rs` failed before the fix: failing-first),
  fixed by storing `max(now, latest + 1 µs)` of the workflow inside the locked transaction and answering with the
  row that was written and not "the newest row".

### What is left

- **G193 — There is no retention policy for artifacts.** The database refuses to remove an artifact a version pins
  (D191); nothing lets one go. Until a policy says which versions may be forgotten (and forgets them), every pinned
  artifact stays. FR-011-006 asks for the block "until a safe retention policy is in place": the block is built, the
  policy is not.
- **G194 — The history of the pointer has no retention.** It only grows. A row is small and a workflow moves a few
  times a day; a bound (and how a bound can be reconciled with "append-only") is a decision for when someone has a
  number to bound.
- **G195 — The server does not gate the publication of a breaking change.** Activation and promotion refuse it
  (D188); a direct publication does not, because the draft is the author's to change. `sagat publish` refuses before
  sending unless `--accept-breaking`, which is the tool's check and not the platform's; REST clients do not have it.
- **G196 — A rollback is not subject to the approvals of the namespace.** By design (an emergency needs the version
  that ran last week), and a way around the policy for anyone with `version:activate`. The permission and the
  history are the controls.
- **G197 — Versions published before a namespace asked for approvals are trusted as they are.** Raising a policy
  does not review what is already active.
- **G198 — There are no Studio screens** for the policy, the requests, the history, the promotion or the version
  diff between environments (`VersionDiff.tsx` of the plan is not built). The REST API, `sagat` and the runbook are
  the interface. The Studio already shows the diff of a draft (007).
- **G199 — The manifest lists workflows, not scripts.** Script artifacts are built and signed by feature 005's
  endpoints and pinned by digest in the documents; keeping their sources in Git and building them from a pipeline is
  not done. A promotion carries the pins, not the artifacts: the target needs them (`UNKNOWN_SCRIPT` otherwise).
- **G200 — The policy is one number per namespace.** Not per workflow, by pattern or by environment of the source;
  the maximum is 3.
- **G201 — The tool reads the whole namespace to compare** (the drafts included, up to 5 000 workflows), has no
  machine-readable output and does everything one call at a time. Measured at 50 workflows: milliseconds
  (`tests/performance/reports/gitops-versioning.md`); a namespace of thousands would want a lookup by name.
- **G202 — `publish` is not atomic across workflows.** Each workflow is its own transaction (the platform has no
  multi-workflow transaction); the tool stops at the first failure and says which were applied, and running it again
  completes the rest.
- **G203 — `sagat` is not packaged.** It builds with the workspace (`cargo build -p sagat-cli --release`); there is
  no release workflow for the binary, no installer, no checksums or signature, no shell completion.
- **G204 — The tool authenticates with a bearer token only.** A token exchange for a pipeline's own identity
  (OIDC workload identity) is not built, and the token file's permissions are not checked.
- **G205 — Paths of the plan that were not used.** The task list names `tools/cli/`, `crates/script-registry/` and
  `routes/versions.rs`; the code is in `apps/cli/`, `crates/application/src/publishing.rs` and
  `crates/api-http/src/routes/versioning.rs`, and no new crate was needed for the registry.
- **G206 — What ran where.** The suites below ran against a local PostgreSQL 18.6; CI runs PostgreSQL 16. The suites
  that need Docker (other features') could not run on the machine where this was written and run in CI.
- **G207 — "Another person" is another subject.** Two accounts of one human count as two. Separation of duties
  beyond the subject (teams, domains) is not modelled.
- **G208 — `sagat requests` shows the first page** (50, newest first); older requests need the REST list.
