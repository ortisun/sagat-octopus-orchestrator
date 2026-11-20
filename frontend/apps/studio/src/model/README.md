# model

## Overview

Pure workflow IR editing, local advice, graph layout/projection, reference rewriting, exact JSON parsing, and status view models.

## Interface

- `Advice`/`advise` and `databaseIssue` provide browser authoring diagnostics; `object` narrows object values. [frontend/apps/studio/src/model/advice.ts:Advice] [frontend/apps/studio/src/model/advice.ts:advise] [frontend/apps/studio/src/model/database.ts:databaseIssue] [frontend/apps/studio/src/model/database.ts:object]
- `EditError`/`EditErrorCode` describe edit failures; node/link/field/entrypoint operations include add/remove/rename, connect/disconnect, cycle checks, and orphan detection. [frontend/apps/studio/src/model/edit.ts:EditError] [frontend/apps/studio/src/model/edit.ts:EditErrorCode] [frontend/apps/studio/src/model/edit.ts:addNode] [frontend/apps/studio/src/model/edit.ts:removeNode] [frontend/apps/studio/src/model/edit.ts:renameNode] [frontend/apps/studio/src/model/edit.ts:connect] [frontend/apps/studio/src/model/edit.ts:disconnect] [frontend/apps/studio/src/model/edit.ts:wouldCreateCycle] [frontend/apps/studio/src/model/edit.ts:setField] [frontend/apps/studio/src/model/edit.ts:setEntrypoint] [frontend/apps/studio/src/model/edit.ts:orphans]
- `IrNode`, `IrTrigger`, and `IrDocument` describe editable IR; `emptyDocument`, `readDocument`, `canonical`, `sameDocument`, and `findNode` read and compare documents. [frontend/apps/studio/src/model/ir.ts:IrNode] [frontend/apps/studio/src/model/ir.ts:IrTrigger] [frontend/apps/studio/src/model/ir.ts:IrDocument] [frontend/apps/studio/src/model/ir.ts:emptyDocument] [frontend/apps/studio/src/model/ir.ts:readDocument] [frontend/apps/studio/src/model/ir.ts:canonical] [frontend/apps/studio/src/model/ir.ts:sameDocument] [frontend/apps/studio/src/model/ir.ts:findNode]
- Layout contracts and `layoutGraph`/`layeredLayout` calculate positions; `readableInitialViewport` calculates the initial camera. [frontend/apps/studio/src/model/layout.ts:LayoutNode] [frontend/apps/studio/src/model/layout.ts:LayoutEdge] [frontend/apps/studio/src/model/layout.ts:Direction] [frontend/apps/studio/src/model/layout.ts:layoutGraph] [frontend/apps/studio/src/model/layout.ts:layeredLayout] [frontend/apps/studio/src/model/initialViewport.ts:readableInitialViewport]
- `stepReferences`, `renameStepReference`, `expressionFields`, and `danglingReferences` handle expression references; the node registry exposes kind/field/handle contracts, NODE_KINDS, kindOf, and linksOf. [frontend/apps/studio/src/model/references.ts:stepReferences] [frontend/apps/studio/src/model/references.ts:renameStepReference] [frontend/apps/studio/src/model/references.ts:expressionFields] [frontend/apps/studio/src/model/references.ts:danglingReferences] [frontend/apps/studio/src/model/registry.ts:NodeKindSpec] [frontend/apps/studio/src/model/registry.ts:NODE_KINDS] [frontend/apps/studio/src/model/registry.ts:kindOf] [frontend/apps/studio/src/model/registry.ts:linksOf]
- `RunModel`, `modelFromGraph`, `applyChanges`, `edgeState`, and `needsAttention` project runtime graphs. Event/status views and `parseAuthoringJson` provide bounded display and exact-number parsing. [frontend/apps/studio/src/model/runGraph.ts:RunModel] [frontend/apps/studio/src/model/runGraph.ts:modelFromGraph] [frontend/apps/studio/src/model/runGraph.ts:applyChanges] [frontend/apps/studio/src/model/runGraph.ts:edgeState] [frontend/apps/studio/src/model/runGraph.ts:needsAttention] [frontend/apps/studio/src/model/events.ts:eventLabel] [frontend/apps/studio/src/model/events.ts:eventTone] [frontend/apps/studio/src/model/status.ts:nodeStatus] [frontend/apps/studio/src/model/status.ts:runStatus] [frontend/apps/studio/src/model/safeJson.ts:parseAuthoringJson]

## Invariants

- Local advice stays separate from platform validation and does not block save/publish. Database checks bound SQL to 64 KiB, parameters to 256, rows to 10,000, result bytes to 4 MiB, timeout to 30 seconds, and connect timeout to 10 seconds. [frontend/apps/studio/src/model/advice.ts:advise] [frontend/apps/studio/src/model/database.ts:databaseIssue]
- Node IDs have 1–64 characters, start alphanumerically, and otherwise allow letters, digits, dots, dashes, and underscores. readDocument preserves unknown fields and reports malformed/duplicate-ID nodes as unreadable. [frontend/apps/studio/src/model/ir.ts:NODE_ID] [frontend/apps/studio/src/model/ir.ts:readDocument]
- Edits reject invalid/occupied IDs, unknown nodes/kinds, self-links, cycles, duplicate multi-links, non-handle links, and reserved/link-field writes. Rename updates entrypoint, structural links, and parsed steps references. [frontend/apps/studio/src/model/edit.ts:EditErrorCode] [frontend/apps/studio/src/model/edit.ts:connect] [frontend/apps/studio/src/model/edit.ts:setField] [frontend/apps/studio/src/model/edit.ts:renameNode]
- Positions are calculated rather than stored in IR. Graphs above 1,500 nodes use the linear layered layout, which tolerates cycles. [frontend/apps/studio/src/model/layout.ts:LAYERED_ABOVE] [frontend/apps/studio/src/model/layout.ts:layoutGraph] [frontend/apps/studio/src/model/layout.ts:layeredLayout]
- Runtime changes are classified by sequence as reset, gap, stale, or applied; at most 500 recent events are retained. [frontend/apps/studio/src/model/runGraph.ts:applyChanges] [frontend/apps/studio/src/model/runGraph.ts:RECENT]
- Authoring JSON rejects numbers that cannot be preserved exactly. Status views carry text, shape, tone, and help; unknown labels are clipped to 32 characters. [frontend/apps/studio/src/model/safeJson.ts:parseAuthoringJson] [frontend/apps/studio/src/model/status.ts:StatusView] [frontend/apps/studio/src/model/status.ts:nodeStatus] [frontend/apps/studio/src/model/status.ts:runStatus]

## Side Effects

- None defined in this directory.

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
