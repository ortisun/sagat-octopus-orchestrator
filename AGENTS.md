# Instructions for implementation agents — Sagat Octopus Orchestrator

You are implementing a durable workflow platform **from specifications**, not a set of loose scripts.

1. Read `.specify/memory/constitution.md` and the `spec.md`, `plan.md`, `tasks.md` of the active feature BEFORE changing code.
2. Do not implement functionality from another feature without an approved dependency. When diverging from an ADR, create a new ADR and update dependents.
3. A state transition, next-task scheduling, timer/subscription update and outbox **MUST** obey the documented transactional boundaries.
4. Do not promise exactly-once for external effects; require an idempotency key / reconciliation for critical effects.
5. The Rust domain must not import Axum, SQLx, NATS, Kubernetes, serde_json as a representation of business rules. Serde on wire/adapter types is allowed.
6. Do not create empty VO/DTO/model mappers. Use shared types at module boundaries when there is no independent semantics; map only at necessary boundaries.
7. Never run code inline in the control plane / scheduler process. Isolate, enforce quotas and block access to secrets beyond their capabilities.
8. The SDK protocol must have conformance tests common to TypeScript, Go, Python, C# and Java, without forking the server semantics.
9. Test crashes at critical points, duplicates, the race between timeout and signal, lost broker delivery, failed compensation and restart.
10. Update spec, contracts, tests and documentation in the same delivery. Always include OTel, tenant boundary, typed errors and auditing.
11. **Do not** create a fake implementation of unproven guarantees. Where there is a placeholder, mark it explicitly as a pending decision/test.
12. Run and report real tests; if that is not possible, explain the gap without inventing results.
13. **Do not** add `Co-Authored-By` trailers to commit messages.
14. Branching follows **Gitflow**: `main` holds releases and `develop` is the integration branch. Work happens on `feature/<spec-id>-<slug>`, `bugfix/<slug>`, `release/<semver>` and `hotfix/<semver>` branches; feature and bugfix branches are cut from `develop` and merged back with `--no-ff`. Never commit directly to `main` or `develop`.
15. Commit messages follow **Conventional Commits**: `type(scope): imperative subject` (subject ≤ 72 characters), with `type` one of `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `build`, `ci`, `chore`, `revert`. Mark breaking changes with `!` and a `BREAKING CHANGE:` footer. Reference the traced items in a footer, e.g. `Refs: T001-004, FR-001-001`.
16. **Infrastructure is self-contained.** Everything needed to run or test the platform is declared in this repository (`infra/docker/compose.yaml`, `compose.observability.yaml`, test fixtures that start their own disposable containers). Never rely on a container, volume or service of another project or one started by hand. A new component (for example OpenBao for feature 015) is added to the compose file with a pinned image, loopback-only ports and a healthcheck.

## Definition of Done

Traced requirement → automated tests → updated contract → essential metrics → logs without sensitive data → reproducible execution → documentation and demo of the happy/failure scenario.
