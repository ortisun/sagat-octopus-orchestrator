package dev.sagatoctopusorchestrator.sdk.worker;

import java.util.Objects;

/** Explicitly selects a descriptor as an activity result; ordinary JSON-shaped objects remain JSON. */
public record BlobOutput(BlobDescriptor descriptor) {
    public BlobOutput {
        Objects.requireNonNull(descriptor, "descriptor");
    }
}
