# Exact selected-test guard, 2026-10-27

A real negative control injected a nonexistent libtest selector into the in-process scenario
mapping while retaining the actual verified builds. The old runner accepted exit0/running0tests
as compatibility success. The control failed its required rejection assertion in4.43s (RED).
The control script and its exact output are retained; its prepared artifact path is historical.

The runner now checks the exact selected case name and the final outer libtest summary:
exactly1passed/0failed/0ignored. Successful nested old phases cannot mask zero/failed/multiple
outer cases. Unix-only fixtures are rejected explicitly on unsupported hosts. Five new outcome
controls plus eight existing source-boundary tests pass13/0 in0.081s. The actual negative control
then passes in2.82s because zero matching tests are rejected before any evidence-success message.

After the fix the actual source-verified scenarios pass separately:

| Scenario | Coordinator | Outer runner |
| --- | --- | --- |
| active-data | 1/0/0,6.47s | 9.43s |
| inflight-worker | 1/0/0,5.53s | 8.31s |
| applied-lookup | 1/0/0,5.35s | 8.63s |

These are separate invocations, not a summed aggregate. The underlying Rust binary/fixtures
are unchanged from bd4f611d; this fixes only the runner's acceptance check. Final Applied build
metadata records the unchanged binaries and the newer Python input snapshot. Transcript
whitespace normalization and both checksums are recorded. No capabilities/secrets are stored.
