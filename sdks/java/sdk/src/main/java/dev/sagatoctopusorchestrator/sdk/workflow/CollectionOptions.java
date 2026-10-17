package dev.sagatoctopusorchestrator.sdk.workflow;

/** Required bounds of a runtime-sized collection. */
public record CollectionOptions(int maxItems, int maxConcurrency) {
    /** Creates the required collection bounds. */
    public static CollectionOptions of(int maxItems, int maxConcurrency) {
        return new CollectionOptions(maxItems, maxConcurrency);
    }
}
