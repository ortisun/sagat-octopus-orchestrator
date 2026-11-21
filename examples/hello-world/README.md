# hello-world

## Overview

The same two-activity greeting workflow served and started by five SDK language examples.

## Interface

- [dotnet](dotnet/README.md) — .NET hello-world command with Release build and launch wrappers.
- [go](go/README.md) — Go hello-world command with build and launch wrappers.
- [java](java/README.md) — Maven preparation and launch commands for the Java hello-world example.
- [python](python/README.md) — Python hello-world command with virtualenv preparation and launch wrappers.
- [typescript](typescript/README.md) — Preparation and launch commands for the TypeScript hello-world example.

## Invariants

- Commands require the four platform connection settings and use bounded client waits. [examples/hello-world/typescript/src/main.ts:setting] [examples/hello-world/typescript/src/main.ts:run]
- Compose produces the greeting consumed by deliver; a terminal status other than SUCCEEDED fails the command. [examples/hello-world/typescript/src/main.ts:run]

## Side Effects

- Preparation installs/builds the local SDK and example; execution starts a worker and a durable workflow run, then prints the outcome. [examples/hello-world/typescript/prepare.sh:npm_install] [examples/hello-world/typescript/src/main.ts:run]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
