# Current canonical contract gates — 2026-10-18

Root executed the exact four repository CI validation commands at23127c38 without source changes. OpenAPI passes3.75s, Protobuf compilation0.14s, AsyncAPI4.54s, workflow example/IR JSON Schema1.99s. AsyncAPI reports zero errors/warnings and one informational newer-standard notice; no contract version was changed.

One real Rust invocation passes all three targets: payload classification18/18 (7.25s), platform foundation30/30 (1.27s), workflow primitives47/47 (0.45s), zero ignored/failures. Wall78.73s includes64s compilation. This is95 actual tests across three targets, not the whole workspace.1125 native source/contract input hashes match before/after the gates. CLI-specific input checksum seal is after-only, explicitly labelled; exact commands and raw outputs are retained.

Local environment is macOS/arm64, Node26.12.0 (CI config uses22), protoc36.2, Redocly1.34.21 selected through existing CI @1 and AsyncAPI CLI3.6.0. The existing npx dependency resolution is not a locked reproduction guarantee; no tooling/dependency/CI setting was changed. See environment.json for actual inputs and versions.

This supplies current contract evidence only for still-open T014-043. Locked workspace, final Clippy/format/architecture, Compose smoke and feature-specific recovery/resource/activation gates are separate; no canonical task closes. Root executed; no agent-reported test result is substituted.
