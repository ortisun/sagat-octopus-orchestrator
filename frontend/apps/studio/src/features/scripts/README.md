# scripts

## Overview

Script listing and editing, revisioned draft saving, sandbox test runs, publication/build following, and lazy Monaco editing.

## Interface

- `ScriptsPage` lists, creates, paginates, opens, and deletes scripts; `ScriptEditorPage` composes source/schema editing, tests, versions, limits, and publication. [frontend/apps/studio/src/features/scripts/ScriptsPage.tsx:ScriptsPage] [frontend/apps/studio/src/features/scripts/ScriptEditorPage.tsx:ScriptEditorPage]
- `scriptsApi` exposes namespace-id-scoped script CRUD, version list/get/publish, and test-run list/start/get operations. [frontend/apps/studio/src/features/scripts/api.ts:scriptsApi]
- `DraftSaver` exposes external-store subscription, draft changes, saving, conflict adoption/merge, and changed-field inspection. [frontend/apps/studio/src/features/scripts/draft.ts:DraftSaver]
- `CodeEditor`/`EditorControls` form the editing boundary, loading Monaco lazily while retaining a plain-text fallback; `createEditor` is the Monaco facade with worker/theme/model/problem helpers. [frontend/apps/studio/src/features/scripts/CodeEditor.tsx:CodeEditor] [frontend/apps/studio/src/features/scripts/CodeEditor.tsx:EditorControls] [frontend/apps/studio/src/features/scripts/editorModule.ts:loadEditor] [frontend/apps/studio/src/features/scripts/monaco.ts:createEditor] [frontend/apps/studio/src/features/scripts/monacoSetup.ts:workerKind] [frontend/apps/studio/src/features/scripts/monacoSetup.ts:modelAddress] [frontend/apps/studio/src/features/scripts/monacoSetup.ts:tidyProblems]
- `TryPanel` and `useTestRun` manage draft testing/history; `PublishDialog` and `useVersionList` manage publication and build following. [frontend/apps/studio/src/features/scripts/TryPanel.tsx:TryPanel] [frontend/apps/studio/src/features/scripts/useTestRun.ts:useTestRun] [frontend/apps/studio/src/features/scripts/PublishDialog.tsx:PublishDialog] [frontend/apps/studio/src/features/scripts/queries.ts:useVersionList]
- Supporting exports provide bounded line diffs, JSON/schema validation/equivalence, hostile-text sanitization/clipping, language metadata, status presentation, and bounded polling. [frontend/apps/studio/src/features/scripts/diff.ts:lineDiff] [frontend/apps/studio/src/features/scripts/jsonText.ts:checkSchemaText] [frontend/apps/studio/src/features/scripts/jsonText.ts:sameSchema] [frontend/apps/studio/src/features/scripts/text.ts:sanitizeLog] [frontend/apps/studio/src/features/scripts/text.ts:clipText] [frontend/apps/studio/src/features/scripts/languages.ts:LANGUAGES] [frontend/apps/studio/src/features/scripts/status.ts:testRunStatus] [frontend/apps/studio/src/features/scripts/polling.ts:startPolling]

## Invariants

- Query keys include namespace IDs so cached script data is scoped to its namespace. [frontend/apps/studio/src/features/scripts/queries.ts:keys]
- Draft saves send the server revision and only changed valid fields. Empty schemas become null; size/syntax violations are withheld. Identical request fingerprints reuse an idempotency key. [frontend/apps/studio/src/features/scripts/draft.ts:DraftSaver.#changes] [frontend/apps/studio/src/features/scripts/draft.ts:DraftSaver.#send]
- Revision conflict stops autosave without losing local text. Rejected text waits for an edit, while retryable failures use bounded delayed retries. [frontend/apps/studio/src/features/scripts/draft.ts:DraftSaver.#failed] [frontend/apps/studio/src/features/scripts/draft.ts:SaveState]
- Publication saves and rereads the draft, then refuses a revision that differs from the author snapshot. Sandbox testing starts only after the complete draft is flushed; one bounded poller watches it. [frontend/apps/studio/src/features/scripts/PublishDialog.tsx:PublishBody] [frontend/apps/studio/src/features/scripts/useTestRun.ts:useTestRun]
- Polling permits one request in flight, settles once, aborts on stop, and ends at its duration limit, final value, or failure limit. [frontend/apps/studio/src/features/scripts/polling.ts:startPolling]
- Schema validation requires object schemas and bounded UTF-8 size. Log/label processing strips terminal/control sequences, reveals invisible characters, and clips without splitting surrogate pairs. [frontend/apps/studio/src/features/scripts/jsonText.ts:checkSchemaText] [frontend/apps/studio/src/features/scripts/jsonText.ts:checkInputText] [frontend/apps/studio/src/features/scripts/languages.ts:LIMITS] [frontend/apps/studio/src/features/scripts/text.ts:sanitizeLog] [frontend/apps/studio/src/features/scripts/text.ts:clipText] [frontend/apps/studio/src/features/scripts/text.ts:clipLabel]
- Monaco does not fetch JSON schemas; workers are bundled, and models/listeners/schema associations are disposed together. [frontend/apps/studio/src/features/scripts/monaco.ts:applyJsonSchemas] [frontend/apps/studio/src/features/scripts/monacoSetup.ts:monacoEnvironment] [frontend/apps/studio/src/features/scripts/monaco.ts:createEditor]
- The current UI language registry marks Go and Java unavailable; TypeScript, Python, and C# are the built languages exposed here. [frontend/apps/studio/src/features/scripts/languages.ts:LANGUAGES]

## Side Effects

- Performs HTTP reads/mutations on namespace scripts, versions, and test runs through encoded path construction. [frontend/apps/studio/src/features/scripts/api.ts:scriptsApi]
- Schedules autosave/retry timers and supplied update calls; leaving the last subscription can send one pending update. Tests/builds create abortable pollers, invalidate queries, and announce state changes. [frontend/apps/studio/src/features/scripts/draft.ts:DraftSaver.#arm] [frontend/apps/studio/src/features/scripts/draft.ts:DraftSaver.#leave] [frontend/apps/studio/src/features/scripts/useTestRun.ts:useTestRun] [frontend/apps/studio/src/features/scripts/PublishDialog.tsx:PublishBody] [frontend/apps/studio/src/features/scripts/queries.ts:useVersionList]
- Persists plain-editor preference in localStorage and dynamically imports the Monaco wrapper. Monaco installs the worker environment, configures language services, creates bundled workers, and watches theme/media changes. [frontend/apps/studio/src/features/scripts/preference.ts:writePlainPreference] [frontend/apps/studio/src/features/scripts/editorModule.ts:loadEditor] [frontend/apps/studio/src/features/scripts/monaco.ts:watchTheme] [frontend/apps/studio/src/features/scripts/monacoSetup.ts:monacoEnvironment]
- Navigates after creation, guards unsaved work with beforeunload, invalidates caches, and copies requested IDs/digests to the clipboard. [frontend/apps/studio/src/features/scripts/CreateScriptDialog.tsx:CreateForm] [frontend/apps/studio/src/features/scripts/ScriptEditorPage.tsx:Editor] [frontend/apps/studio/src/features/scripts/CopyButton.tsx:CopyButton]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
