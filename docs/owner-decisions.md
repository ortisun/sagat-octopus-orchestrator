# Owner decisions

Decisions that only the product owner could take, with what each one unblocks. Each entry is also
recorded where the open item lived (gap register, ADR, runbook or specification). "Not built"
means the decision is taken and the implementation is still to be done; it is not evidence.

## 2026-10-16

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | G42, quota values | The current defaults (10,000 active runs per namespace and 50,000 per tenant; 1,000 workflows; 1,000 worker sessions; 500 secrets; 10 GiB content and 1 GiB logs; 100 API requests/s with burst 200; 1,000 events/s with burst 2,000; 30-day events; 365-day scripts) are the **single default tier**. Commercial tiers wait for a billing model; operators keep adjusting per tenant under the ceiling. | `crates/domain/src/quota.rs`, ADR-0027, 009 `research.md` | Done (no behaviour change) |
| 2 | G42/G80, payload classification | **Per field, four levels** (`public`, `internal`, `pii`, `secret`); an unmarked field is `pii`; `pii` needs a new audited permission; `secret` is never shown. | [ADR-0033](adr/ADR-0033-payload-values-are-classified-per-field.md), 004 and 009 `research.md` | Not built (needs its own feature) |
| 3 | SC-008-001 | **N = 10**: a diagnosis passes with at most 9 counted interactions. | 008 `spec.md`, `docs/validation/human-acceptance.md` | Done; the human sessions are still to run |
| 4 | G21, run result | Reaching `end` evaluates `outputExpr`, stores the result (at most 256 KiB, charged to the content quota) in the terminal transaction and gives it to the parent as `steps.<child>.output`; disclosure follows ADR-0033. | 002 `research.md` | Not built |
| 5 | G141, late heartbeat | Option (b): the server answers a heartbeat for an attempt the same worker already completed with a distinct non-error status; SDKs unchanged. | 006 `research.md` | Not built |
| 6 | G7, token revocation | **Accepted risk**: tokens live at most 1 hour, roles are read on every request, and suspending a membership is the incident response. | 001 `research.md` | Closed |
| 7 | G9, denied requests | **Every 403 is audited**, bounded by the existing per-subject rate limit. | 001 `research.md` | Not built |
| 8 | D241, RPO/RTO | Reference objectives for the Helm topology with a synchronous standby: **RPO 0** for acknowledged work, **failover RTO ≤ 5 min**, **PITR restore RTO ≤ 1 h**; the drill is to gate a release. An asynchronous standby states its own RPO. | 010 `research.md`, `docs/runbooks/disaster-recovery.md`, `docs/traceability.md` | Objectives published and enforced by the drill, which gates the `release` workflow; not measured on a real cluster |
| 9 | G48, secret delivery | A secret gets an optional list of **worker pools** allowed to receive it; a lease is not handed to another worker and the refusal is audited. | 009 `research.md` | Not built |
| 10 | G162/G163, Studio | The provisional `S` stays until the owner picks one of the three geometric logos (G162 open). **Scripts follows Workflows** in the navigation (G163). | 007 `research.md`, `frontend/apps/studio/src/app/Shell.tsx` | G163 done; G162 open |
| 11 | T007-049/T007-051 | The owner reviews the visual corrections of 2026-10-12 before accepting them. | — | Waiting for the owner |
| 12 | G123/G203/G216, publication | **GitHub Releases only**: SDK packages and `sagat` binaries with checksums and signatures; no npm, PyPI, NuGet or Maven for now; Go from the GitHub module path. | 006, 011, 012 `research.md`, `docs/runbooks/release.md` | Workflow built (`bugfix/github-releases`); first tagged release still to run |
| 13 | Two drafts numbered 013 | `013-federated-login` keeps 013; `013-otel-openbao` becomes **015** (`FR-015`, `SC-015`) and its draft ADR is renumbered (ADR-0022 is taken; next free after ADR-0033). Its constitution edits are redone on the ratified constitution. | `docs/roadmap.md` | Decided; the drafts live uncommitted in their own worktrees and are renumbered when they are committed |
