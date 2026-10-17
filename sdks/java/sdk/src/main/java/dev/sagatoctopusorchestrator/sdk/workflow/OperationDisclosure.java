package dev.sagatoctopusorchestrator.sdk.workflow;

/** Explicit authorization boundaries for declared operation results. */
public final class OperationDisclosure {
    /** An authorized caller may receive the declared, bounded business result. */
    public static final String DECLARED_BUSINESS_RESULT_V1 = "declared-business-result-v1";

    private OperationDisclosure() {}
}
