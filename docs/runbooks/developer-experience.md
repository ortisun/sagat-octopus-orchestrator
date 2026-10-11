# Runbook: the developer experience (feature 012)

For the people who write workflows and workers, and for the people who keep the local stack and the documentation
honest. Design: [ADR-0019](../adr/ADR-0019-the-first-day-runs-on-the-real-engine-with-canned-answers-and-a-development-identity-provider.md);
decisions and gaps: [`specs/012-developer-experience/research.md`](../../specs/012-developer-experience/research.md).
The path itself is [the quickstart](../quickstarts/README.md).

## 1. What there is

| Part | What it is | Where |
|---|---|---|
| `sagat` | The command line: `new`, `workflow validate\|publish\|run\|runs\|inspect\|cancel`, `script test\|publish`, `worker dev\|bind`, `dev token`, `doctor`, and the manifest commands of [feature 011](gitops.md). It talks only to the REST API (and the worker API for `worker dev`) and checks documents with the platform's own codec. | `apps/cli` |
| Templates | Six workflows with the answers to give them and a README: `hello-world`, `event-trigger`, `cron`, `approval`, `compensation`, `inline-script`. `sagat new --list`. | `templates/` |
| `worker dev` | A real worker that answers from a file (`output`, `echo`, `fail`, `lose`, `sequence`, `delayMs`; compensations succeed and reconcilers say `unknown` unless the file says otherwise). | `apps/cli/src/worker.rs`, `mocks.rs` |
| Examples | The hello-world in five SDKs; a payment saga in TypeScript. | `examples/` |
| The local stack | Compose: PostgreSQL, NATS, the control plane (API and Studio), the engine, the development identity provider. | `infra/docker/compose.yaml` |
| `dev-idp` | The development identity provider (gap G10). | `apps/dev-idp` |
| The reference | Generated from the contracts. | `docs/reference/` |

There is deliberately **no second engine** for local use. A separate emulator would have its own idea of a retry, a
compensation and a reconciliation, and a workflow that passed on it could fail on the platform. `worker dev` replaces
only the worker: what the platform does around it is the platform's own.

## 2. The development identity provider

The platform verifies OIDC tokens and the repository had no provider, so a developer without one could not use the
stack. `dev-idp` signs an ES256 token for **any subject of any tenant it is asked for**. That is the whole point of it
and the whole danger, so:

