package dev.sagatoctopusorchestrator.sdk.workflow;

/** One ordered branch of a switch decision. */
public record SwitchCase(String id, String when, String target) {}
