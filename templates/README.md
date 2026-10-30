# Workflow starter templates

## Overview

CLI starter assets. The source-bearing script template is indexed below; declarative-only starters remain in their existing directories.

## Interface

The child contracts define the source boundaries in this directory.

## Subdirectories & Boundaries

- [inline-script](./inline-script/README.md) — Order enrichment script and artifact-pinned workflow.

## Invariants

- No additional executable invariant is introduced by this grouping; source checks and contract requirements are documented in the linked components.

## Side Effects

- None from the directory grouping itself. The linked component contracts describe command execution, storage and network effects.

## Verification

Run from the repository root:

```sh
cargo test -p sagat-cli --locked
```

These are verification entry points; no runtime test result is implied by this index.
