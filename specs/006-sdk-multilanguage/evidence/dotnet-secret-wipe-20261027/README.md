# C# credential observation test race — 2026-10-27

Refs: T006-035, NFR-006-SEC. Actual CI RED: job122484043817 in run40587867281,
PR55 headb2e1671e:449passed/1failed, expected names0 but observed sentinel-1.
The test file blob7d9546ae3f9f7283cf9e58663c7c07046e6c6e09 matches origin/develop70f09435
exactly, so this is existing test behavior rather than the Kafka codec update.
The adjacent excerpt retains the exact assertion/source location and no credentials.
Source: https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40587867281/job/122484043817

FakeWorkerServer records the CompleteStep before invoking OnComplete. Seeing one recorded call
never proved the callback had assigned its observation; the test could read-1 meanwhile.
The callback still samples kept.Names.Count at the same point. A TaskCompletionSource with
asynchronous continuations now gates the assertion on that completed sample, with a bounded
10s failure timeout and an explicit single-report assertion. Empty/disposed secret and refused
later resolution assertions remain. No SDK production code, delay/sleep, retry or assertion
removal is introduced.

Actual full dotnet test:450passed/0failed/0skipped in21s harness,26.92s command including restore.
Both SDK target frameworks build. dotnet format --verify-no-changes --no-restore exits0 in7.93s.
The first local --no-restore command correctly refused absent project.assets.json in the new
worktree; it executed no tests and is recorded as setup failure, not behavior RED. The CI failure
above is the real RED. Text copies normalize trailing whitespace; original logs remain in the
agent-owned /tmp verification directory. Other SDK source is unchanged from the prior checks.
