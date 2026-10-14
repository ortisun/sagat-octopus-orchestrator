package dev.sagatoctopusorchestrator.sdk.worker;

/** Optional upload metadata and preconditions. Zero and empty values omit a precondition. */
public record BlobWriteOptions(String contentType, long expectedSize, String expectedSha256) {
    public BlobWriteOptions {
        contentType = contentType == null ? "" : contentType;
        expectedSha256 = expectedSha256 == null ? "" : expectedSha256;
    }

    public static BlobWriteOptions defaults() {
        return new BlobWriteOptions("", 0, "");
    }
}
