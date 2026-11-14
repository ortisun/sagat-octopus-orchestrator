# Internal

## Overview

Internal REST transport, error metadata, bounds, JSON/TLS and safe observability helpers. [HttpTransport.cs:HttpTransport] [Wire.cs:Wire]

## Interface

- Internal APIs are HttpCall/HttpAnswer/HttpTransport, ErrorCatalog, Limits, Log, Telemetry, Tls, Wire, Safe/SafeLogger, StrictJsonEncoder, Answers (the typed reads of an answer) and Requests/QueryBuilder (the checks and the spelling of a path and a query). [HttpTransport.cs:HttpCall] [HttpTransport.cs:HttpAnswer] [HttpTransport.cs:HttpTransport] [ErrorCatalog.cs:ErrorCatalog] [Limits.cs:Limits] [Log.cs:Log] [Telemetry.cs:Telemetry] [Tls.cs:Tls] [Wire.cs:Wire] [Safe.cs:SafeLogger] [StrictJsonEncoder.cs:StrictJsonEncoder] [Answers.cs:Answers] [Requests.cs:Requests] [Requests.cs:QueryBuilder]

## Invariants

- Only safe-to-repeat calls with retryable mapped errors repeat. Bodies cap at 32 MiB, server waits at 60 seconds; error text/reasons are bounded and trace IDs require 32 lowercase hex characters. [HttpTransport.cs:SendAsync] [Limits.cs:MaxBodyBytes]
- SDK handlers disable redirects, preserve certificate checks, support custom roots/mTLS and reject ServerName on plaintext. Safe wrappers contain application logger/metric/trace listener exceptions. [Tls.cs:CreateHandler] [Safe.cs:Run]

## Side Effects

- Performs HTTP I/O and retry waits, reads the embedded catalog, constructs TLS/network handlers and emits fixed-template logs plus ActivitySource/Meter measurements. [HttpTransport.cs:OnceAsync] [ErrorCatalog.cs:RawBytes] [Tls.cs:Configure] [Log.cs:Log] [Telemetry.cs:Source]

## Verification

Run from the repository root:

```sh
cd sdks/dotnet && dotnet test && dotnet format --verify-no-changes
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
