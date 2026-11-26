# Resolution of round 10 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed commit of round 10: 4a943e7c15d9ca73e9c1afa6274ad5da8cfb5cbb

How the Majors of round 10 were answered, by the author under the owner's delegation (OD-2026-11-20-6,
OD-2026-11-24-4). The decisions are Decision 044-213 to Decision 044-222. Every Nit was applied.

Counts before fixes: design re-read Blocker 0, Major 4, Nit 6; implementation readiness re-read 0, 2, 8; security 0, 2,
6; architecture 0, 5, 14; correctness 0, 4, 10. Total Blocker 0, Major 17, Nit 44. Round 9 had Blocker 0, Major 16,
Nit 50; every round-9 Major is closed or partly closed and answered below.

| Findings | Answer |
|---|---|
| design A; correctness 2 (continue-as-new input as a blob) | Never a blob; inline with the state within 192 KiB; a typed SDK error (Decision 044-214) |
| design B (no storage model for command-value blobs) | `COMMAND_VALUE`, commitment by the accepting completion, a deferral's hold (Decision 044-215) |
| design C (SDK-issued commands and the batch limit) | 339 commands: 256 from the function, 83 from the SDK; 16 open scopes (Decision 044-216) |
| design D; security 1; correctness 3; architecture M3 (cancellation without grace after a cooperative request) | A no-grace arm that stops whatever was journaled; the grace in the idempotency key (Decision 044-217) |
| readiness A (conformance workflow worker) | `run.sh workflow-worker`, a separate process with its own environment and a line protocol (Decision 044-221) |
| readiness B (`d(k)` bytes) | A table of kinds and targets; hexadecimal digests (Decision 044-213) |
| security 2 (unbounded blob validation) | Each distinct blob once, at most 4 MiB per completion, a fair semaphore (Decision 044-215) |
| correctness 1 (carrier-dependent digest) | Every value field hashed as its descriptor (Decision 044-213) |
| correctness 4; architecture M1 (server-side fields and resets) | One place, `server_detail`, readers named, copied by the reset (Decision 044-218) |
| architecture M2 (replay-check phase and feature 018's executor) | `REPLAY_PENDING`, checks created ahead, `created_at`, `busy_checks` (Decision 044-219) |
| architecture M4 (migration locks) | Split validation; index builds; lock time stated (Decision 044-220) |
| architecture M5 (test against the pause rule) | Renamed to assert both halves |
