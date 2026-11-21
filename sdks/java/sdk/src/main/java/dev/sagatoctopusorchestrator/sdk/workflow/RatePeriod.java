package dev.sagatoctopusorchestrator.sdk.workflow;

import java.util.Locale;

/** The sliding window of a rate limit (feature 022). */
public enum RatePeriod {
    /** One second. */
    SECOND,
    /** One minute. */
    MINUTE,
    /** One hour. */
    HOUR;

    String wire() {
        return name().toLowerCase(Locale.ROOT);
    }
}
