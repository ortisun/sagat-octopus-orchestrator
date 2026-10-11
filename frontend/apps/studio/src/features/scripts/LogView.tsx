import { useMemo, useState } from 'react';
import { Button } from '../../ui/Button';
import { clipText, sanitizeLog } from './text';
import type { ClipLimits } from './text';

/** What is drawn at first, and at most: enough to read, bounded so a flood of output cannot freeze the page. */
const FIRST: ClipLimits = { maxChars: 32 * 1024, maxLines: 400, maxLineChars: 2000 };
const WHOLE: ClipLimits = { maxChars: 1024 * 1024, maxLines: 20_000, maxLineChars: 20_000 };

/**
 * Text from a script or from the platform, as text: preformatted, scrollable (and reachable by
 * keyboard because it scrolls), cleaned of terminal escapes and hidden characters, and clipped with
 * a notice when it is long.
 */
export function LogView({ text, label, platformCut, empty }: { text: string; label: string; platformCut?: boolean; empty: string }) {
  const [whole, setWhole] = useState(false);
  const safe = useMemo(() => sanitizeLog(text), [text]);
  const shown = useMemo(() => clipText(safe, whole ? WHOLE : FIRST), [safe, whole]);
  return (
    <div className="log-view">
      {/* A region that scrolls must be reachable by keyboard, or its end cannot be read without a mouse. */}
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex */}
      <pre className="log" role="region" aria-label={label} tabIndex={0}>
        {shown.text === '' ? empty : shown.text}
      </pre>
      {shown.clipped ? (
        <p className="hint">
          Showing the start of {shown.totalChars.toLocaleString()} characters ({shown.totalLines.toLocaleString()} lines).{' '}
          {whole ? (
            'This is the most the Studio draws.'
          ) : (
            <Button onClick={() => setWhole(true)}>Show more</Button>
          )}
        </p>
      ) : null}
      {platformCut ? <p className="hint">The platform cut this at the output limit of the script; what follows was not kept.</p> : null}
    </div>
  );
}
