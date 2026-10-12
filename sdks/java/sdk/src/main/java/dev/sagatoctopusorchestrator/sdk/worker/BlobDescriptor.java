package dev.sagatoctopusorchestrator.sdk.worker;

/** Verified identity of an immutable worker-protocol blob. */
public record BlobDescriptor(String id, long size, String sha256, String contentType) {}
