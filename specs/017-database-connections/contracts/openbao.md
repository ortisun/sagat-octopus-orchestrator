# OpenBao provider boundary

Implemented port: `sagat_application::secret_provider::SecretProvider`.
Implementation: `sagat_infra_secrets::openbao::OpenBao`. Internal backend boundary only; no HTTP management route is enabled by this adapter alone.

A provider reference consists of tenant UUID, namespace UUID and provisioning-operation UUID, all allocated/read from trusted Sagat persistence. The configured mount is a single alphanumeric/underscore/hyphen segment. Paths are `/v1/{mount}/data/sagat/credentials/{tenant}/{namespace}/{operation}`. Each operation has one immutable KV object, created with `options.cas = 0` and `data.value` containing base64 of 1–16384 secret bytes. Reads explicitly request provider version 1 and validate the returned version/deletion metadata. Logical rotation creates another operation object; it never overwrites an existing one.

`create` returns success only with a valid version-1 receipt. On a refused CAS, it reads the object: existence returns `Conflict`, never success. The application must verify the durable content binding before activation. `read` returns missing only for provider 404. `delete` permanently removes metadata/versions, and is only for authorized retention/abandoned-intent cleanup.

Production uses verified HTTPS, optional configured CA, and an absolute token-file path maintained by OpenBao Agent. Each request rereads the bounded token file, validates lookup-self has a positive TTL no greater than 24 hours and no root policy, and sends a sensitive `X-Vault-Token` header. Agent authentication/renewal is external to this adapter. The full operation, including lookup and any receipt read, has one 1–10 second deadline. Redirects and environment proxies are disabled. Responses are bounded to 64 KiB before parsing. Only an explicitly selected development mode permits loopback HTTP/root tokens.

Errors are a closed enum: configuration, unavailable, denied, conflict, rejected, invalid response. They carry no provider response text, secret, token or URL. Buffers owned by the adapter for plaintext, base64 JSON and token-file contents are zeroized; HTTP/TLS libraries also hold transient copies, so this is not a claim of eliminating every process-memory copy. No cross-request plaintext cache exists.

Metrics: `secret_provider_requests_total{provider,operation,outcome}` and `secret_provider_request_seconds{provider,operation}`, all closed labels. Read/create tracing spans skip every argument. Application-layer audits and tenant authorization are required before exposing this port to users.

Sources: [OpenBao HTTP API](https://openbao.org/api-docs/), [KV v2](https://openbao.org/docs/secrets/kv/kv-v2/), [official releases](https://github.com/openbao/openbao/releases). Real-service fixture: pinned `openbao/openbao:2.7.1`; production digest/profile is a separate delivery task.
