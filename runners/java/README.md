# Java inline runner

The builder accepts one top-level `Script.java`. `Script` must be public, have a public no-argument
constructor and implement `sagat.inline.InlineScript`. The input and result are Java maps, lists,
strings, booleans, numbers or null. `Octopus` provides mediated fetch, secret, log and the stable
step idempotency key.

The trusted builder runs `javac --release 17 -proc:none` with explicit arguments. It does not run a
project, Maven, Gradle, an annotation processor or options from the author. Lockfiles and external
dependencies are refused. The v3 artifact contains only `Script.class` and `Script$*.class`. The
runtime decodes them in memory with a parent-first loader, so the signed trusted API and pinned
Jackson JSON implementation cannot be replaced by the script. The JVM verifier remains the final
class-file validator; the container is the security boundary.

## Interface

- [api](api/README.md) — Public Java sagat-inline-v1 script and mediated host-call contract.
- [host](host/README.md) — Trusted Java compilation and runtime hosts exchanging newline-delimited JSON over standard I/O.

## Invariants

- `api`: The source identifies this entry point as the stable `sagat-inline-v1` contract. [runners/java/api/sagat/inline/InlineScript.java:InlineScript]
- `host`: Build requests use the exact `op/source/lockfile/packages` envelope, require `op=build`, and reject lockfiles and packages. Compilation fixes Java 17 and disables annotation processing. [runners/java/host/sagat/host/BuildHost.java:build]

## Side Effects

- `api`: Implementations mediate fetch, leased-secret access, and logging; these files define the interface without performing those operations themselves. [runners/java/api/sagat/inline/Octopus.java:Octopus]
- `host`: Creates `/tmp/sagat-build`, writes source and class files, launches fixed `javac`, and emits built/rejected messages. [runners/java/host/sagat/host/BuildHost.java:build]

## Verification

From the repository root, with Docker available for the container integration suite and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-infra-sandbox --locked
cargo test -p sagat-tests --test integration_inline_code_sandbox_containers --locked
```

The crate checks and container integration target are declared in `crates/infra-sandbox/Cargo.toml` and `tests/Cargo.toml`.
