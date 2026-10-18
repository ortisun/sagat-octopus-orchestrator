# Expression boundary contract

All five current public SDK builders produce identical reviewed canonical documents. The existing
public query operation executes1024UTF8-byte source (508nonASCII characters), literal64 and
constructed4096. Constructed4097 fails with stable existing VALIDATION_FAILED / reason
OPERATION_EVALUATION_FAILED; a subsequent successful query sees unchanged Run/state revisions.

publicationRejections maps SDK-authored workflow keys to the exact existing server refusal reason.
The shared HTTP staging phase creates each editable draft, attempts actual publication, checks
422/VALIDATION_FAILED/not-retryable/exactreason, and independently verifies active_version_id=NULL
and zero workflow_versions. It omits these workflows from runtime configuration and exports closed
publicationEvidence in complete reviewed normalized output. Syntax-invalid sources never execute.
Publication is explicitly a shared server HTTP phase; SDKs expose no publication API. No runner
semantics fork, invented runtime refusal, product/wire change or automatic golden blessing.

Unknown rejection keys/invalid map values must fail staging; expected evidence must retain every
refused document. Normal scenarios without this phase keep unchanged behavior and output.
