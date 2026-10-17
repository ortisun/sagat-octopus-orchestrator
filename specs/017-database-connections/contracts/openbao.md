# OpenBao provider boundary

Implemented port: `sagat_application::secret_provider::SecretProvider`.
Implementation: `sagat_infra_secrets::openbao::OpenBao`. Backend-only boundary. Management routes are enabled when the control plane has OpenBao settings and an intent binding key.

A provider reference consists of tenant UUID, namespace UUID and provisioning-operation UUID, all allocated/read from trusted Sagat persistence. The configured mount is a single alphanumeric/underscore/hyphen segment. Paths are `/v1/{mount}/data/sagat/credentials/{tenant}/{namespace}/{operation}`. Each operation has one immutable KV object, created with `options.cas = 0` and `data.value` containing base64 of 1–16384 secret bytes. Reads explicitly request provider version 1 and validate the returned version/deletion metadata. Logical rotation creates another operation object; it never overwrites an existing one.

`create` returns success only with a valid version-1 receipt. On a refused CAS, it reads the object: existence returns `Conflict`, never success. The application must verify the durable content binding before activation. `read` returns missing only for provider 404. `delete` permanently removes metadata/versions, and is only for authorized retention/abandoned-intent cleanup.

Production uses verified HTTPS, optional configured CA, and an absolute token-file path maintained by OpenBao Agent. Each request rereads the bounded token file, validates lookup-self has a positive TTL no greater than 24 hours and no root policy, and sends a sensitive `X-Vault-Token` header. Agent authentication/renewal is external to this adapter. The full operation, including lookup and any receipt read, has one 1–10 second deadline. Redirects and environment proxies are disabled. Responses are bounded to 64 KiB before parsing. Only an explicitly selected development mode permits loopback HTTP/root tokens.

Errors are a closed enum: configuration, unavailable, denied, conflict, rejected, invalid response. They carry no provider response text, secret, token or URL. Buffers owned by the adapter for plaintext, base64 JSON and token-file contents are zeroized; HTTP/TLS libraries also hold transient copies, so this is not a claim of eliminating every process-memory copy. No cross-request plaintext cache exists.

Metrics: `secret_provider_requests_total{provider,operation,outcome}` and `secret_provider_request_seconds{provider,operation}`, all closed labels. Read/create tracing spans skip every argument. Application-layer audits and tenant authorization are required before exposing this port to users.

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
