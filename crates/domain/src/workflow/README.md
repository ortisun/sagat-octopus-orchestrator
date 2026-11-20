# crates/domain/src/workflow

## Overview

Pure domain proof object for the frozen version-1 static shape of a successful root leaf-collection path.

## Interface

- `CollectionSuccessShapeError` distinguishes unsupported owner, root definition and body. [collection_shape.rs:CollectionSuccessShapeError]
- `LeafCollectionSuccessShape` is re-exported by `workflow`; `check` constructs it and accessors expose contract version, owner, body, ordered activity IDs and return node. [collection_shape.rs:LeafCollectionSuccessShape] [collection_shape.rs:LeafCollectionSuccessShape::check] [../workflow.rs:LeafCollectionSuccessShape]

## Invariants

- The selected owner must be a root `ForEach`; the definition must be structured, without continuation policy/triggers, with one fully visited acyclic root chain of ordinary prefix activities, one or more collections and a terminal `End`. [collection_shape.rs:LeafCollectionSuccessShape::check]
- Root nodes cannot have `on_error` or effects, every body must be referenced and the selected body must exist. [collection_shape.rs:LeafCollectionSuccessShape::check]
- Ordinary activities are declared, non-script, non-database and effect-free, with no compensation/reconciliation companions. [collection_shape.rs:ordinary]
- Each body chain has one to eight activities and its sole terminal `Return`; selected-body activities must be ordinary, and all body nodes are declared without `on_error`. [collection_shape.rs:chain]
- The proof has contract version `1`; it deliberately contains no workflow version, run or settled flag. [collection_shape.rs:LeafCollectionSuccessShape::contract_version]

## Side Effects

- `check` and `chain` traverse/allocate in-memory sets, vectors and IDs without ports, storage, network, clock, randomness or process work. [collection_shape.rs:LeafCollectionSuccessShape::check] [collection_shape.rs:chain]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-domain --locked
```

These commands were not run in this documentation-only pass.
