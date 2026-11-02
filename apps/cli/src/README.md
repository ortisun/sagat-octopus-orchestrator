# Sagat CLI source

## Overview

Embeddable sagat_cli command dispatcher and the sagat executable: local authoring, platform operations, development workers and onboarding diagnostics.

## Interface

- `run` / `run_until` accept arguments, an `Env` and caller-owned output streams; public helpers sanitize terminal text and expose exit constants `EXIT_DONE` through `EXIT_TIMEOUT`. `main` adapts process arguments and exit status. [lib.rs:run] [lib.rs:run_until] [lib.rs:Env] [lib.rs:printable] [lib.rs:one_line] [main.rs:main]
- Public modules are `args`, `client`, `diagnostics`, `doctor`, `manifest`, `mocks`, `new` and `telemetry`. Other command modules remain private. [lib.rs:args] [lib.rs:telemetry]
- Arguments expose `USAGE`, `UsageError`, `Common`, `Command`, `Invocation` and `parse`; the HTTP boundary exposes `Failure`, `CallError`, `Client`, encoding/query helpers and categorized transport diagnostics. [args.rs:parse] [args.rs:Command] [client.rs:Failure] [client.rs:CallError] [client.rs:Client] [client.rs:encode] [client.rs:with_query]
- Diagnostics expose the catalog, typed `Diagnostic`, document/classification reports and server-problem rendering. Doctor exposes `WORKER_PROTOCOL`, `TokenFacts` and `read_token`; JWT claim extraction does not verify authentication. [diagnostics.rs:Diagnostic] [diagnostics.rs:CATALOG] [diagnostics.rs:analyze_document] [diagnostics.rs:classification_report] [doctor.rs:read_token]
- Manifest APIs provide parsing/loading, slug/path/YAML helpers, `Manifest`, `Entry`, `Problem`, `DocumentError` and `Loaded`. Mock APIs provide `Mocks`, task kinds/answers, activity discovery and bounded mock schemas. [manifest.rs:Manifest] [manifest.rs:load] [manifest.rs:DocumentError] [manifest.rs:Loaded] [mocks.rs:Mocks] [mocks.rs:Kind] [mocks.rs:Answer] [mocks.rs:activities_by_role]
- Templates expose `File`, `Template`, `TEMPLATES` and `find`; onboarding telemetry exposes `Onboarding`. [new.rs:File] [new.rs:Template] [new.rs:TEMPLATES] [new.rs:find] [telemetry.rs:Onboarding]
- CLI verbs cover help/version/doctor/new; manifest validate/diff/export/publish; promote/rollback/history/requests/approve/reject; workflow validate/publish/run/runs/inspect/cancel/rerun; script test/publish; worker dev/bind; dev token; and namespace reveal-grants list/grant/revoke. Global flags select server, token file, namespace, gRPC and manifest file. [args.rs:USAGE] [args.rs:parse]

## Invariants

- Exit values are 0 done, 1 failed, 2 usage, 3 pending approval, 4 breaking refusal, 5 differences and 6 timeout/person wait. Unknown flags, invalid positional arguments and bounded numeric settings are rejected by parsing. [lib.rs:EXIT_DONE] [lib.rs:EXIT_TIMEOUT] [args.rs:parse]
- Authenticated endpoints require HTTPS or loopback HTTP without embedded credentials; tokens are nonempty graphic ASCII. Redirects are disabled and responses/deadlines are bounded. [client.rs:is_safe_address] [client.rs:Client] [client.rs:MAX_ANSWER] [client.rs:MAX_PUBLIC_ANSWER]
- Manifests allow at most 500 workflows, files up to 1 MiB and release notes up to 2048 bytes. Relative paths and canonical targets must remain within the manifest directory; duplicate names/files and document-name mismatches are rejected. YAML expansion has a separate accounting bound. [manifest.rs:MAX_WORKFLOWS] [manifest.rs:MAX_BYTES] [manifest.rs:MAX_RELEASE_NOTES] [manifest.rs:confined] [manifest.rs:MAX_EXPANDED]
- Mocks cap activity count and delays, require one answer form, and constrain reconciler verdicts. Mock sequences repeat their final answer; missing forward mocks fail, while missing reconciliations remain unknown. [mocks.rs:MAX_NAMES] [mocks.rs:MAX_DELAY_MS] [mocks.rs:Mocks]
- Development workers refuse unknown/non-development environments unless explicitly allowed. Value/PII disclosure rendering accepts closed shapes with bounded trees; terminal output sanitizers remove control/invisible formatting characters. [worker.rs:environment_of] [workflow.rs:render_disclosure] [workflow.rs:render_node] [lib.rs:printable]

## Side Effects

- Reads environment, credentials, manifests, workflow/mock/script sources and file inputs. Commands can create templates, export files and private development token files; template/export helpers enforce their overwrite/symlink rules. [lib.rs:Env] [manifest.rs:load] [new.rs:write] [export.rs:write_new] [dev.rs:write_private]
- Uses authenticated REST for workflow, run, version/publication, script, binding and disclosure operations; development workers register/poll/report over gRPC. Doctor also makes bounded TCP probes. Waiting commands use timers and workers handle shutdown signals. [client.rs:Client] [worker.rs:channel] [worker.rs:dev] [doctor.rs:connect]
- Writes caller output streams/process status; opt-in telemetry can install tracing, export bounded signals and append onboarding NDJSON records. [lib.rs:run_until] [main.rs:main] [telemetry.rs:Onboarding] [telemetry.rs:write_report]

## Verification

From the repository root, with a private Cargo target directory outside the checkout:

```sh
cargo test -p sagat-cli --locked
```

This command is derived from the package manifest; it was not run during this documentation pass.
