package dev.sagatoctopusorchestrator.sdk;

/**
 * What to do when a closed execution still remembers the business identifier. A policy is a string: one this SDK does
 * not know is sent as given, and the server judges it.
 */
public final class BusinessIdReusePolicy {
    /** The default: the identifier may be used again. */
    public static final String ALLOW_DUPLICATE = "ALLOW_DUPLICATE";
    /** The identifier may be used again only when the closed execution failed. */
    public static final String ALLOW_DUPLICATE_FAILED_ONLY = "ALLOW_DUPLICATE_FAILED_ONLY";
    /** The identifier may not be used again while the server remembers it. */
    public static final String REJECT_DUPLICATE = "REJECT_DUPLICATE";

    private BusinessIdReusePolicy() {}
}
