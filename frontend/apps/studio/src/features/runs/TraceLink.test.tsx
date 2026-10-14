import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { renderApp } from '../../../test/render';
import { runOf } from '../../../test/runs';
import { RunHeader } from './RunHeader';
import { TraceLink, traceLink } from './TraceLink';

const TRACE_ID = '0123456789abcdef0123456789abcdef';

describe('trace links', () => {
  it('substitutes one strict trace id into a safe HTTPS template', () => {
    expect(traceLink('https://traces.example.test/explore?trace={traceId}', TRACE_ID)).toBe(
      `https://traces.example.test/explore?trace=${TRACE_ID}`,
    );
  });

  it('allows plain HTTP only for loopback tracing tools', () => {
    expect(traceLink('http://localhost:16686/trace/{traceId}', TRACE_ID)).toBe(`http://localhost:16686/trace/${TRACE_ID}`);
    expect(traceLink('http://127.0.0.2:16686/trace/{traceId}', TRACE_ID)).toBe(`http://127.0.0.2:16686/trace/${TRACE_ID}`);
    expect(traceLink('http://[::1]:16686/trace/{traceId}', TRACE_ID)).toBe(`http://[::1]:16686/trace/${TRACE_ID}`);
    expect(traceLink('http://traces.example.test/trace/{traceId}', TRACE_ID)).toBeUndefined();
  });

  it('rejects missing, zero, malformed and non-canonical trace ids', () => {
    const template = 'https://traces.example.test/trace/{traceId}';
    for (const traceId of [undefined, '', '0'.repeat(32), 'a'.repeat(31), 'a'.repeat(33), `${'a'.repeat(31)}g`]) {
      expect(traceLink(template, traceId)).toBeUndefined();
    }
  });

  it('rejects relative, credentialed and malformed templates', () => {
    for (const template of [
      undefined,
      '/trace/{traceId}',
      'javascript:alert({traceId})',
      'https://user:password@traces.example.test/{traceId}',
      'https://traces.example.test/no-placeholder',
      'https://traces.example.test/{traceId}/{traceId}',
    ]) {
      expect(traceLink(template, TRACE_ID)).toBeUndefined();
    }
  });

  it('opens explicitly in a separated tab without making a request itself', () => {
    const fetcher = vi.spyOn(globalThis, 'fetch');
    render(<TraceLink template="https://traces.example.test/trace/{traceId}" traceId={TRACE_ID} />);
    const link = screen.getByRole('link', { name: 'Open trace' });
    expect(link).toHaveAttribute('href', `https://traces.example.test/trace/${TRACE_ID}`);
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', expect.stringContaining('noopener'));
    expect(link).toHaveAttribute('rel', expect.stringContaining('noreferrer'));
    expect(fetcher).not.toHaveBeenCalled();
    fetcher.mockRestore();
  });

  it('renders nothing when either deployment config or trace id is unusable', () => {
    const { container } = render(<TraceLink template={undefined} traceId={TRACE_ID} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('keeps trace copying and adds the configured link to run details', () => {
    renderApp(
      <RunHeader
        run={runOf({ traceId: TRACE_ID })}
        status="RUNNING"
        workflowName="orders"
        version="v3"
        tenant="acme"
        namespace="dev"
        follow="idle"
        operations={[]}
        onChoose={vi.fn()}
        onRefresh={vi.fn()}
        refreshing={false}
      />,
      { appOptions: { settings: { oidc: null, traceUrl: 'https://traces.example.test/trace/{traceId}' } } },
    );
    expect(screen.getByRole('button', { name: 'Copy trace id' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Open trace' })).toHaveAttribute('href', `https://traces.example.test/trace/${TRACE_ID}`);
  });
});
