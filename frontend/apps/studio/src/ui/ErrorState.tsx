import { isApiError, remedy } from '../api/errors';
import { Banner } from './Banner';
import { Button } from './Button';

/** What to tell a person who hit an error: what the platform said, what to do about it, and the code and trace id they can quote. Never a payload. */
export function ErrorState({ error, onRetry, what }: { error: unknown; onRetry?: () => void; what?: string }) {
  if (!isApiError(error)) {
    return (
      <Banner tone="error" title={what ? `${what} failed.` : 'Something went wrong.'}>
        Reload the page; if it happens again, tell whoever runs the platform.
      </Banner>
    );
  }
  // What the platform says to do wins over the general advice for the kind of error.
  const advice = error.remediation ?? remedy(error.code);
  return (
    <Banner tone="error" title={what ? `${what} failed.` : 'The request failed.'}>
      {error.message}{' '}
      {advice ? <>{advice} </> : null}
      <span>
        Code <code>{error.code}</code>
        {error.reason ? (
          <>
            {' '}
            (<code>{error.reason}</code>)
          </>
        ) : null}
        .{' '}
      </span>
      {error.traceId ? (
        <span>
          Trace <code>{error.traceId}</code>.{' '}
        </span>
      ) : null}
      {error.retryable && onRetry ? <Button onClick={onRetry}>Try again</Button> : null}
    </Banner>
  );
}
