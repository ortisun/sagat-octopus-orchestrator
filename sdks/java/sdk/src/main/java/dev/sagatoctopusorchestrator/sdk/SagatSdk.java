package dev.sagatoctopusorchestrator.sdk;

/** Constants that identify this SDK on the wire. */
public final class SagatSdk {
    /** SDK version, sent in {@code User-Agent}. Kept equal to the Maven version by a unit test. */
    public static final String VERSION = "0.1.0";

    /** The language this SDK declares when a worker registers (SPEC section 6.1). */
    public static final String LANGUAGE = "java";

    /** The newest minor version of worker protocol 1 this SDK speaks (SPEC section 6.1). */
    public static final String PROTOCOL_VERSION = "1.2";

    private SagatSdk() {}

    static String userAgent(String suffix) {
        String base = "sagat-sdk-" + LANGUAGE + "/" + VERSION;
        return suffix == null || suffix.isBlank() ? base : base + " " + suffix;
    }
}
