# @sagat-octopus-orchestrator/workflows-bundler

`sagat-workflows bundle` makes the one script a TypeScript worker loads to run workflow code (design 044, Technical
Plan 1.2 and 3.2): the workflow modules, every module they import and the SDK's determinism runtime, bundled with
esbuild; and the manifest that names each workflow type's code digest.

```sh
TZ=UTC npx sagat-workflows bundle src/workflows.ts --out dist/workflows --refuse ./activities
```

- Each exported function of the entry module is a workflow; its export name is its workflow type
  (`^[A-Za-z][A-Za-z0-9_-]{0,47}$`).
- Imports of Node's built-in modules, with or without `node:`, of the SDK's activity side
  (`@sagat-octopus-orchestrator/sdk` and its other entry points) and of the prefixes given with `--refuse` are refused
  before any code runs. `@sagat-octopus-orchestrator/sdk/workflows` resolves to the runtime alone (the export
  condition `sagat-workflow`), which is evaluated first, so the globals are replaced before any workflow module runs.
- The digest is the SHA-256 of the canonical JSON of the language, the runtime (Node's version and its ICU data), the
  SDK's name and version, the workflow type and the SHA-256 of the bundle file. Run the command with the Node the
  workers run: a worker recomputes the digests from the file it loads and refuses a manifest of another runtime.
- The bundle has no absolute path of the build machine: module paths are relative to the project's `package.json`.

Its one dependency, esbuild, is pinned to an exact version and locked with its integrity; install it with
`npm ci --ignore-scripts` (esbuild's install script only checks its platform binary, which is an optional dependency
of its own) and check the registry's signatures with `npm audit signatures`.

## Verification

```sh
(cd ../typescript && npm ci --ignore-scripts && npm run build)
npm ci --ignore-scripts && npm audit signatures && npm test
```
