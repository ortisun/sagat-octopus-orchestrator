# Governance

The record of how the repository's rules changed. The rules themselves are in the [constitution](../constitution.md);
the decisions only the owner can take are in [owner decisions](../owner-decisions.md).

## Constitution amendments

Every amendment of the constitution comes with a Sync Impact Report: what changed, why the version moved the way it did,
the hash of each principle before and after, and the documents that had to follow. `scripts/check-design-docs.py`
checks the newest report against the constitution. Newest first:

| Report | Version |
|---|---|
| [constitution-5.1.0-sync-impact.md](constitution-5.1.0-sync-impact.md) | 5.1.0: two audited journal-deletion paths after a verified archive, a third declared set (a resource's settings and its OAuth access token), the throughput gate, and design 044's wording |
| [constitution-5.0.0-sync-impact.md](constitution-5.0.0-sync-impact.md) | 5.0.0: the full-parity scope of release 0.3.0: workflow as code beside the graph, one writable region per cell with fenced failover, internal apps and a curated hub admitted |
| [constitution-4.7.0-sync-impact.md](constitution-4.7.0-sync-impact.md) | 4.7.0: documentation first, words in full, and the identifiers written as words |
| [constitution-4.6.0-sync-impact.md](constitution-4.6.0-sync-impact.md) | 4.6.0: a change of a contract file runs the contract suites and the packages that read it |
| [constitution-4.5.0-sync-impact.md](constitution-4.5.0-sync-impact.md) | 4.5.0: a change of the step-log code runs the step-log suites |
| [constitution-4.4.0-sync-impact.md](constitution-4.4.0-sync-impact.md) | 4.4.0: a change of the engine's hot path runs the suite that budgets what it costs the database |
| [constitution-4.3.0-sync-impact.md](constitution-4.3.0-sync-impact.md) | 4.3.0: narrows what the CI of a pull request runs |
| [constitution-4.2.0-sync-impact.md](constitution-4.2.0-sync-impact.md) | 4.2.0: a merge into `develop` starts no CI run |
| [constitution-4.1.0-sync-impact.md](constitution-4.1.0-sync-impact.md) | 4.1.0: a pull request runs the tests of what it changes; a release runs everything |
| [constitution-4.0.0-sync-impact.md](constitution-4.0.0-sync-impact.md) | 4.0.0: design docs written with the Elephant-Goldfish Model replace Spec Kit |
| [constitution-3.0.0-sync-impact.md](constitution-3.0.0-sync-impact.md) | 3.0.0: the amendment its report calls "B01", split out of feature 037 and adopted on 2026-11-01 |
| [compensation-safety-sync-impact.md](compensation-safety-sync-impact.md) | 2.0.0: Principle III, compensation safety |
| [sync-impact-rust-sdk-2026-10-27.md](sync-impact-rust-sdk-2026-10-27.md) | 1.1.0: the official SDK languages grow from five to six with Rust (2026-10-27) |

## How documents name things

| Page | What it holds |
|---|---|
| [identifier-scheme.md](identifier-scheme.md) | The abbreviations to write out, the old and new form of every identifier (`T022-064` becomes `Task 022-064`), and the transition until the sweep |

## Plans that change the repository

| Plan | What it changes |
|---|---|
| [specs-into-docs-plan.md](specs-into-docs-plan.md) | Moving `specs/` under `docs/` and renaming the identifiers to words, in one mechanical pull request |
