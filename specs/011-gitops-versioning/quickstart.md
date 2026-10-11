# Validation quickstart: GitOps, versioning and definition promotion

Everything below was run against this repository. The pieces that are **not** built, and what could not be run where it was written, are listed at the end.

## Prerequisites

- The Rust toolchain pinned by `rust-toolchain.toml` and a PostgreSQL the suites can create databases in: Docker (Testcontainers starts PostgreSQL 16; no port is published on the host) or, with `SAGAT_TEST_DATABASE_URL=postgres://…@host:port/postgres`, a server you name (this is how the figures below were produced: PostgreSQL 18.6 from Homebrew).
- Nothing else: the suites of this feature start the API, the engine and the command line in the test process (the engine as a process only for US02, where a run has to be woken).
- To use it by hand: `cargo build -p sagat-cli --release`, then `docs/runbooks/gitops.md`.

## Validation flow

1. **The user stories**, against real PostgreSQL, a fake OIDC provider and real sockets, through the command line (the library the executable wraps) and REST:

   ```sh
   cargo test -p sagat-tests --test e2e_gitops_versioning            # US01-US04, the command line's own paths, the metrics (15)
   cargo test -p sagat-tests --test fault_gitops_versioning          # races, killed connections, duplicates, interruptions (15)
   cargo test -p sagat-tests --test integration_gitops_versioning    # approvals, the pointer and its history, promotion, the migration back and forth, artifacts, tenant isolation (35)
   cargo test -p sagat-tests --test contract_rest_gitops_versioning --test contract_manifest_gitops_versioning --test contract_rest_security_multitenancy   # the 12 new operations against the OpenAPI; the manifest schema and its reader; the cross-tenant battery
   cargo test -p sagat-cli -p sagat-domain -p sagat-application      # the parser and the manifest reader; the request state machine (11); the permission matrix
   ```

2. **The Studio's types** are generated from the OpenAPI: `npm run api` and `npm run check:api` in `frontend/` (the contract changed: the Versions tag, 12 operations, `scripts` on a version).

3. **The benchmark** (what publishing, moving the pointer and the command line cost):

   ```sh
   SAGAT_PERF_REPORT=tests/performance/reports/gitops-versioning.md \
     cargo test -p sagat-tests --test perf_gitops_versioning --release -- --ignored --nocapture
   ```

4. **Using it:** `docs/runbooks/gitops.md`.

## Story evidence

| Story | Acceptance scenario | Evidence |
|---|---|---|
| US01 Publish immutable version | A validated draft is published as v3: the IR/deps digest is fixed and new runs can use v3 | `e2e us01::us01_a_manifest_becomes_immutable_versions_and_a_rerun_changes_nothing` (three versions from a manifest in Git; the digest of each is the digest of its document; version 1 is exactly as it was; a run started afterwards is on version 3; running the command again changes nothing), `…what_cannot_be_published_is_refused_before_anything_is_sent` (a broken document stops the whole publication before anything is sent; a manifest that points outside its directory; somebody who may not publish is told by the platform); `integration activation.rs::every_publication_is_a_row_of_the_history_and_a_rollback_adds_one` |
| US02 Keep old run | A run on v2 waits for an event; v3 is published; the run resumes with v2 semantics/artifacts | `e2e us02::us02_a_run_waiting_on_version_one_resumes_with_version_one_after_two_is_published` (the engine as a process, a worker serving both activities: the old run asks for version 1's activity, the run started after the publication for version 2's; both finish); `integration activation.rs::a_new_run_uses_the_active_version_and_one_in_flight_keeps_its_own` |
| US03 Promote across environments | A manifest in Git is promoted dev → stage → prod: history, approvals and dependencies stay traceable | `e2e us03::us03_a_version_goes_from_dev_to_staging_to_production_with_its_history_and_its_approval` (the same digest in three namespaces; staging at once; production becomes a request, its author cannot approve it, somebody else does, the same command published it; the history says `PROMOTE`, the request and the reason; the dependencies of the version travel with it; going back is refused); `e2e cli::publishing_a_manifest_into_a_namespace_that_asks_for_approvals_waits_and_then_converges`; `integration promotion.rs` (7: a secret the target lacks, a draft with changes, a change that breaks callers, the same document twice, not backwards, the permission) |
| US04 Perform safe rollback | v4 shows a regression; the operator switches the pointer to v3: new runs use v3 without rewriting v4's history | `e2e us04::us04_rolling_back_moves_the_pointer_and_rewrites_nothing` (a developer cannot, an operator can and says why; the pointer moves; every version is byte for byte what it was; the run in flight keeps its version; the history has the rollback on top of the publications; `diff` warns that publishing the manifest would activate the old document again); `integration activation.rs` (6: the history, the permission and the reason, the compare-and-swap, the breaking-change gate, the triggers follow the pointer, runs in flight) |

