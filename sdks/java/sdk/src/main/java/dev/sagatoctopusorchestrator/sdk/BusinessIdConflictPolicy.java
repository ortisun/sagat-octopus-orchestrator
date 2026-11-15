package dev.sagatoctopusorchestrator.sdk;

/**
 * What to do when an open execution already holds the business identifier. A policy is a string: one this SDK does not
 * know is sent as given, and the server judges it.
 */
public final class BusinessIdConflictPolicy {
    /** The default: the call is refused with {@code CONFLICT} ({@code BUSINESS_ID_IN_USE}). */
    public static final String FAIL = "FAIL";
    /** Nothing is created: the call answers the run that holds the identifier, and {@code started} is false. */
    public static final String USE_EXISTING = "USE_EXISTING";

    private BusinessIdConflictPolicy() {}
}
