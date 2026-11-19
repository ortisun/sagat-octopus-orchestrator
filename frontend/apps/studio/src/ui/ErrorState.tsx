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
  // design 043: where a run input is refused and by which keyword of its schema; a pointer of declared names, never a value
  const field = typeof error.details?.['field'] === 'string' ? error.details['field'] : undefined;
  const rule = typeof error.details?.['rule'] === 'string' && /^[A-Za-z]{1,32}$/.test(error.details['rule']) ? error.details['rule'] : undefined;
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
      {field || rule ? (
        <span>
          {field ? (
            <>
              At <code>{field}</code>
            </>
          ) : null}
          {field && rule ? ', ' : null}
          {rule ? (
            <>
              rule <code>{rule}</code>
            </>
          ) : null}
          .{' '}
        </span>
      ) : null}
      {error.traceId ? (
        <span>
          Trace <code>{error.traceId}</code>.{' '}
        </span>
      ) : null}
      {error.retryable && onRetry ? <Button onClick={onRetry}>Try again</Button> : null}
    </Banner>
  );
}