## Edge cases of the specification

| Edge case | Where it is handled |
|---|---|
| Draft edit conflict | The draft's revision is checked on every edit (feature 002); editing the draft supersedes the open request so approvers' decisions never apply to a document that is gone (`integration approvals.rs::editing_the_draft_supersedes_the_request_and_nothing_is_published_from_it`, `fault requests.rs::an_edit_that_races_an_approval_never_lets_the_request_publish_what_nobody_saw`); the tool replaces a draft only with the revision it read |
| Rollback during a Run | Runs are pinned to the version they started on; the pointer decides only where a new run starts (`us04`, `activation.rs`) |
| Artifact changed under the same name | A script artifact is identified by digest and a document pins the digest: a changed artifact is another digest, so another document, so another version (`integration artifacts.rs::a_version_records_the_artifacts_it_pins_and_an_artifact_in_use_cannot_be_removed`) |
| Offline dependency disappeared | The database refuses to remove an artifact a version pins; publishing and promoting refuse a digest the tenant does not have (`UNKNOWN_SCRIPT`); a rollback to a version that pins one finds it there (`artifacts.rs`: 3) |
| Duplicate trigger after promotion | The triggers of the active version are registered again with the ids they had, and an event is delivered once whatever moved the pointer meanwhile (`fault events.rs`: 2; `integration activation.rs::the_triggers_in_force_are_those_of_the_active_version_and_follow_a_rollback`) |
| Two operators, two clocks | A change of the pointer is stored strictly after the one before it, inside the locked transaction; the first version of this stamped it before waiting for the lock, and `fault history.rs` and `fault storm.rs` failed on it (D198) |
| A policy raised after the approvals | The approvals are counted again against the policy in force when the request is published (`approvals.rs::a_policy_raised_after_the_approvals_asks_for_one_more_before_it_publishes`) |
| A connection killed in the middle of a publication | Nothing half done is visible: under three seconds of every connection being terminated, eight versions and a pointer moved back and forth leave one chain, one row per version and a pointer where the last change left it (`fault storm.rs`) |
| A command line interrupted, or two at once | Running it again completes the rest; two pipelines publishing one manifest leave one workflow and one version of each (`fault cli.rs`: 4) |
| Cross-tenant | Every new operation is in the cross-tenant battery of feature 009 (`contract_rest_security_multitenancy`), the database refuses a row that points into another tenant (`integration isolation.rs`), and from the command line another tenant's namespace is indistinguishable from one that does not exist (`fault cli.rs`) |
| Secrets and payloads | Nothing of a run, a payload or a secret is in a request, a decision, a history row, an audit entry or a metric label (`e2e telemetry.rs` names no one); the command line never prints its token |

## Gates run

