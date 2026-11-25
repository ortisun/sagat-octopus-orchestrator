# Bundle version 5 and layer records (design 046)

Bundle versions 1 (text files, TypeScript and Python), 2 (C#) and 3 (Go and Java) exist in
`crates/infra-schema/src/bundle.rs`. Version 4 is planned by feature 023 (PHP and the reserved `python-data` runtime).
Version 5 is a TypeScript or Python version with packages: canonical JSON (RFC 8785), at most 2 MiB stored, with exactly
these keys:

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
    "runtime": "sha256:<the dependency runtime image digest>",
    "manifestDigest": "sha256:...",
    "lockDigest": "sha256:...",
    "unpacker": "sha256:<the layer-tool image digest>"
  }
}
```

- `language`, `policy`, `inputSchema`, `outputSchema`, `entry` and `files` keep the meaning and bounds of version 1.
- `ecosystem` is `npm` with `typescript`, or `pypi` with `python`.
- No layer, policy, report or snapshot is in the bundle: one source and one lock give one artifact, and per-version facts
  stay in `script_version_dependencies`.

The bundle's signature is unchanged (feature 005, Decision 98): Ed25519 over `sagat/script-artifact/v1 ‖ tenant ‖ digest ‖
language` with the script signing key.

## Layer records

One per (artifact digest, target), signed with the same key over
`sagat/script-layer/v1 ‖ tenant ‖ artifact digest ‖ target ‖ layer digest ‖ unpacker`, stored in `script_layer_records`
and served to runners with the artifact. A runner uses a layer only if its record verifies and names the digest its
verify container computed. A deployment that adds a target later gets records for existing versions from the backfill
pass; nothing is republished.

## The layer digest

The SHA-256 of a canonical serialization of the unpacked tree, computed by the layer tool's verify step on a read-only
mount: for each entry in byte order of its path, excluding every `__pycache__` directory and the marker, the path, a NUL
byte, the kind (`d` or `f`), a NUL byte, for a file its size as decimal, a NUL byte and its SHA-256 in lower-case
hexadecimal, and a newline. Modes and owners are not part of it (the seal step fixes the modes).

The marker `.sagat-layer`, written last by the seal step, holds `sagat/layer/v1`, the tenant, the digest and the unpacker,
one per line. A runner mounts a volume only if the marker names the recorded digest.
