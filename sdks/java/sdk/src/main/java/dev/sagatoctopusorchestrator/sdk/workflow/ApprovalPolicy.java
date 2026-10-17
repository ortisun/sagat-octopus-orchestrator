package dev.sagatoctopusorchestrator.sdk.workflow;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/** Subjects and built-in roles eligible to decide an approval. */
public record ApprovalPolicy(List<String> subjects, List<ApprovalRole> roles, Boolean excludeInitiator) {
    public ApprovalPolicy {
        subjects = subjects == null ? null : Collections.unmodifiableList(new ArrayList<>(subjects));
        roles = roles == null ? null : Collections.unmodifiableList(new ArrayList<>(roles));
    }

    /** Creates a policy whose omitted initiator flag uses the server default of true. */
    public static ApprovalPolicy of(List<String> subjects, List<ApprovalRole> roles) {
        return new ApprovalPolicy(subjects, roles, null);
    }

    /** Returns this policy with an explicit initiator exclusion choice. */
    public ApprovalPolicy withExcludeInitiator(boolean exclude) {
        return new ApprovalPolicy(subjects, roles, exclude);
    }
}
