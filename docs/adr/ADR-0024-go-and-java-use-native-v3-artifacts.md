# ADR-0024 — Go and Java use signed v3 artifacts and dedicated trusted hosts

Status: **Accepted; native ARM64 and AMD64 startup proofs verified** | Date:
2026-10-12 | Feature: 005

Supersedes only ADR-0013's refusal of Go and Java. The isolation, signing, tenant, lease,
host-call and execution boundaries of ADR-0013 remain in force. Artifact v1 for TypeScript/Python
and v2 for C# remain unchanged.

## Context

Go and Java need compiled artifacts, but their execution models differ. A Go executable is native to
one Linux architecture and cannot be started from a `noexec` scratch mount. Java bytecode is portable
across the runner architectures we use, but accepting a JAR or an author's project would expose more
names, metadata and build behavior than one inline class needs. Neither language runtime is a security
boundary; both continue to rely on the container profile established by ADR-0013.

The existing worker claim says only `language`, not native target. Advertising one Go language from a
pool that mixes ARM64 and AMD64 would therefore let a valid artifact reach an incompatible runner.
Emulation is also not evidence for a native execution guarantee: the local AMD64-under-ARM64 probe
failed inside nested QEMU while the equivalent native ARM64 path passed.

## Decision

1. **Bundle v3 is additive and exact.** Its top-level fields are exactly `v`, `language`, `target`,
   `abi`, `policy`, `inputSchema`, `outputSchema`, `entry`, `encoding` and `files`. The existing
   signature covers the digest of these canonical bytes, so target, ABI, policy, schemas, encoding,
   entry and every file are bound by the signature. Stored bytes remain capped at 2 MiB.
2. **Go is a native artifact.** The target is exactly `linux-arm64` or `linux-amd64`, the ABI is
   `sagat-inline-v1`, the entry is `script.elf`, the encoding is `base64+gzip`, and there is exactly
   one file. Base64 and gzip must be canonical. The compressed payload is at most 1,400 KiB and the
   expanded executable at most 8 MiB. The validator rejects concatenated members, trailing bytes,
   bad CRCs, truncation and expansion bombs. It accepts only ELF64 little-endian executable files for
   the signed machine, with bounded program/section tables, an entry in an executable load segment,
   no interpreter, no writable-executable load segment and a non-executable GNU stack.
3. **The Go builder owns the project.** It writes one fixed module and wrapper around the author's
   `package main` source, exposes only the trusted `sagat.invalid/inline/octopus` package, and runs one
   explicit `go build`. `CGO_ENABLED=0`, `GOTOOLCHAIN=local`, `GOPROXY=off`, `GOSUMDB=off`,
   `GOWORK=off` and the native `GOARCH` are fixed. It never runs an author's `go.mod`, generator,
   test, install or dependency command. Any non-empty lockfile or package request is refused. The
   image contains a read-only standard-library build cache so the build fits the bounded scratch
   mount; per-build cache additions stay in `/tmp`.
4. **Go executes from sealed anonymous memory.** A trusted Python loader decodes and validates the
   bounded gzip again, creates an executable `memfd`, writes the exact signed bytes and applies
   `F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL`. It creates and seals the input JSON
   the same way and exposes it as inherited fd 3. Standard input remains reserved for supervised host
   replies and standard output for protocol messages. The loader calls `os.execve` with the executable
   fd; `/tmp` stays `noexec,nosuid,nodev`. The executable fd deliberately omits `MFD_CLOEXEC` because
   the verified kernel must retain the final fd while entering it. It contains only immutable signed
   script bytes. If the executable initially occupies fd 3, the loader moves it above fd 9 before
   installing the input and closes every unrelated loader fd.
5. **Go capability is native and fail closed.** A runner has one configured Go target and rejects a
   bundle whose signed target differs. Before advertising Go, startup compiles a bundled trusted probe
   and executes it through the same production images, protocol, loader and container profile. A
   failed optional probe omits Go; a deployment configured to require Go fails readiness/startup.
   Until claims include a target, every Go-eligible deployment or pool is homogeneous for that one
   target. ARM64 is enabled by its native proof; AMD64 remains disabled until the same proof passes on
   native AMD64 CI. Building both Docker platforms does not establish either execution capability.
6. **Java is constrained class-file bytecode.** The target is `jvm-17`, ABI `sagat-inline-v1`, entry
   `Script.class` and encoding `base64`. A bundle contains only `Script.class` and at most 31 nested
   `Script$*.class` files, with a 1,400 KiB aggregate limit. Each class must have the Java 17 major
   version 61, no preview minor version, and an internal binary name matching its file. JARs and
   unrelated classes are refused.
