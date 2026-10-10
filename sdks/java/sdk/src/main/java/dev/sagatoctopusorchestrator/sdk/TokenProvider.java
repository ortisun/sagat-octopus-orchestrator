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

    /**
     * Calls the provider and turns its failure into the canonical error: {@code UNAUTHENTICATED} with the reason
     * {@code TOKEN_PROVIDER_FAILED}. The provider's own exception is neither the cause nor part of the message, only its
     * class name: what it says (and the stack trace it leaves) can carry what it was talking to, a client secret or a URL.
     */
    static String resolve(TokenProvider provider) {
        String token;
        try {
            token = provider.token();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw SagatException.of(ErrorCode.CANCELLED, "interrupted while asking for a token");
        } catch (Throwable e) {
            Hooks.rethrowIfFatal(e);
            throw SagatException.builder(ErrorCode.UNAUTHENTICATED, "the token provider failed (" + e.getClass().getSimpleName() + ")")
                    .reason("TOKEN_PROVIDER_FAILED")
                    .tokenProviderFailure()
                    .build();
        }
        if (token == null || token.isEmpty()) {
            throw SagatException.builder(ErrorCode.UNAUTHENTICATED, "the token provider returned no token")
                    .reason("TOKEN_PROVIDER_FAILED")
                    .tokenProviderFailure()
                    .build();
        }
        return token;
    }
}
