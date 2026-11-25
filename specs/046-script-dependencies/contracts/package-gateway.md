# The package gateway (design 046)

`apps/package-gateway`. A Rust service with no database and no tenant data: it holds only public files. Two listeners:

- **Package listener** (`8095` in the compose, published on `127.0.0.1:${SAGAT_PACKAGE_GATEWAY_PORT:-18095}`): the
  public routes below, for requests runners forward with a job grant.
- **Internal listener** (`8096`, not published): metadata, advisory archives and store administration, for the control
  plane's service token only.

The image's healthcheck is the subcommand `package-gateway healthcheck`, as the other images of
`infra/docker/Dockerfile` have.

## Tokens

EdDSA JSON Web Tokens verified with `jsonwebtoken`, signed by the control plane with the package token key
(`SAGAT_PACKAGE_TOKEN_KEY`), which is not the script signing key; the gateway holds the public keys
(`SAGAT_PACKAGE_GATEWAY_TRUSTED_KEYS`, several for rotation). The gateway never calls the control plane.

| Token | Audience | Claims | Lifetime |
|---|---|---|---|
| Job grant | `package-gateway` | `tenant`, `job` (a dependency job, a test run or a materialization), `attempt`, `kind` (`resolve`, `install`, `materialize`, `test-run`), `ecosystem`, `cut` (the quarantine cut, RFC 3339, from the control plane's clock), `exp` | until the job attempt's deadline, at most 10 minutes |
| Service token | `package-gateway-internal` | `sub` = `control-plane` | 5 minutes |

A token never appears in a URL, a log line or a container: the runner adds it to the requests it forwards.

## Package routes (public content only)

| Route | Meaning |
|---|---|
| `GET /npm/{name}` | the npm version document, abbreviated form (`application/vnd.npm.install-v1+json`), with every version published after `cut` left out |
| `GET /npm/{name}/-/{file}` | a tarball |
| `GET /pypi/simple/{project}/` | the project page of the simple API in its JSON form (PEP 691), with versions after `cut` and every source distribution left out |
| `GET /pypi/files/{sha256}/{filename}` | a wheel |
| `GET /pypi/files/{sha256}/{filename}.metadata` | the wheel's core metadata (PEP 658, PEP 714) |
| `GET /files/sha256/{hex}` | a stored public file by hash |

Paths follow a closed grammar: names validated (npm name rules; PEP 508 names) and normalised before any upstream path
is built; no percent-encoded separator, no `..`, no `//`, no query. Yanked versions are marked as the registry marks
them. Allow and deny rules, waivers, advisories and reserved names are not applied here: reserved names never reach the
gateway (the runner sends them to the control plane's private routes).

## Internal routes

| Route | Meaning |
|---|---|
| `POST /internal/v1/metadata` | for up to 2,000 entries (ecosystem, name, version, file hash): `uploadedAt`, `licence` (at most 1 KiB), `installScripts` (npm), `yanked` or `deprecated`, `declaredDependencies`, `stored`, `size` |
| `GET /internal/v1/advisories/{ecosystem}` | the OSV archive of the ecosystem, fetched from `SAGAT_PACKAGE_GATEWAY_ADVISORY_SOURCE` and cached, streamed |
| `GET /internal/v1/store/objects?olderThan=…&after=…` | page through stored objects |
| `DELETE /internal/v1/store/objects/sha256/{hex}` | delete one object (the collection pass only) |

## Upstreams

`SAGAT_PACKAGE_GATEWAY_NPM_UPSTREAM` (default `https://registry.npmjs.org`), `SAGAT_PACKAGE_GATEWAY_PYPI_UPSTREAM` (default
`https://pypi.org`), `SAGAT_PACKAGE_GATEWAY_FILE_HOSTS` (default `registry.npmjs.org,files.pythonhosted.org`),
`SAGAT_PACKAGE_GATEWAY_ADVISORY_SOURCE` (default the public OSV bucket). Fetches are GET-only, https only, to those hosts
only, through the streaming send of `crates/infra-egress` (addresses checked once, private ranges refused unless the
operator allows an upstream's, redirects not followed). A file is checked against the registry's declared hash before
it is stored once under its SHA-256 in the public store (`SAGAT_PACKAGE_STORE_*`). `SAGAT_PACKAGE_GATEWAY_MODE=offline`
disables every upstream.

## Offline bundles

`package-gateway export --lock <file>... --out <bundle>` writes a tar of files named by hash and their metadata, signed
with the operator's export key (Ed25519). `package-gateway import <bundle> --trusted-key <file>` refuses a bundle whose
signature or any hash does not verify, and reports the hashes it imported so the operator can pin them
(`control-plane packages pin --bundle <id>`).
