# batch

## Overview

Batch operations (feature 018): starting a durable, rate-limited job from the filters of the run list, following its progress and what became of each run, and cancelling it.

## Interface

- `BatchFromRuns` is the button and dialog of the run list that turns its filters into a job: choose an action, count the runs, give a reason, confirm. [frontend/apps/studio/src/features/batch/BatchFromRuns.tsx:BatchFromRuns]
- `BatchesPage` lists the jobs of the namespace, newest first; `BatchPage` shows one job with its progress, its counters, the outcome of each run and the way to cancel it. [frontend/apps/studio/src/features/batch/BatchesPage.tsx:BatchesPage] [frontend/apps/studio/src/features/batch/BatchPage.tsx:BatchPage]
- `model` holds the pure rules: `selectorOf`/`describeFilters` (the selector of the filters), `requestOf` (the request a draft makes, with every check), `progress`, `jobState`/`itemState`/`reasonText`/`failureText` (words and shapes). [frontend/apps/studio/src/features/batch/model.ts:selectorOf] [frontend/apps/studio/src/features/batch/model.ts:requestOf] [frontend/apps/studio/src/features/batch/model.ts:progress]
- `useNamespaceId` resolves the slug the Studio holds to the id the routes carry; `notServed` recognises a deployment with batch operations off. [frontend/apps/studio/src/features/batch/queries.ts:useNamespaceId] [frontend/apps/studio/src/features/batch/queries.ts:notServed]

## Invariants

- The dialog is offered only with `run:batch`; the pages need `run:read`. Nothing is asked of the platform before a person acts, and the job is started only after the runs have been counted for the same limit and a reason of 8 to 2,000 characters. [frontend/apps/studio/src/features/batch/BatchFromRuns.tsx:BatchForm] [frontend/apps/studio/src/features/batch/model.ts:reasonProblem]
- The selector is the filters of the run list and nothing else, the business id prefix included (a literal, as the address of the list holds it; customer text, shown only as text in the sentence of what the job takes, never put in an address or a log). What was counted is what the job takes: a different limit or different filters are a different count, and the job is not started on a count that is not of the selector now. `UPDATE` and `REPIN` are not offered because the platform refuses them. An unchanged request keeps its idempotency key across a retry. [frontend/apps/studio/src/features/batch/model.ts:selectorOf] [frontend/apps/studio/src/features/batch/model.ts:describeFilters] [frontend/apps/studio/src/features/batch/BatchFromRuns.tsx:BatchForm] [frontend/apps/studio/src/features/batch/model.ts:BATCH_ACTIONS]
- A signal payload must be a JSON object within 256 KiB; the payload of a job is never shown, nor its digest, only that it has one. [frontend/apps/studio/src/features/batch/model.ts:requestOf] [frontend/apps/studio/src/features/batch/BatchPage.tsx:Summary]
- A job at work is followed (2 s for the job, 3 s for its items, 5 s for the list) and left alone once it has ended. [frontend/apps/studio/src/features/batch/BatchPage.tsx:Job] [frontend/apps/studio/src/features/batch/BatchesPage.tsx:Jobs]

## Side Effects

- The dialog posts a preview and then the job, announces the result and navigates to the job; the job page posts a cancellation with its reason and refreshes the job. [frontend/apps/studio/src/features/batch/BatchFromRuns.tsx:BatchForm] [frontend/apps/studio/src/features/batch/BatchPage.tsx:CancelForm]
