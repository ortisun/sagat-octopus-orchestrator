package dev.sagatoctopusorchestrator.sdk;

/**
 * Called before every request, so that a rotated token is used at once. A static token is the simple case:
 * {@code TokenProvider.of("...")}. The SDK never writes a token to a log, an error message or a metric label.
 */
@FunctionalInterface
public interface TokenProvider {
    /** A bearer token that is valid now. */
    String token() throws Exception;

    /**
     * Whether asking again can give a different token. False for a static token: a refused static token is not
     * asked for again.
     */
    default boolean refreshable() {
        return true;
    }

    /** A provider that always returns {@code token}. */
    static TokenProvider of(String token) {
        if (token == null || token.isEmpty()) {
            throw new IllegalArgumentException("a token is required");
        }
        return new TokenProvider() {
            @Override
            public String token() {
                return token;
            }

            @Override
            public boolean refreshable() {
                return false;
            }

            @Override
            public String toString() {
                return "TokenProvider[static]";
            }
        };
    }

    /** Calls the provider and turns its failure into the canonical error, without echoing its message. */
    static String resolve(TokenProvider provider) {
        try {
            String token = provider.token();
            if (token == null || token.isEmpty()) {
                throw SagatException.builder(ErrorCode.UNAUTHENTICATED, "the token provider returned no token").build();
            }
            return token;
        } catch (SagatException e) {
            throw e;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw SagatException.builder(ErrorCode.CANCELLED, "interrupted while asking for a token").cause(e).build();
        } catch (Exception e) {
            throw SagatException.builder(ErrorCode.UNAUTHENTICATED, "the token provider failed (" + e.getClass().getSimpleName() + ")")
                    .cause(e)
                    .build();
        }
    }
}
