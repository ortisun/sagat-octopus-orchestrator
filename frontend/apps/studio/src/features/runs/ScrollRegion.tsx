import type { ReactNode, RefObject } from 'react';

/**
 * The scrolling box of a long table. It is focusable on purpose: a box that scrolls must be
 * reachable by keyboard, or people who do not use a mouse cannot read what is off screen.
 */
export function ScrollRegion({ label, scroller, children }: { label: string; scroller: RefObject<HTMLDivElement | null>; children: ReactNode }) {
  return (
    // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- a scrollable region must take focus (WCAG 2.1.1)
    <div className="rn-scroll" ref={scroller} data-window="" tabIndex={0} role="region" aria-label={label}>
      {children}
    </div>
  );
}
