# Patched matcher for the locked frontend tools

This private, tooling-only workspace preserves the callable CommonJS/default import used by
jsx-a11y and Redocly while exposing the named imports used by TypeScript ESLint and ESLint.
Every operation delegates to the **unmodified, exactly pinned minimatch 11.0.1** installed as
`minimatch-upstream`. There is no copied matcher, install-time patch or audit exclusion.

Its local identity is `minimatch@11.0.1+sagat.1`: the build suffix identifies this repository's
import bridge, not an upstream release. Six consumer-scoped npm overrides select this workspace
by version. A global override would conflict with npm's direct workspace link; a relative
`file:` override produced broken nested links with Node 22's npm. The scoped version overrides
produce a single canonical workspace link and preserve the other tools' locked versions.

The npm lockfile separately records the unmodified upstream registry package and its integrity.
Install with `npm ci --ignore-scripts`; the Docker Studio build copies this workspace before
its filtered install and runs the same compatibility tests before building.

This is **not** a general replacement for every minimatch 3/5 API. Version 11 removes `makeRe`,
`AST`, RegExp-backed `set` entries and other internals, and fixes several edge-case matches.
None of the current consumers uses those removed APIs. Upgrades of these consumers must run
`npm run test:tooling` plus lint, types, unit tests, build and `check:api`; the tests exercise
actual Redocly header selection, jsx-a11y wildcard controls and both module import forms.
The bounded amplification regression checks the actual resolved upstream parser in a child
process with a memory/time limit. The audit remains mandatory.

Remove this bridge and override when all consumers accept the patched upstream interface.
Do not widen its compatibility claims or implement old regex generation to accommodate a new
consumer. Review new requirements explicitly instead.

Sources: [advisory](https://github.com/advisories/GHSA-g3hv-75wg-m249),
[upstream fix](https://github.com/isaacs/minimatch/commit/77a0f5c),
[11.0 changelog](https://github.com/isaacs/minimatch/blob/v11.0.1/changelog.md).