The complete local run, including all failures and ignored tests, is recorded in
[`validation-2026-10-11.md`](validation-2026-10-11.md). Workspace totals: **1,626 passed, 63 failed,
26 ignored**; all six failing targets require containers that Docker could not create.

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `scripts/check-architecture.py` | passes (20 workspace crates, layers respected, no cycles) |
| `cargo test --workspace --locked --no-fail-fast` | 1626 passed, 63 failed: every failure is in a suite that starts containers other than PostgreSQL (`e2e_durable_execution` NATS, `e2e_events_webhooks_messaging` Kafka/SQS/NATS, `*_inline_code_sandbox*` images, `integration_durable_execution` NATS), which Docker could not create here (see the end); none is in a suite of this feature, and the 008 gauge test that read a process-wide recorder was made robust |
| `npm run check:api` | the generated types match the contract |
| `npm run typecheck` in `frontend/` | passes |
| `cargo audit --deny warnings` | passes |
| OpenAPI / Protobuf / AsyncAPI / IR example | lint/compile/validation pass; OpenAPI has 9 existing warnings, AsyncAPI has one informational version notice |

## Reviews of the implementation (T011-034)

Reviewed the domain rules, the application services, PostgreSQL transactions and constraints, REST handlers,
generated contract types, CLI boundaries, README and the GitOps runbook. Evidence was checked on 2026-10-11;
this is a code and test review, not an independent penetration test or production approval.

| Concern | Evidence and outcome |
|---|---|
| Separation of duties | `approvals::whoever_saved_the_draft_cannot_approve_a_request_somebody_else_opened` and the domain test refuse both the requester and the recorded editor, including when a pipeline opens the request. Refusals are audited with deduplication. |
| Tenant and resource identity | The REST security battery covers all implemented operations; `integration isolation.rs` rejects foreign tenants, mismatched workflows/namespaces/versions, and mutation of the activation history even in replication mode. Request identity is frozen by the database. |
| Approval races | The request is opened against a locked draft revision, is superseded and audited by a later edit, and the publication transaction checks revision, digest, state and required approvals. Covered by `integration approvals.rs` and `fault requests.rs`. |
| Lost publication response | Extended `requests::a_retry_storm_with_one_key_gets_one_answer_and_later_the_same_one` to save an invalid new draft before retrying. It failed with `SUPERSEDED` before the recovery fix and passed afterwards. Recovery now reads the committed version using the immutable request fingerprint; another key is refused. REST conformance also checks the replay, viewer refusal and cross-tenant refusal. |
| Redaction and terminal output | Release notes reject terminal controls and format characters (newlines and tabs are allowed); operator reasons are bounded plain text; CLI tests cover token handling and manifest boundaries. User-authored reasons and notes are persisted: operators must not put secrets in them. |
| Observability | Request spans retain trace IDs; errors have typed reasons and remediation, including `REQUEST_NOT_APPROVED`. Metrics use closed label sets; `e2e telemetry.rs` checks that tenant, workflow and request names do not appear. Superseded requests and prohibited decisions are on the audit trail. |
| Compatibility | OpenAPI and generated Studio types include the editor identity and committed-request recovery. Migration rollback/forward tests and the US02/US04 run-pinning tests pass. README and the runbook identify the remaining product gaps. |

The focused validation command below passed **74 tests, zero failures or ignored tests** against an isolated
PostgreSQL 18.6 server: 3 manifest contract, 1 GitOps REST contract, 5 security REST, 15 GitOps e2e, 15 fault and
35 integration tests. The benchmark figures above are from the earlier recorded run, not a new benchmark.

```sh
SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:55439/postgres \
  cargo test -p sagat-tests --locked --no-fail-fast \
    --test contract_manifest_gitops_versioning --test contract_rest_gitops_versioning \
    --test contract_rest_security_multitenancy --test e2e_gitops_versioning \
    --test fault_gitops_versioning --test integration_gitops_versioning
```

