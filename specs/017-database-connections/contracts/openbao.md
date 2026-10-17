# OpenBao provider boundary

Implemented port: `sagat_application::secret_provider::SecretProvider`.
Implementation: `sagat_infra_secrets::openbao::OpenBao`. Backend-only boundary. Management routes are enabled when the control plane has OpenBao settings and an intent binding key.

A provider reference consists of tenant UUID, namespace UUID and provisioning-operation UUID, all allocated/read from trusted Sagat persistence. The configured mount is a single alphanumeric/underscore/hyphen segment. Paths are `/v1/{mount}/data/sagat/credentials/{tenant}/{namespace}/{operation}`. Each operation has one immutable KV object, created with `options.cas = 0` and `data.value` containing base64 of 1–16384 secret bytes. Reads explicitly request provider version 1 and validate the returned version/deletion metadata. Logical rotation creates another operation object; it never overwrites an existing one.

`create` returns success only with a valid version-1 receipt. On a refused CAS, it reads the object: existence returns `Conflict`, never success. The application must verify the durable content binding before activation. `read` returns missing only for provider 404. `delete` permanently destroys version-1 value bytes and retains metadata as a fence against late CAS=0 creation. It first reads bounded metadata; an absent path gets a noncredential CAS=0 tombstone before version 1 is destroyed. Metadata must prove current_version=1. Retirement uses only metadata reads, data creation and version destruction; no credential read or metadata-delete grant is required. Tombstone metadata accumulates and must be preserved during restore. This operation is only for authorized retention/abandoned-intent cleanup.

Production uses verified HTTPS, optional configured CA, and an absolute token-file path maintained by OpenBao Agent. Each request rereads the bounded token file, validates lookup-self has a positive TTL no greater than 24 hours and no root policy, and sends a sensitive `X-Vault-Token` header. Agent authentication/renewal is external to this adapter. The full operation, including lookup and any receipt read, has one 1–10 second deadline. Redirects and environment proxies are disabled. Responses are bounded to 64 KiB before parsing. Only an explicitly selected development mode permits loopback HTTP/root tokens.

Errors are a closed enum: configuration, unavailable, denied, conflict, rejected, invalid response. They carry no provider response text, secret, token or URL. Buffers owned by the adapter for plaintext, base64 JSON and token-file contents are zeroized; HTTP/TLS libraries also hold transient copies, so this is not a claim of eliminating every process-memory copy. No cross-request plaintext cache exists.

Metrics: `secret_provider_requests_total{provider,operation,outcome}` and `secret_provider_request_seconds{provider,operation}`, all closed labels. Read/create tracing spans skip every argument. Application-layer audits and tenant authorization are required before exposing this port to users.

## Operator retirement

`control-plane cleanup-secrets` processes one batch (`SAGAT_SECRET_CLEANUP_BATCH`, default 32,
range 1..128) with a 60-second command deadline. It uses the runtime PostgreSQL role, checked
outside development, and a separate token file with the cleanup policy below. It needs neither
the local encryption key ring nor the intent-binding key. A deferred/provider-failed batch exits 1
and can be scheduled again; invalid configuration exits 2. A killed job leaves a recoverable
90-second claim. Run it periodically; one invocation does not promise to drain the backlog.

```hcl
path "auth/token/lookup-self" { capabilities = ["read"] }
path "secret/metadata/sagat/credentials/*" { capabilities = ["read"] }
path "secret/data/sagat/credentials/*" { capabilities = ["create"] }
path "secret/destroy/sagat/credentials/*" { capabilities = ["update"] }
```

Replace `secret` with the configured mount. This policy cannot read credential values or delete
metadata. Production tokens are non-root, finite-lived (at most 24 hours), supplied/renewed by
the workload's authentication agent. Management/engine tokens must not receive destruction rights.

Cleanup preserves current plus four prior logical versions, all live/manual-review attempt pins,
and terminal attempt pins for 24 hours. Unreferenced operations become eligible 24 hours after
creation/activation. Abandonment/audit/claim commit before provider retirement; receipt removal,
retained-byte refund and retirement audit commit only after confirmed retirement under the same
unexpired claim. No database transaction waits on provider I/O. Provider metadata is deliberately
retained forever to reject delayed CAS=0 writes; account for its storage growth and preserve it on
restore. Restoring PostgreSQL and OpenBao from unrelated points can leave missing credentials and
must fail closed until reconciled.

