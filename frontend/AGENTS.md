# frontend

The Studio: a same-origin React app over the REST API (ADR-0016). Types come from the contract, not from hand-written interfaces.

- After any change to `specs/001-platform-foundation/contracts/openapi.yaml`: `cd frontend/apps/studio && npm run api`, then `npm run check:api`. CI fails when `src/api/schema.d.ts` is behind the contract.
- Gate before a PR, from `frontend/` (the order CI uses): `npm ci`, `npm audit --audit-level=moderate`, `npm run lint`, `npm run typecheck` and `npm run typecheck --workspace @sagat/e2e`, `npm test`, `npm run build`, `npm run check:api`. The browser journeys are Rust-driven and ignored locally without Node and Chromium: `cargo test -p sagat-tests --test e2e_workflow_studio --locked -- --ignored journeys` (the TypeScript SDK is built first by `bash sdks/typescript/conformance/prepare.sh`).
- Tests do not assert a transient state (a "saving" label that a fast machine skips): use `waitFor`, fake timers or a gate. Prefer role and text queries.
- Show a level or a state by text and shape as well as colour; never render a value the API withheld, and never put a secret or a token in the DOM, a log or the URL.
