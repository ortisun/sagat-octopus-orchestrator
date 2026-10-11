/**
 * What the runs pages' tests need of a browser that jsdom does not give: React Flow's observers,
 * and sizes for the scrolling boxes, the rows and the canvas, which the windowing of long lists and
 * the drawing of a graph read. `installRunsDom()` returns what undoes it.
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

export interface RunsDomOptions {
  /** Height of a scrolling box (`data-window`), in px. */
  viewport?: number;
  /** Height of a row of a long table (`data-index`), in px. */
  row?: number;
  /** Size of the canvas. */
  canvas?: { width: number; height: number };
}

export function installRunsDom(options: RunsDomOptions = {}): () => void {
  const viewport = options.viewport ?? 480;
  const row = options.row ?? 48;
  const canvas = options.canvas ?? { width: 6000, height: 4000 };
  const host = globalThis as unknown as Record<string, unknown>;
  const before = { observer: host['ResizeObserver'], matrix: host['DOMMatrixReadOnly'] };
  host['ResizeObserver'] ??= InertResizeObserver;
  host['DOMMatrixReadOnly'] ??= InertDOMMatrixReadOnly;
  const proto = HTMLElement.prototype;
  const height = Object.getOwnPropertyDescriptor(proto, 'offsetHeight');
  const width = Object.getOwnPropertyDescriptor(proto, 'offsetWidth');
  const rect = Object.getOwnPropertyDescriptor(Element.prototype, 'getBoundingClientRect');
  const svg = SVGElement.prototype as unknown as { getBBox?: () => { x: number; y: number; width: number; height: number } };
  const bbox = svg.getBBox;
  svg.getBBox ??= () => ({ x: 0, y: 0, width: 0, height: 0 });
  Object.defineProperty(proto, 'offsetHeight', {
    configurable: true,
    get(this: HTMLElement) {
      if (this.hasAttribute('data-window')) return viewport;
      if (this.hasAttribute('data-index')) return row;
      return this.classList.contains('react-flow') ? canvas.height : 0;
    },
  });
  Object.defineProperty(proto, 'offsetWidth', {
    configurable: true,
    get(this: HTMLElement) {
      if (this.hasAttribute('data-window')) return 1200;
      return this.classList.contains('react-flow') ? canvas.width : 0;
    },
  });
  Object.defineProperty(Element.prototype, 'getBoundingClientRect', {
    configurable: true,
    writable: true,
    value(this: Element) {
      const h = this.hasAttribute('data-index') ? row : 0;
      return { x: 0, y: 0, top: 0, left: 0, right: 0, bottom: h, width: 0, height: h, toJSON: () => ({}) } as DOMRect;
    },
  });
  return () => {
    if (before.observer === undefined) delete host['ResizeObserver'];
    if (before.matrix === undefined) delete host['DOMMatrixReadOnly'];
    if (height) Object.defineProperty(proto, 'offsetHeight', height);
    if (width) Object.defineProperty(proto, 'offsetWidth', width);
    if (rect) Object.defineProperty(Element.prototype, 'getBoundingClientRect', rect);
    if (bbox === undefined) delete svg.getBBox;
  };
}
