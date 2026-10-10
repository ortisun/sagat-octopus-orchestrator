/**
 * React Flow measures the page with browser APIs jsdom does not have. Importing this file (for its
 * effects) gives a test the few it needs, as inert stand-ins: nothing here draws or measures.
 */
class InertResizeObserver {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

class InertDOMMatrixReadOnly {
  m22: number;
  constructor(transform?: string) {
    const scale = transform?.match(/scale\(([\d.]+)\)/)?.[1];
    this.m22 = scale === undefined ? 1 : Number(scale);
  }
}

const host = globalThis as unknown as Record<string, unknown>;
host['ResizeObserver'] ??= InertResizeObserver;
host['DOMMatrixReadOnly'] ??= InertDOMMatrixReadOnly;

// jsdom lays nothing out, so every element is 0 by 0 and React Flow divides by it. Give boxes the size
// they were asked for in a style, or 1.
if (typeof HTMLElement !== 'undefined') {
  const size = (style: string) => (element: HTMLElement) => parseFloat(element.style.getPropertyValue(style)) || 1;
  const width = size('width');
  const height = size('height');
  Object.defineProperties(HTMLElement.prototype, {
    offsetWidth: {
      configurable: true,
      get(this: HTMLElement) {
        return width(this);
      },
    },
    offsetHeight: {
      configurable: true,
      get(this: HTMLElement) {
        return height(this);
      },
    },
  });
}

if (typeof SVGElement !== 'undefined') {
  const proto = SVGElement.prototype as unknown as { getBBox?: () => { x: number; y: number; width: number; height: number } };
  proto.getBBox ??= () => ({ x: 0, y: 0, width: 0, height: 0 });
}
