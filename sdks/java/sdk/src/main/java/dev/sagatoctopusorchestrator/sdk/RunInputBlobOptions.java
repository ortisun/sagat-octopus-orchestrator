package dev.sagatoctopusorchestrator.sdk;

/** Optional server-side integrity preconditions for a staged run input. */
public record RunInputBlobOptions(String contentType, Long expectedSize, String expectedSha256) {
    public RunInputBlobOptions {
        contentType = contentType == null || contentType.isBlank() ? "application/octet-stream" : contentType;
    }

    public static RunInputBlobOptions defaults() {
        return new RunInputBlobOptions("application/octet-stream", null, null);
    }
}