- It refuses to start unless `SAGAT_ENV=dev` (exactly that).
- It listens on loopback unless `SAGAT_DEV_IDP_EXPOSED=true` says the address is reachable only by the stack
  (the container sets it; compose publishes the port on the host's `127.0.0.1` only).
- It answers only to the host names it is configured with (its issuer's, the machine's own, and
  `SAGAT_DEV_IDP_HOSTS`): a web page that makes a browser resolve another name to this address gets `403` (DNS rebinding).
- `POST /token` takes `application/json` and nothing else (a form or a text body is what a page of another origin
  can send without asking first), at most 4 KiB, for subjects and tenants the platform itself accepts (not the
  reserved `system:` actors) and a life of at most an hour, which is the most the platform takes. It sends no CORS
  headers.
- The key is in `/data/key` (a volume, mode `600`; it refuses a key file others can read) so that a token outlives a
  restart. Without `SAGAT_DEV_IDP_KEY_FILE` the key lives as long as the process.
- It is a stage of its own in the Dockerfile and in no other image. A deployment that is not a laptop uses its own
  provider (set the three `SAGAT_OIDC_*` variables and `SAGAT_ALLOW_INSECURE_JWKS_HTTP=false`) and does not start it.

The token alone does not make someone a member: the platform lets in only the members of a tenant (the seed makes
`dev-admin` of `dev`). A token for `mallory` is a valid signature and still `401`.

`sagat dev token [--subject S] [--tenant T] [--ttl N] [--out FILE]` asks it. With no `--out` the token alone is on the
output (so that `export SAGAT_TOKEN=$(sagat dev token)` works) and a line about it on the error stream; with `--out` it
is written to a file only the owner reads, for `--token-file`. The address is `--idp`, `SAGAT_DEV_IDP`, or
`http://127.0.0.1:18090`; plain http only for a loopback address.

## 3. What `sagat` does with what it is given

- **The token** comes from `--token-file` or `SAGAT_TOKEN`, never from an argument (it would be in the process list
  and the shell history). It is sent only to the address given, which must be https, or http on a loopback address; no
  proxy is used for http; no redirect is followed. It is never printed.
- **What it prints** that somebody else wrote (a reason, a name, a message from the platform) goes through a filter that
  shows control characters and the characters that reorder or hide text as `?`.
- **Documents** are checked with the platform's codec (`sagat-infra-ir`), so a document that validates is a document the
  platform accepts, and an error carries a code that [errors.md](../quickstarts/errors.md) explains. Parse errors do
  not quote the file.
- **`worker bind`** (and `worker dev --bind`) puts the identity of the token in the worker bindings of the
  namespace for the activities named, **replacing** what it was bound to there. It says so. An administrator's token
  can serve as a worker in the dev stack; in production a worker has an identity of its own.
- **Mock files** are bounded (200 names, 10 s of delay) and parsed with a bounded YAML expansion. A mock that
  `lose`s a task holds the lease until it expires (30 seconds by default, `SAGAT_LEASE_TTL_SECS` on the engine).

## 4. Running it

```sh
cp infra/docker/.env.example infra/docker/.env
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env --profile seed run --rm seed
```

| Port | Default | Variable |
|---|---|---|
| REST API and Studio | 18080 | `SAGAT_API_PORT` |
| Worker API (gRPC) | 19000 | `SAGAT_GRPC_PORT` |
| Development identity provider | 18090 | `SAGAT_IDP_PORT` |
| PostgreSQL | 15432 | `SAGAT_PG_PORT` |
| NATS, its monitor | 14222, 18222 | `SAGAT_NATS_PORT`, `SAGAT_NATS_MONITOR_PORT` |

`seed` makes the tenant `dev`, its administrator `dev-admin`, the workspace `dev` and the namespace `payments`. Each of
`SAGAT_SEED_TENANT`, `SAGAT_SEED_ADMIN_SUBJECT`, `SAGAT_SEED_WORKSPACE` and `SAGAT_SEED_NAMESPACE` changes one, and an
empty `SAGAT_SEED_NAMESPACE` leaves none. The Studio is served from the image's own files unless `SAGAT_STUDIO_DIR` is set
to something else (or to nothing).

`down` keeps the data; `down -v` forgets it, and the provider's key with it (tokens made before it stop working).

## 5. When `sagat doctor` says something is wrong

| Check | What it looked at | What to do |
|---|---|---|
| `address` | `--server` / `SAGAT_SERVER`, shaped `http://127.0.0.1:18080/v1` | Include `/v1`; https unless it is a loopback address. |
| `token` | `--token-file` / `SAGAT_TOKEN`, and when it ends | `export SAGAT_TOKEN=$(sagat dev token)` again; a token lives an hour. |
| `reachable`, `ready` | `/health`, `/ready` | `docker compose ps`; the control plane waits for the migration. |
| `credentials` | `/auth/context`: who the platform thinks you are | A token the provider signed for someone who is not a member is `401`: seed, or sign in as `dev-admin` of `dev`. |
| `versions` | `GET /v1/version`: the platform's build, API and worker protocol against the tool's | Build the tool from the same checkout. |
| `namespace` | the namespace exists in the tenant | `--namespace`, or run the seed. |
| `worker` | the gRPC endpoint accepts a connection | `SAGAT_GRPC` / `--grpc`; `docker compose ps execution-engine`. |
| `studio` | `/studio/` | Skipped, not a failure, when it is not served; the API works either way. |
| `--nats` | a broker answering the NATS greeting | The platform works without it (it polls); events and task hints are slower. |

## 6. Keeping the documentation honest

- `docs/reference/api.md`, `workflow-document.md` and `cli.md` are generated from the OpenAPI document, the workflow
  schema and the tool's own help. A test fails when one is not what a regeneration gives:
  `SAGAT_DOCS_BLESS=1 cargo test -p sagat-tests --test contract_developer_experience` rewrites them; read the diff.
- The same target runs the examples of the documents it can: each `sagat ...` line in a shell block is parsed by the
  tool's own parser; a code block marked ```` ```go excerpt=examples/hello-world/go/main.go ```` must be in that file as shown;
  the scripts a README runs exist and are executable; every relative link leads to a file.
- `docs/quickstarts/errors.md` is checked against the codec in both directions (see its head).

## 7. Checking it

| What | How |
|---|---|
| The tool, the templates, the mocks, the errors | `cargo test -p sagat-cli` |
| The development identity provider | `cargo test -p sagat-dev-idp` |
| The first day, the stories, the diagnostics, `doctor`, the seed and the provider against the platform | `cargo test -p sagat-tests --test e2e_developer_experience` (PostgreSQL) |
| The documentation | `cargo test -p sagat-tests --test contract_developer_experience` |
| The examples in five languages and the saga, against a real platform | `cargo test -p sagat-tests --test e2e_developer_experience -- --ignored` (Node, Python, Go, .NET and a JDK; `SAGAT_EXAMPLE_LANGS=go,python` picks some) |
| The compose stack, the provider and the first day on it | `SAGAT_BIN=target/debug/sagat scripts/smoke-compose.sh` (Docker; the `local-environment` job of CI) |

## 8. What is not built

See the gaps in [`specs/012-developer-experience/research.md`](../../specs/012-developer-experience/research.md). In short: a
person's first day has not been timed (`SC-012-001`); there is no prebuilt `sagat`, no published SDK and no hosted
documentation; the SDKs ship no fake context for unit testing an activity; the compose stack is verified by CI and
by running its pieces as processes, not by this author's machine (its container engine was not usable); the dev
identity provider has no browser sign-in flow (the Studio takes a pasted token).
