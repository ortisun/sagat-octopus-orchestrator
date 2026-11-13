# Go inline runner

The builder compiles one `package main` source that defines:

```go
func Run(input any, client *octopus.Client) (any, error)
```

The only import outside the Go standard library is the trusted
`sagat.invalid/inline/octopus` API. Builds have no lockfile, dependency download, cgo, workspace,
generator, test or project command. A read-only standard-library cache is baked with the same build
flags; the external cache helper keeps any source-specific additions in the bounded `/tmp` mount.

The signed v3 artifact is one stripped static ELF for exactly `linux-arm64` or `linux-amd64`,
canonically gzip-compressed. The runtime validates its target and compression again, seals the
executable and input in anonymous memory, installs input as fd 3, and enters the executable by fd
while `/tmp` remains `noexec`. The executable fd has all four write/grow/shrink/seal seals and is
kept across `execve`; it contains only the signed artifact bytes.

A deployment configures exactly one Go target and advertises Go only after `DockerSandbox::probe_go`
compiles and executes the bundled probe through these production images. ARM64 and AMD64 require
separate native evidence. Until worker claims carry a target, Go-enabled pools must be homogeneous.

## Interface

- `Response`, `Client`, `Channel`, `New`, `Fetch`, `Secret`, `Log`, `Result`, and `Fail` provide the public mediated host API. [runners/go/octopus.go:Client] [runners/go/octopus.go:Response] [runners/go/octopus.go:Channel] [runners/go/octopus.go:New]
- The builder and cache helper are separate command entry points; the Python runtime loads their compiled artifact. [runners/go/buildhost.go:main] [runners/go/cacheprog.go:main] [runners/go/runtime.py:main]

## Invariants

- Limits request input to 2 MiB, requires nonempty `op=build` source, rejects lockfiles/packages, injects fixed module/wrapper/API files, and compiles offline with read-only modules. [runners/go/buildhost.go:build]
- Build output is deterministic gzip with zero timestamp and OS 3, carrying base64 `script.elf`. [runners/go/buildhost.go:canonicalGzip]
- Cache operations are limited to get/put/close; digests are 32 bytes and put bodies are capped at 24 MiB. [runners/go/cacheprog.go:main]
- The runtime accepts only `script.elf` with native target and `base64+gzip`; compressed size is bounded to 1,400 KiB and expansion to 8 MiB, with canonical gzip required. [runners/go/runtime.py:decode_artifact]
- Calls serialize under a mutex, use increasing IDs, ignore malformed/unmatched replies, and fail refused or closed-channel responses. [runners/go/octopus.go:Client.call]

## Side Effects

- Writes the fixed project under `/tmp/sagat-build`, launches `go build`, reads `/tmp/script.elf`, and emits build protocol messages. [runners/go/buildhost.go:build]
- Reads a seed cache, writes new bodies beneath `/tmp/sagat-go-cache`, and exchanges GOCACHEPROG JSON. [runners/go/cacheprog.go:main]
- Creates sealed memory files, installs JSON input on descriptor 3, and execs the submitted binary with only PATH and the idempotency key. [runners/go/runtime.py:main]
- Preserves the protocol channel, redirects ordinary stdout to stderr, emits mediated operations/results/errors, and logs to stderr. [runners/go/octopus.go:Channel]

## Verification

From the repository root, with Docker available for the container integration suite and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-infra-sandbox --locked
cargo test -p sagat-tests --test integration_inline_code_sandbox_containers --locked
```

The crate checks and container integration target are declared in `crates/infra-sandbox/Cargo.toml` and `tests/Cargo.toml`.