Observability: `secret.cleanup` span; `secret_cleanup_seconds`,
`secret_cleanup_command_seconds{outcome=success|retry}`, `secret_cleanup_pins_released_total`,
`secret_cleanup_operations_total{outcome=retired|deferred|superseded}`. Audits are
`secret.provider_abandoned`, `secret.provider_retired`, `secret.attempt_pins_released`; logs carry
counts, never credential/provider paths or tokens.

Sources: [OpenBao HTTP API](https://openbao.org/api-docs/), [KV v2](https://openbao.org/docs/secrets/kv/kv-v2/), [official releases](https://github.com/openbao/openbao/releases). Real-service fixture: pinned `openbao/openbao:2.7.1`; production digest/profile is a separate delivery task.

## Deployment settings and management facade

Both control plane and execution engine accept `SAGAT_OPENBAO_ADDRESS` (verified HTTPS),
`SAGAT_OPENBAO_TOKEN_FILE` (absolute path), `SAGAT_OPENBAO_MOUNT` (default `secret`),
`SAGAT_OPENBAO_CA_FILE` (optional absolute PEM path), and `SAGAT_OPENBAO_TIMEOUT_MS`
(default 5000, maximum 10000). Partial configuration fails startup. The control plane additionally
requires `SAGAT_OPENBAO_INTENT_KEY_FILE`: an absolute path containing exactly 32 random raw bytes.
This key must be shared by control-plane replicas, retained across restarts and backed up with
provider-operation metadata. Changing it while receipts remain makes their verification fail;
a key-version/rotation procedure remains a release task. The engine does not need this key.

`SAGAT_OPENBAO_DEVELOPMENT=true` is accepted only with `SAGAT_ENV=dev` and a numeric loopback
provider address. It is for local tests, not a container-network production shortcut. HTTP and
gRPC secret transport still require the existing protected-transport configuration.

With managed storage enabled, PUT secret requires `{value, requestId, expectedVersion}`;
`expectedVersion=0` creates and a positive version rotates. Database forms must also send `requiredProvider: openbao`; an instance without that provider returns 503 before writing any local value. Same actor/key/content recovers the
original receipt. POST `secret-operations/{requestId}/reconcile` needs no secret body; absent
provider evidence requires secure resubmission. POST `secrets/{name}/migrate` takes
`{requestId, expectedVersion}` and requires the old key ring. Management responses return
`provider: openbao` and `keyId: null`, never values or provider paths. Existing local metadata
continues to report `provider: local` and its key ID. See canonical OpenAPI for status codes.

The integration form retains only the request UUID across a failed submission; the credential
stays in its existing transient form state. Editing credential fields allocates a new request.
Successful secret creation clears these fields and retains the logical reference if connector
creation fails. Closing the form discards both form content and the pending request UUID.

## Existing consumers

Ordinary worker secret resolution takes a scoped metadata snapshot, commits, resolves values
outside the run transaction, then reopens the run and checks the live lease/generation, current
binding, cancellation and publisher ACL/version snapshot before auditing and returning anything.
Completion and failure-code scanning use the same two-transaction structure around provider I/O;
a concurrent rotation requires a fresh scan. Durable report receipts replay before consulting the
provider and are checked again after I/O. Heartbeats and cancellation can commit during a slow
provider read. A cancellation discards the pending disclosure/output. Derived compensation and
reconciliation tasks retain the domain's permission to run while their saga is stopping.

The ordinary output contract remains raw/JSON-escaped UTF-8 scanning for retained declared-secret
versions of at least eight bytes. Managed database output uses its separate typed/encoded scanner
and attempt credential pins. This change does not broaden ordinary blob-output permissions.

Inbound HMAC verifies retained migration/grace versions only after every referenced OpenBao value
has resolved successfully. Any provider read failure returns retryable unavailable, admitting no
event. Outbound signing similarly sends nothing and records its existing durable retry when a
remote version cannot be read. Local-only key-ring grace behavior remains supported. Broker
credentials continue to resolve only the selected current version with no local fallback.
