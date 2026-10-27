# Kafka Zstandard dependency maintenance — 2026-10-27

Refs: T004-033, FR-004-005, NFR-004-SEC/COMP; v0.3 integration audit blocker.
Base origin/develop70f09435, bugfix/kafka-zstd-advisories. The root audit actually failed
with4vulnerabilities and10denied unsound warnings in zstd0.13.3/safe7.3/sys2.1.1.
The SDK PR54 CI job122468132701 independently failed at the root cargo audit step.

The verified84-file registry snapshot changes only the normalized rskafka manifest's zstd
minimum. The lock diff changes that package source and only the three codec versions.
No new advisory ignore, public API change, removed compression feature or upstream Rust edit.
See docs/engineering/dependency-overrides.md for source/lock identity and removal conditions.

Actual commands and outputs are the adjacent receipts. Audit exits0 against1380advisories,
627dependencies in1.71s. The real Kafka case passes1/0/0 in6.26s (271.83s with fresh owned
build), using one Zstd partition and one uncompressed partition, and retains original
poison/deduplication/offset-loss/restart assertions. Connector all-target strict Clippy exits0
in51.72s. Formatting, architecture24crates, agent configuration and actionlint pass.
Python tools:49run,48passed,1existing opt-in controlled-Compose-failure test skipped
(SAGAT_RUN_REAL_COMPOSE_DIAGNOSTIC is unset). The6new vendor negative controls all pass.

The bounded source review found mutable provenance could be coherently altered with source.
The named new negative test actually failed before the correction; manifest-anchor-red.txt
retains that failure. Hardcoding the canonical upstream file-map digest plus84-file count
closes it; the second focused review reports None. The root executed all gates; reviewer
reports are not runtime evidence. Owner-selected same-model review only, no external CLI.

This is focused dependency/compatibility evidence, not a full integrated v0.3 release pass.
CI, merge and final release-tree gates remain separate.
