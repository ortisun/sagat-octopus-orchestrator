# crates/domain

The pure core: state, events, plans, permissions, audit actions. It performs no I/O and knows no framework.

- No `axum`, `sqlx`, NATS, Kubernetes or `serde_json` as a representation of business rules (rule 5). `scripts/check-architecture.py` fails the build on a forbidden dependency, layer edge or cycle; run it after touching `Cargo.toml`.
- Time and randomness come in as arguments or ports, never from `now()` or an RNG inside a function.
- Errors are typed enums with a stable meaning; a new meaning of an existing API error code is a `details.reason`, because `Error.code` is a closed set.
- Matches over `Permission`, `AuditAction`, `ResourceType`, `NodeKind` and similar are exhaustive on purpose: a new variant must be decided in every table (roles, admission, projection, telemetry). Do not add a wildcard arm to make it compile.
- Prove invariants with property tests (`crates/domain/tests/*properties*`) as well as examples; a replay of the journal must equal the stored projection.
