# Send signal — T007-056 / G167

The owner's 2026-10-12 completion request includes the remaining Studio action. This slice
uses the already approved feature 002 signal API without changing execution semantics.

## Acceptance and implementation

An authorized `run:signal` user viewing an unfinished run can open Send signal, enter a signal
name and a JSON object, review that all matching waiting steps receive it, then explicitly
confirm. Explain that signals are not buffered and that no matching wait returns a conflict.
Do not accept invalid names, non-object JSON or payloads beyond the server's byte limit.
Use the existing generated response type, canonical error view, W3C request instrumentation
and server audit. Preserve one idempotency key for retries of an identical request and change
it for changed input. Do not persist or log the payload in the browser. Discard the form on
run/namespace/session changes; ignore late responses. Refresh the run and journal on success.
No custom reason is invented: the existing signal contract audits the authenticated actor.

Tests: permission/terminal suppression; invalid name/JSON/size; explicit confirmation and
correct API path/body; stable retry key; context change; failure and success; real Chromium
journey against a waiting run. No task completion claim until verification is recorded.
