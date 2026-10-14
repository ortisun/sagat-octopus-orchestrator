const PLACEHOLDER = '{traceId}';
const TRACE_ID = /^(?!0{32}$)[0-9a-fA-F]{32}$/;

function isLoopback(hostname: string): boolean {
  const host = hostname.replace(/^\[|\]$/g, '').toLowerCase();
  return host === 'localhost' || host === '::1' || /^127(?:\.\d{1,3}){3}$/.test(host);
}

/** Build a trace-tool URL only when both deployment config and trace identity are trustworthy. */
export function traceLink(template: string | null | undefined, traceId: string | null | undefined): string | undefined {
  if (typeof template !== 'string' || !TRACE_ID.test(traceId ?? '')) return undefined;
  if (template.split(PLACEHOLDER).length !== 2 || /\s/.test(template)) return undefined;
  const expanded = template.replace(PLACEHOLDER, traceId as string);
  let url: URL;
  try {
    url = new URL(expanded);
  } catch {
    return undefined;
  }
  if (url.username !== '' || url.password !== '') return undefined;
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && isLoopback(url.hostname))) return undefined;
  return url.href;
}

/** An explicit navigation to the configured trace tool. Rendering never contacts that tool. */
export function TraceLink({ template, traceId }: { template: string | null | undefined; traceId: string | null | undefined }) {
  const href = traceLink(template, traceId);
  return href ? (
    <a href={href} target="_blank" rel="noopener noreferrer">
      Open trace
    </a>
  ) : null;
}
