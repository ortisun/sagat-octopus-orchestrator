import { Heading as AstryxHeading } from '@astryxdesign/core/Heading';
import type { ReactNode, Ref } from 'react';
import { lookClass } from './text';

/**
 * A heading that focus can be moved to by the page (after a selection opens a panel, the reader lands
 * on its title). It is not a tab stop: only a script focuses it, through `ref`.
 */
export function FocusHeading({ level, children, id, look, className, ref }: { level: 1 | 2 | 3 | 4; children: ReactNode; id?: string; look?: 1 | 2 | 3 | 4 | 5 | 6; className?: string; ref?: Ref<HTMLHeadingElement> }) {
  return (
    <AstryxHeading level={level} {...(id ? { id } : {})} {...lookClass(level, look, className)} {...(ref ? { ref } : {})} tabIndex={-1} wordBreak="break-word">
      {children}
    </AstryxHeading>
  );
}
