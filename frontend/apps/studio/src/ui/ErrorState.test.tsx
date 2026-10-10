import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ApiError } from '../api/errors';
import { ErrorState } from './ErrorState';

const TRACE = '0123456789abcdef0123456789abcdef';

describe('ErrorState', () => {
  it('says what the platform said, what to do about it, and the code and trace id to quote', () => {
    render(<ErrorState error={new ApiError({ code: 'FORBIDDEN', status: 403, message: "the caller's role does not allow this operation", traceId: TRACE })} what="Retrying the step" />);
    const alert = screen.getByText(/Retrying the step failed/).closest('div') as HTMLElement;
    expect(alert).toHaveTextContent("the caller's role does not allow this operation");
    expect(alert).toHaveTextContent('Ask an administrator for the permission it needs.');
    expect(alert).toHaveTextContent('Code FORBIDDEN.');
    expect(alert).toHaveTextContent(`Trace ${TRACE}.`);
  });

  it('names the stable reason of a conflict, and leaves its message to say the way out', () => {
    render(<ErrorState error={new ApiError({ code: 'CONFLICT', status: 409, message: 'the step is not waiting for a decision', details: { reason: 'STATE_CONFLICT' }, traceId: TRACE })} />);
    expect(screen.getByText(/the step is not waiting for a decision/).closest('div')).toHaveTextContent('Code CONFLICT (STATE_CONFLICT).');
    expect(screen.queryByText(/Ask an administrator/)).not.toBeInTheDocument();
  });

  it('shows nothing of what is not an answer of the platform: no message, no code, no trace', () => {
    render(<ErrorState error={new Error('TypeError: x is undefined at /src/secret.ts:12')} what="Opening the run" />);
    expect(screen.getByText(/Opening the run failed/)).toBeInTheDocument();
    expect(document.body).not.toHaveTextContent('secret.ts');
    expect(document.body).not.toHaveTextContent('Code');
  });

  it('puts the text of an error as text, never as markup', () => {
    render(<ErrorState error={new ApiError({ code: 'BAD_REQUEST', status: 400, message: '<img src=x onerror=alert(1)>' })} />);
    expect(document.querySelector('img')).toBeNull();
    expect(document.body).toHaveTextContent('<img src=x onerror=alert(1)>');
  });
});
