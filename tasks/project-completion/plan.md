# Complete the project's open implementation tasks

User authorization: 2026-10-17, "Todas as tasks abertas do projeto".

Deliver the 11 open tasks of feature 014 and the 64 open tasks of feature 016 on
`feature/016-project-completion`, cut from `origin/develop@555a67b`. The canonical
feature task lists remain the requirement/evidence ledger. Feature 017 is already
implemented; reconcile it with the combined platform without changing its standalone
reference branch. Preserve every other worktree and its uncommitted work.

## Integration and dependencies

1. Reuse the committed workflow-primitives baseline and collection-source/compatibility
   branches. Resolve shared documentation and harness modules by preserving both additions.
2. Reconcile feature 017's protocol, migration identities, root task pins and ADR identity
   under ADR-0036. Preserve scoped worker identity and async provider security together.
3. Establish feature 016's pure domain, exact wire contract, publication findings, bounded
   schema interpreter and immutable rules pin. New migrations start at 20261201000000.
4. Implement input/result/journal disclosure and namespace reveal grants. Read, current
   authority, projection and audit commit before any value leaves. Then restrict event reads.
5. Add schema options to all SDKs, explicit CLI disclosure/grants and ephemeral Studio views.
6. Finish feature 014's remaining accepted collection/nesting/reservation and continuation
   behavior. Fix transition cost before accepting the required large-workload evidence.
7. Execute the integrated compatibility, tenant/canary/fault, five-SDK, browser, performance,
   architecture, contract, build/advisory and Compose gates. Close tasks only with actual evidence.

## Work ownership

Root owns integration, conflict resolution, canonical schemas/protocol/OpenAPI, migrations,
application composition, shared harness registration and final documentation/evidence. Agents
use exclusive isolated worktrees and explicit file ownership. Domain and SDK authoring can
proceed independently of the root integration; their commits are reviewed before merging.
Schema interpretation precedes disclosure, grants precede event reveal, and frozen wire
types precede client UI. No agent writes another agent's worktree.

## Verification

Each implementation slice has tests first and a focused real verification command. One
coordinated build cache avoids multiplying storage; no unrelated build or container is stopped.
Run the existing integration and fault targets with real disposable infrastructure and bounded
parallelism. Capture all failures and skips. Reuse an earlier result only when its tested tree
and scope remain applicable. Independent review covers security and transactional decisions.

No existing acceptance criterion is replaced by an unsupported-publication refusal or an
unmeasured lower ceiling. Unreleased cross-feature compositions outside the accepted scopes
fail explicitly, while the accepted 014 and 016 tasks still require implementation and evidence.

## Operational limits

The integration migration profile targets the released develop baseline plus the preserved
014 migration history. A standalone, unreleased 017 database is not an accepted in-place
upgrade baseline: its SQLx version identities collide with 014. Preserve its reference branch
and backup; do not edit a deployed migration ledger or claim an untested bridge. Production
rollout, owner-only visual acceptance and unrelated uncommitted 013/015 draft specifications
are outside this implementation request.
