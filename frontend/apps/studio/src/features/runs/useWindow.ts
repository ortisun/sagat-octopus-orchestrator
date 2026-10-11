import { useVirtualizer } from '@tanstack/react-virtual';
import type { VirtualItem, Virtualizer } from '@tanstack/react-virtual';
import type { RefObject } from 'react';

export interface Windowed {
  /** The rows to draw, with their positions. */
  items: VirtualItem[];
  /** Height of the rows above the first one drawn, and below the last (spacer rows keep the scroll bar honest). */
  padTop: number;
  padBottom: number;
  /** Put on each row (with `data-index`) so its real height is used. */
  measure: (element: Element | null) => void;
  virtualizer: Virtualizer<HTMLElement, Element>;
}

/**
 * Draws only the rows in view and a few around them, in a scrolling element, with table
 * semantics kept by spacer rows. A list of 10 000 rows costs what a screenful costs (D147).
 */
export function useWindowedRows(options: { count: number; estimate: number; overscan?: number; scroller: RefObject<HTMLElement | null> }): Windowed {
  // The virtualizer object is read each render and never memoized by this code, which is what the rule warns about.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer<HTMLElement, Element>({
    count: options.count,
    getScrollElement: () => options.scroller.current,
    estimateSize: () => options.estimate,
    overscan: options.overscan ?? 8,
  });
  const items = virtualizer.getVirtualItems();
  const first = items[0];
  const last = items[items.length - 1];
  return {
    items,
    padTop: first?.start ?? 0,
    padBottom: last === undefined ? 0 : Math.max(0, virtualizer.getTotalSize() - last.end),
    measure: virtualizer.measureElement,
    virtualizer,
  };
}
