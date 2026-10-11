package dev.sagatoctopusorchestrator.sdk.worker;

/**
 * The code of an activity. It returns the output (anything Jackson can write: a map, a record, a bean, a
 * {@code JsonNode}; {@code null} is JSON {@code null}; {@link #NO_RESULT} sends no value) or fails by throwing: an
 * {@link ActivityFailure} for a failure on purpose, anything else is reported as {@code ACTIVITY_ERROR}.
 *
 * @param <I> the type of the run's input
 * @param <O> the type of the output
 */
@FunctionalInterface
public interface ActivityHandler<I, O> {
    /** Return this to complete the step with no value at all (an empty {@code output_json}), as opposed to {@code null}. */
    Object NO_RESULT = new Object() {
        @Override
        public String toString() {
            return "NO_RESULT";
        }
    };

    O handle(I input, ActivityContext context) throws Exception;
}