7. **The Java builder and runtime are trusted hosts.** The builder invokes `javac --release 17
   -proc:none` with explicit fixed arguments and an empty inherited environment. It never invokes
   Maven, Gradle, an annotation processor or author-controlled compiler options. A script implements
   the trusted `InlineScript` API and exchanges JSON values as maps, lists and primitives. The runtime
   uses a parent-first in-memory class loader, the JVM verifier and a pinned Jackson 2.17.2 stack with
   no polymorphic/default typing. It loads no author JAR. Any non-empty lockfile or package request is
   refused.
8. **The existing supervisor semantics apply to both languages.** Fetch, secret, log and idempotency
   calls cross the existing bounded JSON-line channel. Network remains absent from the script
   container, secrets remain capability-scoped and redacted, and the existing fenced lease,
   signature, audit and at-least-once external-effect rules do not change. The build key, SBOM and
   telemetry bind the compiler, ABI, target, compression/JSON implementation and exact build/runtime
   image identities.

## Verification and rollout gates

The native ARM64 production-path probe compiled a 1,835,156-byte static executable into a canonical
778,628-byte gzip artifact, validated the signed v3 bundle, then executed it from the four-seal
memfd. Both stages ran as uid 65534 with network
disabled, a read-only root, all capabilities dropped, `no-new-privileges`, the default seccomp
profile and a `noexec,nosuid,nodev` scratch mount; runtime was limited to 256 MiB, one CPU and 64
processes. The builder image is based on pinned Go 1.25.3
`sha256:4f43b271f9673eb7bd0cb3a49cc17b08d8d6ee110277e26dbacc93c43a5a7793`; the loader image is
pinned Python 3.13 Alpine
`sha256:2d9aefe2fef018a7eb2c13064c89c71929800fd2e5dccdbf52ea5da5bb8d929a`.

The native ARM64 Java `DockerSandbox` proof compiled with the same isolation profile and executed a
Java 17 class with nested map/list/primitive input, supervised fetch and secret calls, bounded log,
and the stable idempotency key. Its runtime used 256 MiB, one CPU and 64 processes. The exact JDK/JRE
images and Jackson archive hashes are pinned in the two Java Dockerfiles.

Malformed v3, gzip, ELF and class-file tests and v1/v2 regression tests are mandatory. Native AMD64
Go compile-and-execute CI, real supervisor host-call tests for both languages and startup capability
tests remain rollout gates; they cannot be replaced with an emulated result or a relaxed seccomp or
executable scratch mount.

## Consequences and alternatives

Go images are larger because they carry a trusted standard-library cache, and each architecture
needs its own native proof and homogeneous pool. This cost keeps customer builds offline and bounded.
Adding target-aware claims later can safely permit mixed pools, but language-only claims cannot.

Executing a copied file from `/tmp` was rejected because it requires weakening the established
`noexec` profile. Relaxing seccomp for emulation was rejected because it changes production isolation
to accommodate a non-native test. JARs and author projects were rejected because they add manifest,
classpath, plugin and build-tool surfaces with no inline-script benefit. An ad-hoc Java JSON parser
was rejected in favor of a pinned, reviewed library.

## Sources

- [Python `os.memfd_create` and descriptor flags](https://docs.python.org/3/library/os.html#os.memfd_create)
- [Python `os.execve` with an open file descriptor](https://docs.python.org/3/library/os.html#os.execve)
- [Python `fcntl` sealing constants](https://docs.python.org/3/library/fcntl.html)
- [Oracle Java 17 `javac` options](https://docs.oracle.com/en/java/javase/17/docs/specs/man/javac.html)
- [Java 17 class-file format](https://docs.oracle.com/javase/specs/jvms/se17/html/jvms-4.html)
- [Go command build cache](https://pkg.go.dev/cmd/go#hdr-Build_and_test_caching)
- ADR-0013 and `specs/005-inline-code-sandbox/spec.md`.

Native AMD64 startup and six-sample Go/Java production execution passed on commit `4aeb618` in [CI job 114653433277](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38200759888/job/114653433277), alongside native ARM64 job 114653433252. The CI job also runs the full native host-call and resource suite on subsequent revisions.

Full native host-call and resource validation subsequently passed on `64f1532`: all five ignored tests in [AMD64 job 114655611633](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38201472580/job/114655611633) and [ARM64 job 114655611559](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38201472580/job/114655611559), including CPU termination, network/host-call isolation and cleanup. AMD64 completed in 37.73 seconds.
