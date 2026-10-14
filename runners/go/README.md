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
