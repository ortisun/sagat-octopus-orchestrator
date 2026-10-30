# ui

## Overview

Accessible React primitives for announcements, controls, dialogs, statuses, tabs, and responsive subscriptions.

## Interface

- `AnnouncerProvider`/`useAnnounce` deliver ordered polite messages; `Banner`, `Button`, `Dialog`, `ErrorState`, `Field`, and `StatusBadge` provide presentation primitives. [frontend/apps/studio/src/ui/Announcer.tsx:AnnouncerProvider] [frontend/apps/studio/src/ui/Announcer.tsx:useAnnounce] [frontend/apps/studio/src/ui/Banner.tsx:Banner] [frontend/apps/studio/src/ui/Button.tsx:Button] [frontend/apps/studio/src/ui/Dialog.tsx:Dialog] [frontend/apps/studio/src/ui/ErrorState.tsx:ErrorState] [frontend/apps/studio/src/ui/Field.tsx:Field] [frontend/apps/studio/src/ui/StatusBadge.tsx:StatusBadge]
- `TabSpec`/`Tabs` implement ARIA tabs, and `useMediaQuery` supplies reactive media-query state. [frontend/apps/studio/src/ui/Tabs.tsx:TabSpec] [frontend/apps/studio/src/ui/Tabs.tsx:Tabs] [frontend/apps/studio/src/ui/useMediaQuery.ts:useMediaQuery]

## Invariants

- Announcements retain at most five messages and expire each after 120 seconds; error/warning banners use alert while information/success use status. [frontend/apps/studio/src/ui/Announcer.tsx:KEPT] [frontend/apps/studio/src/ui/Announcer.tsx:LIVES_MS] [frontend/apps/studio/src/ui/Banner.tsx:Banner]
- Buttons default to type=button. Dialog cancellation is delegated to the owner after preventing browser cancellation, and children render only while open. [frontend/apps/studio/src/ui/Button.tsx:Button] [frontend/apps/studio/src/ui/Dialog.tsx:Dialog]
- Field binds label/hint/error ARIA attributes to its control. Tabs wrap arrows and support Home/End with linked tab/tabpanel relationships. [frontend/apps/studio/src/ui/Field.tsx:Field] [frontend/apps/studio/src/ui/Tabs.tsx:Tabs]
- Missing matchMedia and server snapshots resolve to false. [frontend/apps/studio/src/ui/useMediaQuery.ts:useMediaQuery]

## Side Effects

- Schedules message-expiry timers and updates the polite live region. [frontend/apps/studio/src/ui/Announcer.tsx:AnnouncerProvider]
- Opens/closes native dialogs and calls owner close callbacks; registers/removes MediaQueryList change listeners. [frontend/apps/studio/src/ui/Dialog.tsx:Dialog] [frontend/apps/studio/src/ui/useMediaQuery.ts:useMediaQuery]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
