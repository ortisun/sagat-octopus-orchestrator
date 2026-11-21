# apps

## Overview

Browser applications in the frontend npm workspace.

## Interface

- [studio](studio/README.md) — Studio package and Vite, TypeScript, ESLint, and Vitest configuration.

## Invariants

- Studio dependency/tool versions are pinned. API types come from specs/001-platform-foundation/contracts/openapi.yaml, and build/typecheck use tsc --noEmit. [frontend/apps/studio/package.json:dependencies] [frontend/apps/studio/package.json:devDependencies] [frontend/apps/studio/package.json:scripts.api] [frontend/apps/studio/package.json:scripts.build] [frontend/package.json:engines.node]
- Frontend TypeScript uses ES2023, bundler resolution, strict/unchecked-index protection, isolated modules, and unused-declaration checks; Studio emits no TypeScript output. [frontend/tsconfig.base.json:compilerOptions] [frontend/apps/studio/tsconfig.json:compilerOptions]
- Production uses /studio/ as asset base, dist output, no source maps, and content-hashed filenames. Development listens on 5173 and proxies /v1 to SAGAT_API_URL or http://127.0.0.1:8080 without changing origin. [frontend/apps/studio/vite.config.ts:base] [frontend/apps/studio/vite.config.ts:build] [frontend/apps/studio/vite.config.ts:server]
- Lint excludes generated schema/build output and forbids explicit any, floating promises, dangerouslySetInnerHTML, eval, and implied eval. Vitest uses jsdom, a setup module, CSS disabled, and at most four workers. [frontend/apps/studio/eslint.config.js:ignores] [frontend/apps/studio/eslint.config.js:rules] [frontend/apps/studio/vitest.config.ts:test]

## Side Effects

- Package commands can generate schema.d.ts, build hashed dist assets, start development serving/proxying, and execute checking tools. [frontend/apps/studio/package.json:scripts] [frontend/apps/studio/vite.config.ts:server]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
npm --prefix frontend run check:api
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
