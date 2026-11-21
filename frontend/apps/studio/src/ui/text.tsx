import { Code as AstryxCode } from '@astryxdesign/core/Code';
import { EmptyState } from '@astryxdesign/core/EmptyState';
import { Heading as AstryxHeading } from '@astryxdesign/core/Heading';
import { Text as AstryxText } from '@astryxdesign/core/Text';
import { VisuallyHidden as AstryxVisuallyHidden } from '@astryxdesign/core/VisuallyHidden';
import type { ReactNode } from 'react';

/** A heading of the page's outline. `level` is its place in the outline; `look` lets a deep heading read larger or smaller. */
export function Heading({ level, children, id, look, className }: { level: 1 | 2 | 3 | 4; children: ReactNode; id?: string; look?: 1 | 2 | 3 | 4 | 5 | 6; className?: string }) {
  return (
    <AstryxHeading level={look ?? level} {...(look !== undefined && look !== level ? { accessibilityLevel: level } : {})} {...(id ? { id } : {})} {...(className ? { className } : {})}>
      {children}
    </AstryxHeading>
  );
}

/** A paragraph of ordinary text. */
export function Text({ children, muted, id, as = 'p' }: { children: ReactNode; muted?: boolean; id?: string; as?: 'p' | 'span' | 'div' }) {
  return (
    <AstryxText as={as} display={as === 'span' ? 'inline' : 'block'} {...(muted ? { color: 'secondary' as const } : {})} {...(id ? { id } : {})}>
      {children}
    </AstryxText>
  );
}

/** Secondary text under a title or beside a control: what something is for, or why it is unavailable. */
export function Hint({ children, id, role }: { children: ReactNode; id?: string; role?: 'status' | 'note' }) {
  return (
    <AstryxText as="p" display="block" type="supporting" color="secondary" {...(id ? { id } : {})} {...(role ? { role } : {})}>
      {children}
    </AstryxText>
  );
}

/** An identifier, a code or a value, in the monospace face. */
export function Code({ children }: { children: ReactNode }) {
  return <AstryxCode size="inherit">{children}</AstryxCode>;
}

/** What a list or a region says when it has nothing to show. */
export function Empty({ title, children, action, headingLevel = 3 }: { title: string; children?: string; action?: ReactNode; headingLevel?: 2 | 3 | 4 }) {
  return <EmptyState title={title} isCompact headingLevel={headingLevel} {...(children ? { description: children } : {})} {...(action ? { actions: action } : {})} />;
}

/** Text for assistive technology only. */
export function VisuallyHidden({ children }: { children: ReactNode }) {
  return <AstryxVisuallyHidden>{children}</AstryxVisuallyHidden>;
}
