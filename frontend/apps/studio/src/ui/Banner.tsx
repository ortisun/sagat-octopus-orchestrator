import { Banner as AstryxBanner } from '@astryxdesign/core/Banner';
import type { ReactNode } from 'react';

type Tone = 'error' | 'warn' | 'info' | 'ok';
const STATUS = { error: 'error', warn: 'warning', info: 'info', ok: 'success' } as const;

/**
 * A message in the page. Errors and warnings are announced; information is not (Decision 2404: Astryx draws
 * the banner, the Studio adds the live-region role that Requirement 007-008 needs).
 */
export function Banner({ tone, children, title }: { tone: Tone; children?: ReactNode; title?: string }) {
  return (
    <div role={tone === 'error' || tone === 'warn' ? 'alert' : 'status'}>
      {title ? (
        <AstryxBanner status={STATUS[tone]} title={title} description={children ? <span>{children}</span> : undefined} />
      ) : (
        <AstryxBanner status={STATUS[tone]} title={<span>{children}</span>} />
      )}
    </div>
  );
}