Use the URL of a disposable PostgreSQL instance you control; the suites create separate databases per test.
`cargo audit --deny warnings` passed, and OpenAPI lint passed with 9 existing warnings. Architecture validation
passed for 20 crates, together with its 8 rule tests. Studio `check:api` and TypeScript checking passed.

### Two independent reviews (a security one and a correctness one)

Two read-only reviews of the whole feature, by agents that had not written the code, found no critical or high
defect and about thirty points between them. Each was checked against the code before anything was changed; the
real ones were fixed, and the ones that are fixed have a test (those marked *fails without the fix* were run
without it to see them fail). What was not fixed is recorded in `research.md` (G209–G214).

| # | Severity | Finding | Verdict and fix | Evidence |
|---|---|---|---|---|
| C1 | Medium | `sagat publish` after a rollback replayed the answer to the first publication of the same document (same key) and said "activated" while the pointer stayed where the rollback left it | Real. A publication that re-activates an existing version gets a key of its own (D205) | `e2e us04::us04_publishing_the_manifest_again_after_a_rollback_activates_the_document_in_git` (*fails without the fix*: the pointer stays at 1) |
| C2 | Medium | `rollback` without `--to` takes the previous version of the newest change: run again (a lost answer) it undid the rollback | Real. It refuses to go back from a rollback and asks for `--to` | `e2e us04::us04_rolling_back_again_without_saying_where_does_not_undo_the_rollback` (*fails without the fix*) |
| C3 | Medium | `POST /publication-requests/{id}/publish` did not honour its `Idempotency-Key`: a retry after a lost answer was `REQUEST_STATE`, and a racing call with the same key got a conflict | Real. A published request answers its own key from the publication's record, a racing call is answered after it takes the lock, any other key is refused (D201) | `fault requests.rs::a_retry_storm_with_one_key_gets_one_answer_and_later_the_same_one` |
| C4, S3 | Medium, low | A request could be stored for a draft revision that was saved over while it was prepared, and then held the only slot of the workflow | Real. Opening a request locks the draft row and compares the revision (D200) | `integration approvals.rs::a_request_is_not_born_for_a_revision_that_was_saved_over`, `fault requests.rs::a_request_is_never_left_open_for_a_revision_that_was_saved_over` |
| S1 | Medium | The approver rule was "not the requester", so the person who wrote the document could approve what a pipeline asked for | Real. The last person to save the draft is recorded and cannot decide the request either (D199); that the platform knows only the last editor, not the Git author, is G210 | `domain versioning.rs`, `integration approvals.rs::whoever_saved_the_draft_cannot_approve_a_request_somebody_else_opened` |
| S2 | Medium | A manifest that is a link to a one-line secret was echoed back in the parse error | Real for the echo: parse errors no longer quote the file. The manifest named on the command line is still followed if it is a link (G214 f) | `apps/cli/tests/manifest.rs::a_file_that_is_not_a_manifest_is_not_quoted_back` |
| C5 | Low-medium | The refusal found inside the publishing transaction (`RequestNotApproved`) had `details: {field: slug}`, no reason, no metric | Real. Its own reason, `REQUEST_NOT_APPROVED`, remediation and metric | `crates/api-http/tests/versioning_errors.rs` |
| C6 | Low | A 160 character key made a promotion that creates the workflow a 500 (the key derived for the creation was longer than the store takes) | Real. The derived key is short whatever the caller's is | `integration promotion.rs::a_promotion_with_the_longest_key_creates_the_workflow_in_the_target` (*fails without the fix*: `Internal`) |
| C7 | Low | `sagat validate` exited 0 with a mistyped `--token-file`, skipping the check it was given the flags for | Real. Credentials given and unusable are a failure; nothing given is a local check, said as such | `e2e cli.rs::credentials_that_were_given_and_cannot_be_used_stop_validate_instead_of_skipping_the_check` |
| C8 | Low | The comment and the docs said the key of `promote` makes a retry safe | Real. What makes it convergent is the state of the platform (`ALREADY_ACTIVE`, `REQUEST_OPEN`); the comment, ADR and runbook say so | |
| C9 | Low | `promote` treated `NOT_ENOUGH_APPROVALS` as a failure where `publish` treats it as waiting; a request published meanwhile was shown as pending | Real. Both wait (exit 3); a published one is done; a final one is a failure | Not covered by a test (it needs the policy raised between two calls of one command) |
| S4 | Low | The transaction that publishes a request did not compare its revision and digest, nor take the policy from the workflow's namespace; the schema did not tie versions, requests and workflows together | Real, a second line: it does now, and the database does | `integration approvals.rs::a_request_publishes_the_revision_and_the_document_it_names_and_no_other`, `isolation.rs::a_history_row_and_a_request_belong_to_the_workflow_they_name` |
| S5 | Low | The append-only triggers of the history were skipped where triggers are switched off; the identity of a request could be updated by the runtime role | Real. `ENABLE ALWAYS`, and a trigger that freezes what a request is about. A deferred check that the pointer equals the last history row is not built (G214 b) | `isolation.rs::the_history_stays_append_only_where_triggers_are_switched_off_for_replication`, `approvals.rs` (the update is refused) |
| S6 | Low | `http://localhost:80@evil.example` passed as this machine, and a configured proxy would have been handed a plain-http token | Real. The address is parsed as the client reads it, userinfo is refused, plain http bypasses proxies | `apps/cli/tests/client.rs` |
| S7 | Low | Release notes accepted control characters; `printable` let new lines and bidirectional overrides through, so a note could forge a row of a history | Real. Release notes and justifications are plain text server-side; the tool shows text others wrote on one line without format characters | `approvals.rs::release_notes_are_plain_text_…`, `apps/cli/tests/args.rs` |
| S8 | Low | Refused self-approvals, requests ended by an edit, the from-to of a policy change and the request behind a publication were not on the trail | Real. `publication.denied`, `publication.superseded`, from-to, "by request" (D204) | `approvals.rs` (3) |
| S9 | Low | No bound on churn (opening and withdrawing, flip-flopping the pointer, policy writes); one approver can veto for ever | **Not fixed**, recorded: G212 | |
| S10 | Low | `export` followed a planted link; `--request` was put in a path unchecked; the largest answer read was 128 MiB | Real. It creates files exclusively, checks the id, reads at most 64 MiB | `apps/cli/tests/args.rs`, `e2e cli.rs` |
| C10, S11 | Design | Lowering a policy does not release open requests; the pointer can be moved to versions nobody approved here; an administrator can lower, publish and restore alone; approvals do not expire | **Not changed**, recorded: G209, G211, G213 | |
| C11 | Low | A set of small points: `history` of a never published workflow exited 1; `approve` demanded a namespace though it is decided by an id; `export` could write more workflows than `publish` takes; non-UTF-8 arguments panicked; a rolling deploy, a cursor naming no row, a promotion that stops after replacing a clean draft, ties in the backfill | The first four fixed (`e2e cli.rs`); the rest recorded: G214 | |

## What was not built, and what could not be run

The gaps are `research.md` G193–G214. In short: no retention of artifacts or of the history (G193, G194); the server
does not gate the publication of a breaking change, the tool does (G195); a rollback is not subject to approvals
(G196); no Studio screens (G198); the manifest lists workflows and not scripts (G199); the tool is not packaged and
authenticates with a bearer token only (G203, G204).

On 2026-10-11 Docker answered health queries but refused container creation with an HTTP 500 and
`containerd.metadata.v1.bolt/meta.db: read-only file system`. The first focused run therefore failed before
database setup. The successful focused run used local PostgreSQL 18.6; PostgreSQL 16 in containers, broker and
sandbox container tests, and the Compose smoke gate still require a working Docker environment. No CI result
is inferred from local results. The benchmark was run on a shared machine (load average is in its report).
