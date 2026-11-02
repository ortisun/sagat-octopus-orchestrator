# auth

## Overview

Browser OIDC Authorization Code with PKCE and an in-memory authenticated session store.

## Interface

- `OidcSettings`, `StartDeps`, `FinishDeps`, CALLBACK_PATH, newVerifier, challengeOf, startLogin, and finishLogin define the OIDC flow. [frontend/apps/studio/src/auth/oidc.ts:OidcSettings] [frontend/apps/studio/src/auth/oidc.ts:StartDeps] [frontend/apps/studio/src/auth/oidc.ts:FinishDeps] [frontend/apps/studio/src/auth/oidc.ts:CALLBACK_PATH] [frontend/apps/studio/src/auth/oidc.ts:newVerifier] [frontend/apps/studio/src/auth/oidc.ts:challengeOf] [frontend/apps/studio/src/auth/oidc.ts:startLogin] [frontend/apps/studio/src/auth/oidc.ts:finishLogin]
- `SessionInfo`, `SessionEnd`, `SessionSnapshot`, `expiryOf`, and `SessionStore` expose session state and lifecycle. [frontend/apps/studio/src/auth/session.ts:SessionInfo] [frontend/apps/studio/src/auth/session.ts:SessionEnd] [frontend/apps/studio/src/auth/session.ts:SessionSnapshot] [frontend/apps/studio/src/auth/session.ts:expiryOf] [frontend/apps/studio/src/auth/session.ts:SessionStore]

## Invariants

- OIDC endpoints require HTTPS except HTTP loopback, the exact issuer origin, and no embedded credentials. Discovery/token exchange omit credentials, reject redirects, disable caching, and suppress referrers. [frontend/apps/studio/src/auth/oidc.ts:allowed] [frontend/apps/studio/src/auth/oidc.ts:discover] [frontend/apps/studio/src/auth/oidc.ts:finishLogin]
- The callback removes pending PKCE state and replaces the address before validation/exchange. Returned tokens must be bearer-typed and match the accepted three-part shape. [frontend/apps/studio/src/auth/oidc.ts:finishLogin]
- expiryOf reads exp for display only; whoAmI establishes platform acceptance. Tokens enter the private in-memory field only after successful sign-in, and sign-out clears token and expiry timer. [frontend/apps/studio/src/auth/session.ts:expiryOf] [frontend/apps/studio/src/auth/session.ts:SessionStore.signIn] [frontend/apps/studio/src/auth/session.ts:SessionStore.signOut]

## Side Effects

- Uses cryptographic randomness/SHA-256, sessionStorage, discovery/token fetches, browser navigation, and history replacement. [frontend/apps/studio/src/auth/oidc.ts:newVerifier] [frontend/apps/studio/src/auth/oidc.ts:challengeOf] [frontend/apps/studio/src/auth/oidc.ts:startLogin] [frontend/apps/studio/src/auth/oidc.ts:finishLogin]
- Schedules expiry sign-out and notifies session subscribers. [frontend/apps/studio/src/auth/session.ts:SessionStore]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
