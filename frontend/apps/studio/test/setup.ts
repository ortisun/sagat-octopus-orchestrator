import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

// jsdom does not implement the modal part of <dialog>; this is what a browser does with it.
if (typeof HTMLDialogElement !== 'undefined') {
  const proto = HTMLDialogElement.prototype as Partial<Record<'showModal' | 'close', (this: HTMLDialogElement) => void>>;
  proto.showModal ??= function showModal(this: HTMLDialogElement) {
    this.setAttribute('open', '');
  };
  proto.close ??= function close(this: HTMLDialogElement) {
    this.removeAttribute('open');
    this.dispatchEvent(new Event('close'));
  };
}

// The design system announces through two live regions that it appends to <body> once and keeps for
// the life of the page (a polite `status` and an assertive `alert`). They are its own channel, not
// part of any screen: left visible to role queries, every `getByRole('alert')` of every test would
// also match them, with whatever an earlier test announced. They are hidden from role queries here;
// `announcedAssertively()` and `announcedPolitely()` of `test/ui.ts` read them when a test is about them.
if (typeof document !== 'undefined' && typeof MutationObserver !== 'undefined') {
  const hide = () => {
    for (const region of document.querySelectorAll('[data-astryx-live-region]:not([aria-hidden])')) region.setAttribute('aria-hidden', 'true');
  };
  new MutationObserver(hide).observe(document.body, { childList: true });
}

afterEach(() => cleanup());
