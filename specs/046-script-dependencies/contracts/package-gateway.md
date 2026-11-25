# The package gateway (design 046)

`apps/package-gateway`. A Rust service with no database. Two listeners:

- **Package listener** (`8095` in the compose, published on `127.0.0.1:${SAGAT_PACKAGE_GATEWAY_PORT:-18095}`): the
  routes below for runners, with job tokens.
- **Internal listener** (`8096`, not published): `POST /internal/v1/metadata`, the store administration routes, and
  `GET /healthz`, for the control plane's service token only.

## Tokens

EdDSA JSON Web Tokens verified with `jsonwebtoken`. The control plane signs them with the package token key
(`SAGAT_PACKAGE_TOKEN_KEY`), which is not the script signing key; the gateway holds the public keys
(`SAGAT_PACKAGE_GATEWAY_TRUSTED_KEYS`, several for rotation).

| Token | Audience | Claims | Lifetime |
|---|---|---|---|
| Job token | `package-gateway` | `tenant`, `namespace`, `job` (a resolve or install job id, or a materialization id), `ecosystem`, `policy` (digest), `kind` (`resolve`, `install`, `materialize`) | 15 minutes; refused after its job is finished (the gateway asks the control plane once per token and caches the answer for 60 s) |
| Service token | `package-gateway-internal` | `sub` = `control-plane` | 5 minutes |

A token never appears in a URL, a log line or a container: the runner adds it to the requests it forwards.

## Package routes

| Route | Meaning |
|---|---|
| `GET /npm/{name}` | The npm version document, abbreviated form (`application/vnd.npm.install-v1+json`) or full, filtered by the token's policy (Technical Plan 3 of the design) |
| `GET /npm/{name}/-/{file}` | A tarball, served from the store |
| `GET /pypi/simple/{project}/` | The project page of the simple API in its JSON form (PEP 691), filtered |
| `GET /pypi/files/{sha256}/{filename}` | A wheel, from the store |
| `GET /pypi/files/{sha256}/{filename}.metadata` | The wheel's core metadata (PEP 658 and PEP 714), so that a resolver need not download the wheel |
| `GET /files/sha256/{hex}` | Any stored file by hash (install and materialization), for the token's tenant: a public file, or a private file of that tenant; any other is `404` |

Names are validated and normalised (`lock-formats.md`, Names) before any upstream path is built. A private file of
another tenant, a name that is private to another tenant and a file that does not exist all answer the same `404`.

## Upstreams

Configured by the operator: `SAGAT_PACKAGE_GATEWAY_NPM_UPSTREAM` (default `https://registry.npmjs.org`),
`SAGAT_PACKAGE_GATEWAY_PYPI_UPSTREAM` (default `https://pypi.org`), and the file hosts the gateway may fetch from,
`SAGAT_PACKAGE_GATEWAY_FILE_HOSTS` (default `registry.npmjs.org,files.pythonhosted.org`). The gateway fetches only https
URLs whose host is an upstream or a file host, through the guarded egress client of `crates/infra-egress` (a streaming
send added by Task T046-6: addresses checked once, private ranges refused unless the operator allows an upstream's,
redirects not followed). A file is checked against the registry's declared hash before it is stored, and stored once
under its SHA-256, never replaced. `SAGAT_PACKAGE_GATEWAY_MODE=offline` disables every upstream.

## Metadata

`POST /internal/v1/metadata` takes up to 2,000 entries of (ecosystem, name, version, file hash) and returns for each:
`uploadedAt` (the registry's time), `licence` (the declared expression or text, at most 1 KiB), `installScripts` (the
lifecycle script names declared, npm), `yanked` (Python) or `deprecated` (npm), `declaredDependencies`
(`Requires-Dist` or the version document's dependency maps), `stored` (true or false), `private` (true for an upload).

## Store

Files live in the package store, a blob store separate from the tenant payload store of ADR-0023: its own bucket or
directory and credential (`SAGAT_PACKAGE_STORE_*`, the settings of `crates/infra-blobstore`), which only the gateway
holds. Keys: `public/sha256/<hex>` and `private/<tenant>/sha256/<hex>`. Administration routes on the internal listener:
`GET /internal/v1/store/objects?olderThan=...` (paged) and `DELETE /internal/v1/store/objects/{key}`, used by the
control plane's collection pass.

## Offline bundles

`package-gateway export --lock <file>... --out <bundle>` writes a tar of files named by hash and a manifest of their
metadata, signed with the operator's export key (Ed25519). `package-gateway import <bundle> --trusted-key <file>`
refuses a bundle whose signature or any hash does not verify.
