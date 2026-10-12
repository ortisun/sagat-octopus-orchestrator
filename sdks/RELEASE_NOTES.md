# SDK release notes

## 0.1.0 — release candidate

This is the first local release candidate for the TypeScript, Python, Go, C#/.NET and Java SDKs.
The SDKs are still in the `0.x` compatibility series. Review the language README and the shared
[`SPEC.md`](./SPEC.md) before upgrading a worker or client.

### Included

- REST clients for starting, reading, listing, cancelling and signalling runs, and publishing
  events, with stable idempotency keys, bounded retries and canonical safe errors.
- Typed workflow builders and worker hosts with lease heartbeats, cancellation, compensation,
  reconciliation and protocol negotiation.
- Worker protocol 1.3 streamed blob upload and verified download. Each SDK checks the byte count and
  SHA-256 digest incrementally, exposes an explicit blob-output type and never treats an unfinished
  stream as verified.
- One-pass run-input blob upload and `startWithBlob` (`start_with_blob` in Python). Uploads are not
  retried; a successful response is accepted only after the source reaches EOF and the returned
  descriptor matches the locally measured size and digest.
- Opt-in OpenTelemetry bridges using each ecosystem's API. The application owns provider and
  exporter configuration. Fixed span names and bounded attributes exclude payloads, tokens, trace
  headers and arbitrary exception text; telemetry failures do not change workflow behavior.
- MIT-licensed local packages and clean-consumer smoke tests for all five ecosystems.

### Compatibility

- Workers advertise protocol 1.3 and negotiate the newest compatible protocol 1 minor supported by
  the server. Blob APIs require negotiated protocol 1.3 and fail closed against a 1.2 server.
- Existing inline JSON values are never converted to blob references by their shape. A caller must
  use the language's explicit blob descriptor/output type.
- The package version remains `0.1.0`; no public registry publication or stable `1.x` compatibility
  promise is implied by this candidate.

### Candidate artifacts

The credential-free packaging workflow creates npm, Python wheel/source, Go module source, NuGet and
Maven artifacts plus `SHA256SUMS`. The Maven candidate includes the main JAR, POMs, source JAR and
Javadoc JAR. External publication still requires registry ownership and credentials; Maven Central
also requires release signing.
