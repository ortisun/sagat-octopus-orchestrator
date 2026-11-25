# Bundle version 5 (design 046)

Bundle versions 1 (text files, TypeScript and Python), 2 (C#) and 3 (Go and Java) exist in
`crates/infra-schema/src/bundle.rs`. Version 4 is planned by feature 023 (PHP and the reserved `python-data` runtime).
Version 5 is a TypeScript or Python version with packages. It is canonical JSON (RFC 8785), at most 2 MiB stored, with
exactly these keys:

```json
{
  "v": 5,
  "language": "python",
  "policy": "v1;memory=...",
  "inputSchema": null,
  "outputSchema": null,
  "entry": "main.py",
  "files": { "main.py": "..." },
  "dependencies": {
    "ecosystem": "pypi",
    "runtime": "python-3.13-glibc",
    "manifestDigest": "sha256:...",
    "lockDigest": "sha256:...",
    "unpacker": "sha256:...",
    "layers": {
      "linux-amd64": { "digest": "sha256:...", "size": 123456789, "files": 4021 }
    }
  }
}
```

- `language`, `policy`, `inputSchema`, `outputSchema`, `entry` and `files` have the meaning and bounds of version 1.
- `ecosystem` is `npm` (with `language: "typescript"`) or `pypi` (with `language: "python"`).
- `runtime` names a dependency runtime line (`node-24-glibc`, `python-3.13-glibc`); the runner refuses a line it does
  not have.
- `layers` holds exactly the deployment's required targets at publication.
- The policy digest, the policy report and the advisory snapshot are not in the bundle: they are per-build facts and
  live in the build's provenance (feature 023's `script_build_provenance`, ADR-0057 decision 6), so that one lock and one
  source give one artifact digest.

The signature is unchanged (feature 005, Decision 98): Ed25519 over `sagat/script-artifact/v1 ‖ tenant ‖ digest ‖
language` with the control plane's script signing key.

## The layer digest

The layer digest is the SHA-256 of the canonical serialization of the unpacked tree, computed by the layer tool: for
each entry in byte order of its path, the path, a NUL byte, the kind (`d` or `f`), a NUL byte, for a file its size as
decimal and a NUL byte and its SHA-256 in lower-case hexadecimal, and a newline. Modes are fixed (files 0444,
directories 0555), owners 0, times 0. Bytecode files written after unpacking (Python) are not part of the digest; they
live under `__pycache__` directories, which the unpacker refuses to receive from a wheel.

The layer volume holds, at its root, the file `.sagat-layer` written last, containing `sagat/layer/v1`, the tenant id,
the digest and the unpacker identity, one per line. A runner mounts a volume only if this marker names the signed
digest.
