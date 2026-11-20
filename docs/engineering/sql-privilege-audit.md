# Static SQL privilege audit

The platform-foundation runtime_privileges suite extracts the repository's literal SQL and
checks its relation, column, function and row-lock needs against the real migrated catalog
as sagat_app. It also probes executable statements where its parameter model permits. This
is a lexical fitness function, not an interpreter for arbitrary Rust or runtime-built SQL.
Keep the least-privilege service suites as a separate gate.

The supported string forms include literals, concat!, fixed no-argument SQL predicates and
the existing prefix/literal-tail templates. Parent predicates are read from the actual
infra-postgres execution.rs source. Only their declarations enter the registry; parent SQL
statements are not copied to every child file. Exact crate::execution and $crate::execution
keys remain separate from local names. Child modules inherit the lexical parent; other files
need an explicit top-level use import for an unqualified call. Simple single/braced imports
and aliases are recognized. Local definitions cannot replace a qualified parent declaration.
Unfamiliar import forms do not receive invented bindings.

Expansion keeps unresolved calls separately from display text. Const substitution cannot
clear that state. In a recognized SQL expression or independently SQL-shaped expansion, an
unknown call is an explicit Need::Unresolved even when it precedes the SQL verb or supplies
the whole query. SQLx query, query_as, query_scalar and raw_sql argument boundaries include
generic type lists. Ordinary non-SQL Rust macros and labels are not SQL failures. Identifier
lines preserve direct macro invocation sites; literal lines remain the original caller lines.
This does not add arbitrary macro matching, procedural expansion or general Rust name resolution.
An unfamiliar SQL construction needs explicit audit support and tests, never a guessed expansion.

Run the integrated gate against the repository-owned disposable database infrastructure:

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 RUST_TEST_THREADS=1 cargo test -p sagat-tests --test integration_platform_foundation --locked runtime_privileges:: -- --nocapture
```

The lexer alone is self-contained and can be checked quickly with rustc --edition=2024 --test
on tests/integration/platform-foundation/runtime_privileges/rust_source.rs, writing its binary
to an owned temporary directory. It does not replace the migrated-catalog gate. Regression
fixtures include nested qualified predicates with row-lock needs, unresolved const collisions,
unknown prefixes/whole queries, module shadowing/import aliases, ordinary wrappers and caller lines.

## Verification checkpoint, 2026-10-27

Refs: T014-043, FR-014-015/017. The initial migrated-catalog gate passed32/fail1
(14.15s harness/14.16s command), reporting17 unresolved parent/qualified predicate sites.
No grant or ALLOWED exception was added. New lexer regressions genuinely passed10/fail2
(0.00s/29.67s) for omitted qualified nesting and an unknown nested call. The initial correction
passed the integrated37-case suite (12.30s/43.82s), but adversarial review found const erasure,
SQL-prefix classification loss and qualified/local namespace confusion. Added regressions
passed14/fail2 (0.00s/28.70s) before structured taint and separate scope resolution corrected them.

A second review found ordinary-wrapper false positives and missing direct-call lines; actual
standalone lexer regressions passed17/fail2 (0.00s/0.31s), then19/0/0 (0.00s/0.39s).
One malformed escaped test literal was corrected to a raw string after a formatting diagnostic,
before the corresponding compile; it is not a behavioral RED. The third bounded review found
no remaining concrete defect; reviewers did not run tests. External review remains skipped by
existing owner preference.

The root's final integrated runtime-role gate passes42/0/0 (14.30s harness/43.50s command):
1090 statements in56files,675 distinct needs checked,638 probed as sagat_app,9 statements from
non-definer triggers and74 SECURITY DEFINER triggers not followed by this static probe. Those
counts describe the completed working tree including the separately delivered scoped receiver.
Strict workspace/all-target Clippy passes29.82s; formatting and diff checks pass. A later
comment-only documentation clarification does not change the tested lexer behavior.
