import { isApiError } from '../api/errors';
import { Banner } from './Banner';
import { Button } from './Button';

/** What to tell a person who hit an error, with the trace id they can quote. Never a payload. */
export function ErrorState({ error, onRetry, what }: { error: unknown; onRetry?: () => void; what?: string }) {
  if (!isApiError(error)) {
    return (
      <Banner tone="error" title={what ? `${what} failed.` : 'Something went wrong.'}>
        Reload the page; if it happens again, tell whoever runs the platform.
      </Banner>
    );
  }
  return (
    <Banner tone="error" title={what ? `${what} failed.` : 'The request failed.'}>
      {error.message}{' '}
      {error.traceId ? (
        <span>
          Trace <code>{error.traceId}</code>.{' '}
        </span>
      ) : null}
      {error.retryable && onRetry ? <Button onClick={onRetry}>Try again</Button> : null}
    </Banner>
  );
}
