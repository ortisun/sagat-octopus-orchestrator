# sagat_octopus_orchestrator

## Overview

Typed async/blocking REST facade, local workflow/database authoring and optional telemetry. [__init__.py:__all__]

## Interface

- The facade exports configuration/token/trace/retry/error/JSON helpers and the client, worker, database and workflow symbols enumerated by __all__. Client result contracts cover runs/parents/segments/pages, operations, signals, events/journal, chains, approvals, query results and update receipts. [__init__.py:__all__] [client.py:Run]
- SagatClient provides workflow resolution, run/blob lifecycle, approvals, declared queries/updates, run/chain signals and commands, chain inspection/paging/waits, event publication and journal paging. SyncSagatClient exposes blocking counterparts and list iterators. [client.py:SagatClient] [sync.py:SyncSagatClient]
- Workflow APIs include RetryOptions, SwitchCase, ApprovalPolicy, QueryDefinition, UpdateDefinition, StateDefinition, Flow, Body, WorkflowBuilder and workflow/flow/body factories; builders cover node authoring, metadata, triggers, schemas, state, bodies and chain policy. Database APIs include ConnectionRef, DatabaseType, DatabaseValue, DatabaseParameter and DatabaseLimits. [workflow.py:Flow] [workflow.py:Body] [workflow.py:WorkflowBuilder] [database.py:ConnectionRef] [database.py:DatabaseLimits]
- OpenTelemetryBridge is an opt-in module API. Internal boundaries include HttpTransport, canonical_json, CATALOG, Secret and ProviderFailure. [opentelemetry.py:OpenTelemetryBridge] [_http.py:HttpTransport] [_json.py:canonical_json] [_catalog.py:CATALOG] [config.py:Secret] [config.py:ProviderFailure]

- [_proto](./_proto/README.md) — component contract.
- [worker](./worker/README.md) — component contract.

## Invariants

- Secret redacts string/representation output and resists pickling; configuration wraps tokens. HTTP redirects are refused, decoded bodies cap at 32 MiB and safe retries/server waits are bounded. [config.py:Secret] [config.py:ClientConfig] [_http.py:HttpTransport] [_http.py:MAX_BODY_BYTES] [retry.py:MAX_SERVER_WAIT]
- JSON rejects non-JSON types, non-string keys, invalid Unicode, non-finite numbers, excessive depth and wire integers outside signed 64-bit. Canonical JSON applies UTF-16 key ordering and safe-number constraints. [_json.py:encode_json] [_json.py:canonical_json]
- Client IDs/names/keys/revisions and selected response shapes are checked locally. Uploads are single-pass/non-retried and require EOF plus measured size/SHA-256 agreement. [client.py:_run_id] [client.py:upload_run_input]
- Builders validate local IDs/references/bounds/body shapes and emit IR without publishing or proving server acceptance. The blocking facade delegates to one async client and rejects use after close or from its own loop thread. [workflow.py:build] [database.py:database_node] [sync.py:_run]
- Telemetry contains observer failures, caps open task spans at 1000 and leaves provider/exporter configuration to the application. [opentelemetry.py:on_event] [opentelemetry.py:_MAX_OPEN_TASKS]

## Side Effects

- Clients open/close HTTP connections, send authenticated requests and schedule retry/wait sleeps. The blocking facade creates and joins a daemon thread with a private asyncio loop. [_http.py:HttpTransport] [client.py:SagatClient] [sync.py:SyncSagatClient]
- TLS client-certificate setup writes certificate/key material to a private temporary directory for SSLContext.load_cert_chain, then removes it. Helpers emit bounded logs/plaintext warnings; telemetry emits metrics/spans via application providers. [config.py:ssl_context] [config.py:warn_if_plaintext] [opentelemetry.py:OpenTelemetryBridge]

## Verification

Run from the repository root:

```sh
cd sdks/python && .venv/bin/python -m pytest && .venv/bin/mypy && .venv/bin/ruff check .
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
